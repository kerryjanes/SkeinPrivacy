//! System-wide TUN front-end (macOS `utun`). Opens a virtual interface, runs an `ipstack`
//! userspace netstack that terminates TCP/UDP flows from the captured IP packets, and
//! tunnels each flow `(dst, stream)` through the Weft circuit via the shared [`ClientEngine`]
//! — exactly like the SOCKS front-end, but transparently for *all* routed traffic.
//!
//! Two modes (requires root for the utun device + routing):
//! - **scoped**: route only listed destinations through Weft.
//! - **full tunnel** (`["default"]`): capture ALL traffic via the `0.0.0.0/1`+`128.0.0.0/1`
//!   split (overrides the default route without touching it), pin the exit's own egress to
//!   the physical link via an interface-scoped route (bypassing every utun, including a
//!   co-resident VPN), disable un-tunnelled IPv6, and run a local DNS forwarder on
//!   `127.0.0.1:53`. The `TunHandle`'s `Drop` is a kill-switch: it removes every route and
//!   restores DNS/IPv6, never modifying the system default route.
use std::io;
use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ipstack::{IpStack, IpStackConfig, IpStackStream};
use rand::Rng;
use tun::AbstractDevice;

use crate::client_engine::ClientEngine;

/// The Weft tunnel interface's own address + the MTU we advertise. The netstack reassembles
/// TCP and the client engine re-chunks the byte stream to ≤ one onion cell payload, so the
/// link MTU is independent of the cell size; `ipstack` requires ≥ 1280 (the IPv6 minimum).
const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(10, 73, 0, 2);
const TUN_PEER: Ipv4Addr = Ipv4Addr::new(10, 73, 0, 1);
const TUN_NETMASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
const TUN_MTU: u16 = 1400;

/// The kernel index of `iface`, or `None`.
pub fn interface_index(iface: &str) -> Option<u32> {
    let cname = std::ffi::CString::new(iface).ok()?;
    // SAFETY: `if_nametoindex` reads a NUL-terminated C string and returns 0 on error.
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    (idx != 0).then_some(idx)
}

/// The interface index of the system default route.
pub fn default_interface_index() -> Option<u32> {
    interface_index(&default_interface_name()?)
}

/// The physical egress for the exit's own traffic under a full tunnel: the first `en*`
/// interface that has an IPv4 address AND a DHCP router (the LAN gateway). The exit binds
/// to this and uses an interface-scoped route via the gateway, so it reaches the internet
/// directly — bypassing every `utun` (including a co-resident VPN that owns the default).
/// Returns `(iface, iface_addr, lan_gateway)`.
pub fn physical_egress() -> Option<(String, std::net::IpAddr, String)> {
    let list = Command::new("ifconfig").arg("-l").output().ok()?;
    let names = String::from_utf8_lossy(&list.stdout);
    for name in names.split_whitespace().filter(|n| n.starts_with("en")) {
        let getstr = |opt: &[&str]| {
            Command::new("ipconfig")
                .args(opt)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let addr = getstr(&["getifaddr", name]);
        let gw = getstr(&["getoption", name, "router"]);
        if let (Some(a), Some(g)) = (addr, gw) {
            if let Ok(ip) = a.parse() {
                return Some((name.to_string(), ip, g));
            }
        }
    }
    None
}

/// The primary IPv4 address of `iface` (parsed from `ifconfig`). The exit binds its egress
/// source to this so reply packets route back via the same interface under a full tunnel.
pub fn interface_addr(iface: &str) -> Option<std::net::IpAddr> {
    let out = Command::new("ifconfig").arg(iface).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("inet ")
            .and_then(|r| r.split_whitespace().next())
            .and_then(|a| a.parse().ok())
    })
}

/// The interface name of the system default route.
pub fn default_interface_name() -> Option<String> {
    let out = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("interface:"))
        .map(|s| s.trim().to_string())
}

