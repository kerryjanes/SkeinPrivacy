//! `weft-vpn` — run the VPN client front-ends over the **Tor network**.
//!
//! Usage:
//!   weft-vpn socks [listen_addr]              Local SOCKS5 proxy (default 127.0.0.1:1080).
//!   weft-vpn vless [listen_addr] [opts]       VLESS gateway for V2Box / Happ / sing-box /
//!                                              Xray clients (default 0.0.0.0:8443).
//!
//! Both front-ends route every flow through Tor: genuine multi-hop onion routing to a real
//! Tor exit. No fleet of Weft servers — Tor's global relay network carries the traffic.
//!
//! VLESS options:
//!   --uuid <uuid>          User id (default: the WEFT_UUID env, else a random one printed).
//!   --tls                  Terminate TLS with a self-signed cert (clients allow-insecure).
//!   --tls-sni <host>       SNI/CN for the self-signed cert (default "weft.local").
//!   --tls-cert <f> --tls-key <f>   Terminate TLS with a real PEM cert chain + key.
//!   (no TLS flags = raw TCP, security=none — for testing or behind a TLS front.)

use std::sync::Arc;

use weft_vpn::tor_backend::TorBackend;
use weft_vpn::{socks, vless};

/// Bootstrap the Tor client. First run fetches the Tor directory consensus (~10–30 s); works
/// behind NAT (the client never needs to be reachable).
async fn bootstrap_tor() -> std::io::Result<Arc<TorBackend>> {
    eprintln!("[weft-vpn] bootstrapping Tor (first run downloads the directory, ~10–30s)…");
    let backend = TorBackend::bootstrap().await?;
    eprintln!("[weft-vpn] Tor ready — traffic egresses through the Tor network (multi-hop onion)");
    Ok(Arc::new(backend))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();

    match cmd.as_str() {
        "socks" => {
            let listen = args.next().unwrap_or_else(|| "127.0.0.1:1080".into());
            let listen = listen
                .parse()
                .map_err(|_| std::io::Error::other("bad listen addr"))?;
            let engine = bootstrap_tor().await?;
            let (bound, _task) = socks::serve(engine, listen).await?;
            println!("[weft-vpn] SOCKS5 proxy on {bound} — set your app's SOCKS proxy to it");
            println!("[weft-vpn] e.g. curl --proxy socks5h://{bound} https://example.com");
            std::future::pending::<()>().await;
            Ok(())
        }
        "vless" => run_vless(args).await,
        _ => {
            eprintln!("usage: weft-vpn socks [listen_addr]  |  weft-vpn vless [listen_addr] [--uuid U] [--tls|--tls-cert f --tls-key f]");
            std::process::exit(2);
        }
    }
}

async fn run_vless(mut args: impl Iterator<Item = String>) -> std::io::Result<()> {
    // First positional (if it doesn't start with `--`) is the listen address.
    let mut listen = "0.0.0.0:8443".to_string();
    let mut uuid_str = std::env::var("WEFT_UUID").ok();
    let mut tls = false;
    let mut tls_sni = "weft.local".to_string();
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;
    let mut first = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--uuid" => uuid_str = args.next(),
            "--tls" => tls = true,
            "--tls-sni" => {
                if let Some(v) = args.next() {
                    tls_sni = v;
                }
            }
            "--tls-cert" => tls_cert = args.next(),
            "--tls-key" => tls_key = args.next(),
            other if first && !other.starts_with("--") => listen = other.to_string(),
            other => eprintln!("[weft-vpn] ignoring unknown arg: {other}"),
        }
        first = false;
    }
    let listen = listen
        .parse()
        .map_err(|_| std::io::Error::other("bad listen addr"))?;

    // Resolve the UUID: explicit, env, or a freshly generated one (printed so it can be shared).
    let uuid = match uuid_str {
        Some(s) => vless::parse_uuid(&s)
            .ok_or_else(|| std::io::Error::other("bad --uuid (want 8-4-4-4-12 hex)"))?,
        None => {
            let mut u = [0u8; 16];
            rand::Rng::fill(&mut rand::thread_rng(), &mut u);
            eprintln!("[weft-vpn] generated UUID: {}", vless::format_uuid(&u));
            u
        }
    };

    let security = if let (Some(cert), Some(key)) = (&tls_cert, &tls_key) {
        vless::Security::Tls(vless::tls_config_from_pem(cert, key)?)
    } else if tls {
        eprintln!(
            "[weft-vpn] TLS: self-signed cert for SNI '{tls_sni}' (clients must allow-insecure)"
        );
        vless::Security::Tls(vless::self_signed_tls(&tls_sni)?)
    } else {
        vless::Security::None
    };

    let engine = bootstrap_tor().await?;
    let (bound, _task) = vless::serve(engine, listen, uuid, security).await?;
    let sec = match (&tls_cert, tls) {
        (Some(_), _) => "tls",
        (None, true) => "tls (self-signed)",
        _ => "none",
    };
    println!("[weft-vpn] VLESS gateway on {bound} (security={sec}) → Tor");
    println!(
        "[weft-vpn] share link: {}",
        vless::share_link(&uuid, &bound.to_string(), tls || tls_cert.is_some(), &tls_sni)
    );
    println!("[weft-vpn] add this server in V2Box / Happ / any sing-box or Xray client");
    std::future::pending::<()>().await;
    Ok(())
}
