//! A VLESS gateway front-end — the protocol that V2Box, Happ, and any sing-box / Xray
//! client speak. Each accepted VLESS stream becomes a flow that the [`TorBackend`] tunnels
//! to its destination through the Tor network, so a stock client (no Weft software
//! installed) connects to a Weft gateway and its traffic egresses at a Tor exit.
//!
//! VLESS itself is a thin, stateless framing with no transport crypto of its own; security
//! comes from the transport. We support raw TCP (`security=none`, for testing or behind a
//! TLS-terminating front like a CDN/reverse proxy) and TLS (`security=tls`, rustls), which
//! is how a real internet-facing gateway is deployed. The same onion engine backs both, and
//! a co-resident commercial VPN no longer matters: the client owns OS capture, not us.
//!
//! Wire format (after the transport): the client sends
//! `ver(1)=0 | uuid(16) | addons_len(1) M | addons(M) | cmd(1) | port(2 BE) | atyp(1) | addr`
//! then the payload stream; the server replies `ver(1)=0 | addons_len(1)=0` then the payload.
//! `atyp`: 1 = IPv4, 2 = domain (1-byte len + host), 3 = IPv6. `cmd`: 1 = TCP, 2 = UDP.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{lookup_host, TcpListener};

use crate::tor_backend::TorBackend;

/// A 16-byte VLESS user id.
pub type Uuid = [u8; 16];

/// Parse a canonical UUID string (`8-4-4-4-12` hex, dashes optional) into 16 bytes.
pub fn parse_uuid(s: &str) -> Option<Uuid> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Format 16 bytes as a canonical `8-4-4-4-12` UUID string.
pub fn format_uuid(u: &Uuid) -> String {
    let h = hex::encode(u);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Build a `vless://` share link (the import format V2Box / Happ / sing-box / Xray accept).
/// `tls` selects `security=tls` (with `sni` + `allowInsecure=1` for self-signed certs) vs
/// `security=none`. The transport is raw TCP (`type=tcp`), matching this gateway.
pub fn share_link(uuid: &Uuid, host_port: &str, tls: bool, sni: &str) -> String {
    let id = format_uuid(uuid);
    if tls {
        format!("vless://{id}@{host_port}?type=tcp&security=tls&sni={sni}&allowInsecure=1#Weft")
    } else {
        format!("vless://{id}@{host_port}?type=tcp&security=none#Weft")
    }
}

/// Transport security for the gateway listener.
pub enum Security {
    /// Raw TCP — VLESS framing in the clear (testing, or behind an external TLS terminator).
    None,
    /// TLS via rustls (a real internet-facing deployment).
    Tls(Arc<rustls::ServerConfig>),
}

/// Bind the VLESS listener and serve in the background. Returns the bound address and the
/// accept-loop task handle (abort it to stop the gateway). Only streams presenting `uuid`
/// are served.
pub async fn serve(
    engine: Arc<TorBackend>,
    listen: SocketAddr,
    uuid: Uuid,
    security: Security,
) -> io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(listen).await?;
    let local = listener.local_addr()?;
    let acceptor = match security {
        Security::Tls(cfg) => Some(tokio_rustls::TlsAcceptor::from(cfg)),
        Security::None => None,
    };
    let task = tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let _ = sock.set_nodelay(true);
            // Detect dead clients fast (mobile connections drop without a clean close, which
            // would otherwise leave the tunnel — and its Tor stream — hanging forever).
            let _ = socket2::SockRef::from(&sock).set_tcp_keepalive(
                &socket2::TcpKeepalive::new()
                    .with_time(std::time::Duration::from_secs(60))
                    .with_interval(std::time::Duration::from_secs(15)),
            );
            let eng = engine.clone();
            let acc = acceptor.clone();
            tokio::spawn(async move {
                match acc {
                    // A failed TLS handshake (probe, bad SNI, …) just drops the connection.
                    Some(a) => {
                        if let Ok(tls) = a.accept(sock).await {
                            let _ = handle(eng, uuid, tls).await;
                        }
                    }
                    None => {
                        let _ = handle(eng, uuid, sock).await;
                    }
                }
            });
        }
    });
    Ok((local, task))
}