/// Run a local DNS forwarder on `127.0.0.1:53`. macOS's system resolver (mDNSResponder)
/// won't reliably send queries through our tunnel when a co-resident VPN owns the default
/// route, but it talks to 127.0.0.1 cleanly — and from here the upstream UDP query is
/// OS-routed through the tunnel to `upstream` (e.g. 1.1.1.1) and answered. Returns the task.
async fn spawn_dns_proxy(
    upstream: std::net::SocketAddr,
    bind_if: Option<u32>,
) -> io::Result<tokio::task::JoinHandle<()>> {
    use std::sync::Arc;
    use tokio::net::UdpSocket;
    let sock = Arc::new(UdpSocket::bind("127.0.0.1:53").await?);
    Ok(tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            let (n, from) = match sock.recv_from(&mut buf).await {
                Ok(x) => x,
                Err(_) => break,
            };
            let query = buf[..n].to_vec();
            let down = sock.clone();
            tokio::spawn(async move {
                // Resolve via the physical interface so lookups are fast + reliable (the
                // onion-tunnelled UDP path adds latency that the OS resolver times out on).
                let Ok(up) = upstream_socket(bind_if).await else {
                    return;
                };
                if up.connect(upstream).await.is_err() || up.send(&query).await.is_err() {
                    return;
                }
                let mut rbuf = [0u8; 1500];
                if let Ok(Ok(rn)) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), up.recv(&mut rbuf))
                        .await
                {
                    let _ = down.send_to(&rbuf[..rn], from).await;
                }
            });
        }
    }))
}

/// A UDP socket optionally pinned to `bind_if` (macOS `IP_BOUND_IF`) so it bypasses the tunnel.
async fn upstream_socket(bind_if: Option<u32>) -> io::Result<tokio::net::UdpSocket> {
    #[cfg(target_os = "macos")]
    if let Some(nz) = bind_if.and_then(std::num::NonZeroU32::new) {
        use socket2::{Domain, Socket, Type};
        let s = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
        s.bind_device_by_index_v4(Some(nz))?;
        s.bind(&"0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap().into())?;
        s.set_nonblocking(true)?;
        return tokio::net::UdpSocket::from_std(s.into());
    }
    let _ = bind_if;
    tokio::net::UdpSocket::bind("0.0.0.0:0").await
}

/// The macOS network-service name (e.g. "Wi-Fi") that owns `iface`, for DNS configuration.
pub fn network_service_for(iface: &str) -> Option<String> {
    let out = Command::new("networksetup")
        .arg("-listnetworkserviceorder")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut current: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        // "(1) Wi-Fi" — a service header.
        if let Some(rest) = l.strip_prefix('(') {
            if let Some(idx) = rest.find(") ") {
                if !rest.starts_with("Hardware") {
                    current = Some(rest[idx + 2..].trim().to_string());
                }
            }
        }
        // "(Hardware Port: Wi-Fi, Device: en0)" — binds the last service to a device.
        if l.contains(&format!("Device: {iface})")) {
            return current.clone();
        }
    }
    None
}

/// Read the DNS servers configured for a network service (empty if "no servers").
pub fn get_dns(service: &str) -> Vec<String> {
    Command::new("networksetup")
        .args(["-getdnsservers", service])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.trim().parse::<std::net::IpAddr>().is_ok())
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Set a network service's DNS servers (pass `["Empty"]` to clear).
pub fn set_dns(service: &str, servers: &[&str]) {
    let _ = Command::new("networksetup")
        .args(["-setdnsservers", service])
        .args(servers)
        .status();
}

/// The system default route's gateway IP (e.g. the LAN router), used to install bypass
/// routes so the VPN's own data-plane traffic doesn't loop through the tunnel.
pub fn default_gateway() -> Option<String> {
    let out = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("gateway:"))
        .map(|s| s.trim().to_string())
}

