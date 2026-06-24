//! A VLESS gateway front-end — the protocol that V2Box, Happ, and any sing-box / Xray
//! client speak. Each accepted VLESS stream becomes a flow that the [`ClientEngine`] tunnels
//! to its destination through a fresh onion circuit, so a stock client (no Weft software
//! installed) connects to a Weft node and its traffic egresses at a Weft exit.
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

use crate::client_engine::ClientEngine;

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
    engine: Arc<ClientEngine>,
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
async fn handle<S>(engine: Arc<ClientEngine>, uuid: Uuid, mut s: S) -> io::Result<()>
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
    let dst = match read_addr(&mut s, atyp[0], port).await? {
        Some(d) => d,
        None => return Err(io::Error::other("unresolvable vless target")),
    };

    // Response header: ver(1)=0, addons_len(1)=0. Sent before the payload so the client can
    // start its own stream; the rest of `s` in both directions is the tunnelled payload.
    s.write_all(&[0u8, 0u8]).await?;

    if udp {
        return tunnel_udp(&engine, dst, s).await;
    }
    let seed: u64 = rand::thread_rng().gen();
    engine.tunnel(dst, seed, s, false).await
}

/// VLESS UDP-over-stream: each datagram is framed `len(2 BE) | payload`. We de-frame the
/// client side into the engine's per-datagram `Data` cells (one datagram per engine read)
/// and re-frame the exit's replies. The destination is fixed by the request header.
async fn tunnel_udp<S>(engine: &ClientEngine, dst: SocketAddr, s: S) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let framed = UdpFramed::new(s);
    let seed: u64 = rand::thread_rng().gen();
    engine.tunnel(dst, seed, framed, true).await
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

/// Adapts a VLESS length-prefixed UDP stream `S` to the raw-datagram `AsyncRead`/`AsyncWrite`
/// the engine's UDP path expects: each `poll_read` yields exactly one datagram (the 2-byte
/// length prefix stripped) and each `poll_write` emits one framed datagram. Both directions
/// keep explicit byte-progress state so a mid-frame `Pending` or partial inner read/write
/// never loses or splices bytes — the engine sends one `Data` cell per write and consumes one
/// datagram per read, so framing lines up one-to-one.
struct UdpFramed<S> {
    inner: S,
    // Read side: assemble length, then body.
    len_buf: [u8; 2],
    len_filled: usize,
    body: Vec<u8>,
    body_filled: usize,
    have_len: bool,
    // Write side: hold the framed datagram until it is fully flushed to `inner`.
    wbuf: Vec<u8>,
    wsent: usize,
    wpayload: usize,
}

impl<S> UdpFramed<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            len_buf: [0u8; 2],
            len_filled: 0,
            body: Vec::new(),
            body_filled: 0,
            have_len: false,
            wbuf: Vec::new(),
            wsent: 0,
            wpayload: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for UdpFramed<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        let me = self.get_mut();
        loop {
            if !me.have_len {
                let mut tmp = tokio::io::ReadBuf::new(&mut me.len_buf[me.len_filled..]);
                match std::pin::Pin::new(&mut me.inner).poll_read(cx, &mut tmp) {
                    Poll::Ready(Ok(())) => {
                        let got = tmp.filled().len();
                        if got == 0 {
                            return Poll::Ready(Ok(())); // EOF
                        }
                        me.len_filled += got;
                        if me.len_filled == 2 {
                            let n = u16::from_be_bytes(me.len_buf) as usize;
                            me.body = vec![0u8; n];
                            me.body_filled = 0;
                            me.have_len = true;
                        }
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                let n = me.body.len();
                if n == 0 {
                    // Zero-length datagram: emit nothing, reset for the next frame.
                    me.have_len = false;
                    me.len_filled = 0;
                    return Poll::Ready(Ok(()));
                }
                let mut tmp = tokio::io::ReadBuf::new(&mut me.body[me.body_filled..]);
                match std::pin::Pin::new(&mut me.inner).poll_read(cx, &mut tmp) {
                    Poll::Ready(Ok(())) => {
                        let got = tmp.filled().len();
                        if got == 0 {
                            return Poll::Ready(Ok(())); // EOF mid-body
                        }
                        me.body_filled += got;
                        if me.body_filled == n {
                            me.have_len = false;
                            me.len_filled = 0;
                            if buf.remaining() >= n {
                                buf.put_slice(&me.body);
                                return Poll::Ready(Ok(()));
                            }
                            // Datagram bigger than one onion cell (e.g. a QUIC packet): it
                            // can't ride in a single `Data` cell, so drop it (UDP is lossy)
                            // and loop to read the next one. Returning 0 bytes here would be
                            // read as EOF and tear the flow down, so we must not do that.
                            continue;
                        }
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for UdpFramed<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        use std::task::Poll;
        let me = self.get_mut();
        // Stage a new framed datagram only when the previous one is fully flushed.
        if me.wsent == me.wbuf.len() {
            me.wbuf.clear();
            me.wbuf
                .extend_from_slice(&(data.len() as u16).to_be_bytes());
            me.wbuf.extend_from_slice(data);
            me.wsent = 0;
            me.wpayload = data.len();
        }
        while me.wsent < me.wbuf.len() {
            match std::pin::Pin::new(&mut me.inner).poll_write(cx, &me.wbuf[me.wsent..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::from(io::ErrorKind::WriteZero)))
                }
                Poll::Ready(Ok(k)) => me.wsent += k,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(me.wpayload))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let me = self.get_mut();
        std::pin::Pin::new(&mut me.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let me = self.get_mut();
        std::pin::Pin::new(&mut me.inner).poll_shutdown(cx)
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