/// Read and validate one VLESS request header, then hand the duplex stream to the engine.
async fn handle<S>(engine: Arc<TorBackend>, uuid: Uuid, mut s: S) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // ver(1) + uuid(16)
    let mut head = [0u8; 17];
    s.read_exact(&mut head).await?;
    if head[0] != 0 {
        return Err(io::Error::other("unsupported vless version"));
    }
    if head[1..17] != uuid {
        return Err(io::Error::other("vless auth failed"));
    }
    // addons: len(1) + addons(len). We don't negotiate flow control; skip them.
    let mut m = [0u8; 1];
    s.read_exact(&mut m).await?;
    if m[0] > 0 {
        let mut addons = vec![0u8; m[0] as usize];
        s.read_exact(&mut addons).await?;
    }
    // cmd(1)
    let mut cmd = [0u8; 1];
    s.read_exact(&mut cmd).await?;
    let udp = match cmd[0] {
        1 => false, // TCP
        2 => true,  // UDP
        _ => return Err(io::Error::other("unsupported vless command")),
    };
    // port(2 BE) + atyp(1) + addr
    let mut port = [0u8; 2];
    s.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    let mut atyp = [0u8; 1];
    s.read_exact(&mut atyp).await?;
    let atyp_v = atyp[0];
    let dst = match read_addr(&mut s, atyp_v, port).await? {
        Some(d) => d,
        None => {
            eprintln!("[vless] unresolvable udp={udp} atyp={atyp_v} port={port}");
            return Err(io::Error::other("unresolvable vless target"));
        }
    };
    eprintln!(
        "[vless] {} dst={dst} atyp={atyp_v}",
        if udp { "UDP" } else { "TCP" }
    );

    // Response header: ver(1)=0, addons_len(1)=0. Sent before the payload so the client can
    // start its own stream; the rest of `s` in both directions is the tunnelled payload.
    s.write_all(&[0u8, 0u8]).await?;

    if udp {
        // Tor carries TCP streams only; UDP egress (e.g. QUIC) is unsupported. Stock clients
        // fall back to TCP when the UDP association fails.
        eprintln!("[vless] REJECT udp dst={dst} (Tor is TCP-only)");
        return Err(io::Error::other("UDP egress is not supported over Tor"));
    }
    let seed: u64 = rand::thread_rng().gen();
    let r = engine.tunnel(dst, seed, s, false).await;
    if let Err(e) = &r {
        eprintln!("[vless] tunnel dst={dst} err: {e}");
    }
    r
}

async fn read_addr<S>(s: &mut S, atyp: u8, port: u16) -> io::Result<Option<SocketAddr>>
where
    S: AsyncRead + Unpin,
{
    match atyp {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a).await?;
            Ok(Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(a)), port)))
        }
        3 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a).await?;
            Ok(Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(a)), port)))
        }
        2 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await?;
            let mut host = vec![0u8; len[0] as usize];
            s.read_exact(&mut host).await?;
            let host = String::from_utf8_lossy(&host).to_string();
            Ok(lookup_host((host, port)).await?.next())
        }
        _ => Ok(None),
    }
}

/// Build a rustls server config from a self-signed certificate for `sni` (dev deployments;
/// clients must allow an untrusted cert). For production, load a real cert with
/// [`tls_config_from_pem`].
pub fn self_signed_tls(sni: &str) -> io::Result<Arc<rustls::ServerConfig>> {
    let cert = rcgen::generate_simple_self_signed(vec![sni.to_string()])
        .map_err(|e| io::Error::other(format!("self-signed cert: {e}")))?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| io::Error::other(format!("key: {e}")))?;
    build_tls_config(vec![cert_der], key_der)
}

/// Build a rustls server config from PEM cert-chain + private-key files (production TLS).
pub fn tls_config_from_pem(
    cert_path: &str,
    key_path: &str,
) -> io::Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| io::Error::other(format!("read certs: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io::Error::other(format!("parse certs: {e}")))?;
    if certs.is_empty() {
        return Err(io::Error::other("no certificates in cert file"));
    }
    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| io::Error::other(format!("read key: {e}")))?;
    build_tls_config(certs, key)
}

fn build_tls_config(
    certs: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
) -> io::Result<Arc<rustls::ServerConfig>> {
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| io::Error::other(format!("tls versions: {e}")))?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| io::Error::other(format!("tls config: {e}")))?;
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_round_trip() {
        let s = "b831e9b8-6e2f-4e7a-8c2d-1f3a5b7c9e10";
        let u = parse_uuid(s).unwrap();
        assert_eq!(format_uuid(&u), s);
        assert!(parse_uuid("not-a-uuid").is_none());
        // Dashless form parses to the same bytes.
        assert_eq!(parse_uuid(&s.replace('-', "")).unwrap(), u);
    }
}