/// A handle to a running TUN tunnel. Dropping it removes every route we added (kill-switch),
/// restores the original DNS, and returns to normal routing; the system default route is
/// never modified.
pub struct TunHandle {
    iface: String,
    // Each entry is the `route` argv (after the program name) that undoes one route we added.
    removes: Vec<Vec<String>>,
    // (network service, the DNS servers to restore) — set for full-tunnel.
    dns_restore: Option<(String, Vec<String>)>,
    // The local DNS forwarder task (full-tunnel only).
    dns_proxy: Option<tokio::task::JoinHandle<()>>,
    // Network service whose IPv6 we disabled (re-enabled on teardown). IPv6 isn't tunnelled,
    // so we turn it off to stop apps' happy-eyeballs from hanging on un-routed IPv6.
    v6_restore: Option<String>,
}

impl Drop for TunHandle {
    fn drop(&mut self) {
        if let Some(h) = self.dns_proxy.take() {
            h.abort();
        }
        if let Some(svc) = &self.v6_restore {
            let _ = Command::new("networksetup")
                .args(["-setv6automatic", svc])
                .status();
        }
        if let Some((svc, saved)) = &self.dns_restore {
            if saved.is_empty() {
                set_dns(svc, &["Empty"]);
            } else {
                let refs: Vec<&str> = saved.iter().map(String::as_str).collect();
                set_dns(svc, &refs);
            }
        }
        for args in &self.removes {
            let _ = Command::new("route").args(args).status();
        }
        eprintln!(
            "[weft-vpn] tun {} routes + DNS restored (kill-switch); default route untouched",
            self.iface
        );
    }
}

