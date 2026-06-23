//! System-wide TUN front-end (macOS `utun`). Opens a virtual interface, runs an `ipstack`
//! userspace netstack that terminates TCP flows from the captured IP packets, and tunnels
//! each flow `(dst, stream)` through the Weft circuit via the shared [`ClientEngine`] —
//! exactly like the SOCKS front-end, but transparently for *all* routed traffic.
//!
//! Requires root (to create the utun device and add routes). UDP egress (e.g. DNS) is not
//! yet handled by the exit, so route TCP destinations through the tunnel for now.
use std::io;
use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ipstack::{IpStack, IpStackConfig, IpStackStream};
use rand::Rng;
use tun::AbstractDevice;

use crate::client_engine::ClientEngine;

/// The Weft tunnel interface's own address + the MTU we advertise. MTU is kept low so a
/// single TCP segment fits one onion cell payload (no IP fragmentation).
const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(10, 73, 0, 2);
const TUN_PEER: Ipv4Addr = Ipv4Addr::new(10, 73, 0, 1);
const TUN_NETMASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
const TUN_MTU: u16 = 1000;

/// A handle to a running TUN tunnel. Dropping it removes the scoped routes (kill-switch).
pub struct TunHandle {
    iface: String,
    scoped_routes: Vec<String>,
}

impl Drop for TunHandle {
    fn drop(&mut self) {
        for dst in &self.scoped_routes {
            let _ = Command::new("route").args(["-n", "delete", dst]).status();
        }
        eprintln!(
            "[weft-vpn] tun {} routes removed (kill-switch)",
            self.iface
        );
    }
}

/// Bring up a utun device and start tunnelling. `scoped` lists destination prefixes/IPs to
/// route through Weft (e.g. ["93.184.216.34/32"]); pass `["default"]` for a full-tunnel
/// VPN (NOT done automatically — that reroutes ALL traffic and needs care). Returns a
/// handle that tears the routes down on drop.
pub async fn up(engine: Arc<ClientEngine>, scoped: Vec<String>) -> io::Result<TunHandle> {
    let mut config = tun::Configuration::default();
    config
        .address(TUN_ADDR)
        .destination(TUN_PEER)
        .netmask(TUN_NETMASK)
        .mtu(TUN_MTU)
        .up();

    let device = tun::create_as_async(&config).map_err(|e| io::Error::other(e.to_string()))?;
    let iface = device
        .tun_name()
        .map_err(|e| io::Error::other(e.to_string()))?;
    eprintln!("[weft-vpn] utun up: {iface} ({TUN_ADDR}/24, mtu {TUN_MTU})");

    // Route the requested destinations through the tunnel interface.
    let mut scoped_routes = Vec::new();
    for dst in &scoped {
        let status = Command::new("route")
            .args(["-n", "add", dst, "-interface", &iface])
            .status()
            .map_err(|e| io::Error::other(format!("route add {dst}: {e}")))?;
        if status.success() {
            eprintln!("[weft-vpn] routing {dst} → {iface}");
            scoped_routes.push(dst.clone());
        } else {
            eprintln!("[weft-vpn] WARN: failed to route {dst}");
        }
    }

    let mut ipcfg = IpStackConfig::default();
    ipcfg
        .mtu(TUN_MTU)
        .map_err(|e| io::Error::other(e.to_string()))?;
    // macOS utun frames carry a 4-byte address-family prefix.
    ipcfg.packet_information(true);
    let mut stack = IpStack::new(ipcfg, device);

    let counter = Arc::new(AtomicU64::new(0));
    tokio::spawn(async move {
        loop {
            match stack.accept().await {
                Ok(IpStackStream::Tcp(tcp)) => {
                    let dst = tcp.local_addr(); // the destination the app dialed
                    let eng = engine.clone();
                    let seed =
                        rand::thread_rng().gen::<u64>() ^ counter.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        let _ = eng.tunnel(dst, seed, tcp).await;
                    });
                }
                Ok(IpStackStream::Udp(udp)) => {
                    // UDP egress (DNS, QUIC) is not yet supported at the exit — drop for now.
                    let _ = udp;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    Ok(TunHandle {
        iface,
        scoped_routes,
    })
}
