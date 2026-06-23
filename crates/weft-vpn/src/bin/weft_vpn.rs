//! `weft-vpn` — run the VPN client front-ends.
//!
//! Usage:
//!   weft-vpn socks [listen_addr]      Local SOCKS5 proxy (default 127.0.0.1:1080).
//!   sudo weft-vpn tun <dst|default>   System-wide TUN (macOS). `default` = full tunnel.
//!
//! Env:
//!   WEFT_HOPS=3                       Circuit hop count (2..=5).
//!   WEFT_EXIT_ALLOWLIST=ip1,ip2       Restrict the (self-contained) exit's egress.
//!   WEFT_PEERS=<dir>                  Connect to a REAL external network: load node
//!                                      manifests from <dir> instead of an in-process circuit.

use std::net::IpAddr;
use std::sync::Arc;

use weft_net::keys::WeftKeypair;
use weft_vpn::client_engine::ClientEngine;
use weft_vpn::exit::{EgressBind, EgressPolicy};
use weft_vpn::{localnet, manifest, socks};

/// The circuit backing a session: either a self-contained in-process network (real TCP,
/// kept alive by its guard) or a connection to external relay/exit nodes.
enum Net {
    Local(localnet::LocalNet),
    External(Arc<ClientEngine>),
}

impl Net {
    fn engine(&self) -> Arc<ClientEngine> {
        match self {
            Net::Local(n) => n.engine.clone(),
            Net::External(e) => e.clone(),
        }
    }
}

async fn build_net(hops: usize, policy: EgressPolicy, bind: EgressBind) -> std::io::Result<Net> {
    if let Ok(dir) = std::env::var("WEFT_PEERS") {
        let peers = manifest::load_dir(std::path::Path::new(&dir))?;
        eprintln!(
            "[weft-vpn] connecting to {} external node(s) from {dir}…",
            peers.len()
        );
        let kp = WeftKeypair::generate(&mut rand::thread_rng());
        let engine = ClientEngine::connect(&kp, &peers, hops, 0).await?;
        Ok(Net::External(Arc::new(engine)))
    } else {
        eprintln!("[weft-vpn] starting self-contained circuit ({hops} hops)…");
        Ok(Net::Local(
            localnet::spawn(hops, policy, 50_000, bind).await?,
        ))
    }
}

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
            let net = build_net(hops, policy, EgressBind::default()).await?;
            let (bound, _task) = socks::serve(net.engine(), listen).await?;
            println!("[weft-vpn] SOCKS5 proxy on {bound} — set your app's SOCKS proxy to it");
            println!("[weft-vpn] e.g. curl --proxy socks5h://{bound} https://example.com");
            std::future::pending::<()>().await;
            drop(net);
            Ok(())
        }
        #[cfg(target_os = "macos")]
        "tun" => {
            // Route destinations (IPs/CIDRs) or `default` (full tunnel) through Weft. Root.
            use weft_vpn::tun;
            let scoped: Vec<String> = args.collect();
            if scoped.is_empty() {
                eprintln!("usage: sudo weft-vpn tun <dst-ip-or-cidr|default> [more…]");
                std::process::exit(2);
            }
            // The self-contained exit egresses via the physical link (en0) + the LAN gateway,
            // bypassing every utun — including a co-resident VPN that owns the default route.
            // Bind the exit to that interface + source address so replies route back.
            let phys = tun::physical_egress();
            let bind = match &phys {
                Some((pif, addr, _gw)) => EgressBind {
                    if_index: tun::interface_index(pif),
                    source: Some(*addr),
                },
                None => EgressBind::default(),
            };
            eprintln!(
                "[weft-vpn] exit egress pinned: if#{:?} src={:?}",
                bind.if_index, bind.source
            );
            // Full-tunnel: bypass the data-plane endpoints (remote relay IPs). Self-contained
            // relays are on loopback (no bypass).
            let bypass: Vec<IpAddr> = match std::env::var("WEFT_PEERS") {
                Ok(dir) => manifest::load_dir(std::path::Path::new(&dir))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|m| m.multiaddr.split('/').nth(2)?.parse().ok())
                    .collect(),
                _ => Vec::new(),
            };
            let egress = phys.map(|(pif, _addr, gw)| (pif, gw));
            let net = build_net(hops, policy, bind).await?;
            let _tun = tun::up(net.engine(), scoped, bypass, egress).await?;
            println!("[weft-vpn] system tunnel up — routed traffic now exits via Weft");
            println!("[weft-vpn] Ctrl-C to stop (routes are removed automatically)");
            tokio::signal::ctrl_c().await.ok();
            println!("[weft-vpn] shutting down…");
            drop(_tun);
            drop(net);
            Ok(())
        }
        _ => {
            eprintln!("usage: weft-vpn socks [listen_addr]  |  sudo weft-vpn tun <dst|default>");
            std::process::exit(2);
        }
    }
}