/// Bring up a utun device and start tunnelling.
///
/// - `scoped == ["default"]` → **full-tunnel**: capture ALL traffic via the utun using the
///   `0.0.0.0/1` + `128.0.0.0/1` split (which overrides the default route without touching
///   it, so teardown is clean), and add bypass host-routes for the `bypass` endpoints (the
///   relay IPs the client dials) via the real gateway so the data plane doesn't loop.
/// - otherwise → **scoped**: route only the given destinations through Weft.
pub async fn up(
    engine: Arc<ClientEngine>,
    scoped: Vec<String>,
    bypass: Vec<std::net::IpAddr>,
    egress: Option<(String, String)>, // (physical iface, LAN gateway) for the exit's egress
) -> io::Result<TunHandle> {
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

    let full = scoped.iter().any(|s| s == "default");
    let mut removes: Vec<Vec<String>> = Vec::new();
    let mut dns_restore: Option<(String, Vec<String>)> = None;
    let mut dns_proxy: Option<tokio::task::JoinHandle<()>> = None;
    let mut v6_restore: Option<String> = None;
    let add = |args: &[&str]| -> bool {
        Command::new("route")
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    if full {
        // (1) The exit's own egress: an interface-scoped route via the LAN gateway on the
        // physical link, so the exit (bound to that interface) reaches the internet directly,
        // bypassing both our utun AND any co-resident VPN that owns the default route.
        if let Some((pif, gw)) = &egress {
            for net in ["0.0.0.0/1", "128.0.0.0/1"] {
                if add(&["-n", "add", "-net", net, gw, "-ifscope", pif]) {
                    removes.push(vec![
                        "-n".into(),
                        "delete".into(),
                        "-net".into(),
                        net.into(),
                        "-ifscope".into(),
                        pif.clone(),
                    ]);
                }
            }
            eprintln!("[weft-vpn] exit egress via {pif} → {gw} (ifscoped, bypasses all tunnels)");
            // macOS's system resolver won't reliably route its own queries through the tunnel
            // here, so run a local forwarder on 127.0.0.1:53 (which it talks to cleanly) and
            // point the OS at it. The forwarder relays queries through the tunnel to 1.1.1.1.
            match spawn_dns_proxy("1.1.1.1:53".parse().unwrap(), interface_index(pif)).await {
                Ok(h) => {
                    dns_proxy = Some(h);
                    if let Some(svc) = network_service_for(pif) {
                        let saved = get_dns(&svc);
                        set_dns(&svc, &["127.0.0.1"]);
                        eprintln!("[weft-vpn] DNS → local forwarder (127.0.0.1 → tunnel → 1.1.1.1) on \"{svc}\"");
                        // IPv6 isn't tunnelled — disable it so apps don't hang on un-routed v6.
                        if Command::new("networksetup")
                            .args(["-setv6off", &svc])
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                        {
                            eprintln!("[weft-vpn] IPv6 disabled on \"{svc}\" (restored on exit)");
                            v6_restore = Some(svc.clone());
                        }
                        dns_restore = Some((svc, saved));
                    }
                }
                Err(e) => eprintln!(
                    "[weft-vpn] WARN: DNS forwarder failed ({e}); system DNS may not resolve"
                ),
            }
        }
        // (2) Bypass the data-plane endpoints (remote relays) via the default gateway.
        if let Some(gw) = default_gateway() {
            for ip in &bypass {
                if !ip.is_loopback() {
                    let ips = ip.to_string();
                    if add(&["-n", "add", "-host", &ips, &gw]) {
                        removes.push(vec!["-n".into(), "delete".into(), "-host".into(), ips]);
                    }
                }
            }
        }
        // (3) Capture everything else: 0/1 + 128/1 beat the default route by being more
        // specific, overriding it without touching it (clean teardown).
        for net in ["0.0.0.0/1", "128.0.0.0/1"] {
            if add(&["-n", "add", "-net", net, "-interface", &iface]) {
                removes.push(vec![
                    "-n".into(),
                    "delete".into(),
                    "-net".into(),
                    net.into(),
                ]);
            }
        }
        eprintln!("[weft-vpn] FULL TUNNEL: all traffic now routed through Weft ({iface})");
    } else {
        for dst in &scoped {
            if add(&["-n", "add", dst, "-interface", &iface]) {
                eprintln!("[weft-vpn] routing {dst} → {iface}");
                removes.push(vec!["-n".into(), "delete".into(), dst.clone()]);
            } else {
                eprintln!("[weft-vpn] WARN: failed to route {dst}");
            }
        }
    }

    let mut ipcfg = IpStackConfig::default();
    ipcfg
        .mtu(TUN_MTU)
        .map_err(|e| io::Error::other(e.to_string()))?;
    // The `tun` crate already strips/adds the macOS utun 4-byte address-family prefix
    // (PI=true by default), so ipstack sees bare IP packets — packet_information must be off.
    ipcfg.packet_information(false);
    let mut stack = IpStack::new(ipcfg, device);

    let counter = Arc::new(AtomicU64::new(0));
    tokio::spawn(async move {
        loop {
            match stack.accept().await {
                Ok(IpStackStream::Tcp(tcp)) => {
                    // ipstack: peer_addr() is the original destination, local_addr() the source.
                    let dst = tcp.peer_addr();
                    let eng = engine.clone();
                    let seed =
                        rand::thread_rng().gen::<u64>() ^ counter.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        if let Err(e) = eng.tunnel(dst, seed, tcp, false).await {
                            eprintln!("[weft-vpn] tun: TCP → {dst} error: {e}");
                        }
                    });
                }
                Ok(IpStackStream::Udp(udp)) => {
                    // UDP (DNS, QUIC) — tunnel as a UDP flow to the exit.
                    let dst = udp.peer_addr();
                    let eng = engine.clone();
                    let seed =
                        rand::thread_rng().gen::<u64>() ^ counter.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        if let Err(e) = eng.tunnel(dst, seed, udp, true).await {
                            eprintln!("[weft-vpn] tun: UDP → {dst} error: {e}");
                        }
                    });
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[weft-vpn] tun: netstack accept error: {e}");
                    break;
                }
            }
        }
    });

    Ok(TunHandle {
        iface,
        removes,
        dns_restore,
        dns_proxy,
        v6_restore,
    })
}
