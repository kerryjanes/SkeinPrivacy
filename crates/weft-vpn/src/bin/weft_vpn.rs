//! `weft-vpn` — run the VPN client front-ends.
//!
//! Usage:
//!   weft-vpn socks [listen_addr]      Local SOCKS5 proxy over a self-contained circuit.
//!                                      Default listen 127.0.0.1:1080. Point apps at it.
//!
//! Env: WEFT_HOPS (default 3), WEFT_EXIT_ALLOWLIST=ip1,ip2 (default: open internet).

use std::net::IpAddr;

use weft_vpn::exit::EgressPolicy;
use weft_vpn::{localnet, socks};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let hops: usize = std::env::var("WEFT_HOPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let policy = match std::env::var("WEFT_EXIT_ALLOWLIST") {
        Ok(list) if !list.trim().is_empty() => {
            let ips: Vec<IpAddr> = list
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            EgressPolicy::allowlist(ips)
        }
        _ => EgressPolicy::open(),
    };

    match cmd.as_str() {
        "socks" => {
            let listen = args.next().unwrap_or_else(|| "127.0.0.1:1080".into());
            let listen = listen
                .parse()
                .map_err(|_| std::io::Error::other("bad listen addr"))?;
            eprintln!("[weft-vpn] starting self-contained circuit ({hops} hops)…");
            let net = localnet::spawn(hops, policy, 40_000).await?;
            let (bound, _task) = socks::serve(net.engine.clone(), listen).await?;
            println!("[weft-vpn] SOCKS5 proxy on {bound} — set your app's SOCKS proxy to it");
            println!("[weft-vpn] e.g. curl --proxy socks5h://{bound} https://example.com");
            // Keep `net` alive (its drop tears the circuit down) until killed.
            std::future::pending::<()>().await;
            drop(net);
            Ok(())
        }
        _ => {
            eprintln!("usage: weft-vpn socks [listen_addr]");
            std::process::exit(2);
        }
    }
}
