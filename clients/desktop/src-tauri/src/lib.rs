//! Weft VPN desktop backend. Connect starts a self-contained Weft circuit (relays + a real
//! internet exit) fronted by a local **VLESS gateway**, then launches the bundled **sing-box**
//! core (the engine V2Box/Happ/Hiddify wrap) as a sidecar, pointed at that gateway. sing-box
//! provides OS capture — a local **proxy** (mixed SOCKS+HTTP, no admin) or a system **TUN**
//! (all traffic, admin) — so we reuse a battle-tested tunnel/DNS stack instead of hand-rolling
//! one. Disconnect kills the core and tears the circuit down.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use serde_json::json;
use weft_vpn::client_engine::Counters;
use weft_vpn::exit::EgressPolicy;
use weft_vpn::{localnet, vless};
use tauri::{Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

/// Local proxy port for `mixed` (SOCKS+HTTP) mode.
const PROXY_PORT: u16 = 2080;
const HOPS: usize = 3;

#[derive(Default)]
struct Inner {
    net: Option<localnet::LocalNet>,
    gateway_task: Option<tokio::task::JoinHandle<()>>,
    singbox: Option<CommandChild>,
    counters: Option<Counters>,
    mode: Option<String>,   // "proxy" | "tun"
    inbound: Option<String>, // e.g. "127.0.0.1:2080" or "tun: weft0"
    wallet: Option<String>,
}

#[derive(Default)]
struct AppState {
    inner: Mutex<Inner>,
    base: AtomicUsize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    state: String,
    mode: Option<String>,
    inbound: Option<String>,
    hops: usize,
    exit_mode: String,
    bytes_up: u64,
    bytes_down: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Wallet {
    address: Option<String>,
}

fn snapshot(inner: &Inner) -> Status {
    let (up, down) = inner
        .counters
        .as_ref()
        .map(|c| (c.up.load(Ordering::Relaxed), c.down.load(Ordering::Relaxed)))
        .unwrap_or((0, 0));
    let on = inner.net.is_some() && inner.singbox.is_some();
    Status {
        state: if on { "on" } else { "off" }.into(),
        mode: inner.mode.clone(),
        inbound: inner.inbound.clone(),
        hops: HOPS,
        exit_mode: if on { "open internet".into() } else { "—".into() },
        bytes_up: up,
        bytes_down: down,
    }
}

/// Generate the sing-box config that points the chosen inbound at the local Weft VLESS gateway.
///
/// - `proxy`: a `mixed` (SOCKS5 + HTTP) inbound on 127.0.0.1:PROXY_PORT — no admin.
/// - `tun`: a system `tun` inbound capturing all OS traffic — needs admin. A route rule bypasses
///   this app's own process so the **in-process Weft exit's** egress isn't recaptured (which
///   would loop back through the gateway); loopback is also kept direct.
fn vpn_config(mode: &str, gateway: SocketAddr, uuid: &str) -> serde_json::Value {
    let vless_out = json!({
        "type": "vless",
        "tag": "weft",
        "server": gateway.ip().to_string(),
        "server_port": gateway.port(),
        "uuid": uuid,
        "network": "tcp",
    });
    let outbounds = json!([vless_out, { "type": "direct", "tag": "direct" }]);

    if mode == "tun" {
        json!({
            "log": { "level": "info" },
            // No fixed interface_name: macOS requires `utunN`, Linux allows any — let sing-box
            // auto-assign a valid name per platform.
            "inbounds": [{
                "type": "tun",
                "tag": "tun-in",
                "address": ["172.19.0.1/30"],
                "auto_route": true,
                "strict_route": true,
                "stack": "gvisor"
            }],
            "outbounds": outbounds,
            "route": {
                "rules": [
                    // The in-process exit (this app) must egress directly, not back through the
                    // tunnel — otherwise the self-contained circuit loops on itself.
                    { "process_name": ["weft-desktop", "Weft VPN"], "action": "route", "outbound": "direct" },
                    { "ip_cidr": ["127.0.0.0/8", "::1/128"], "action": "route", "outbound": "direct" }
                ],
                "final": "weft"
            }
        })
    } else {
        json!({
            "log": { "level": "info" },
            "inbounds": [{
                "type": "mixed",
                "tag": "in",
                "listen": "127.0.0.1",
                "listen_port": PROXY_PORT
            }],
            "outbounds": outbounds,
            "route": { "final": "weft" }
        })
    }
}

#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mode: Option<String>,
) -> Result<Status, String> {
    let mode = mode.unwrap_or_else(|| "proxy".into());
    if mode != "proxy" && mode != "tun" {
        return Err(format!("unknown mode '{mode}' (want proxy|tun)"));
    }
    let mut inner = state.inner.lock().await;
    if inner.net.is_some() {
        return Ok(snapshot(&inner));
    }

    // 1. Self-contained Weft circuit (relays + real exit). Fresh base per connect.
    let base = 40_000 + state.base.fetch_add(1, Ordering::Relaxed) * 1000;
    let net = localnet::spawn(HOPS, EgressPolicy::open(), base)
        .await
        .map_err(|e| format!("circuit failed: {e}"))?;
    let counters = net.engine.counters();

    // 2. Local VLESS gateway in front of the circuit (random UUID, plain TCP on loopback).
    let mut uuid = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut uuid);
    let uuid_str = vless::format_uuid(&uuid);
    let (gw_addr, gw_task) = vless::serve(
        net.engine.clone(),
        "127.0.0.1:0".parse().unwrap(),
        uuid,
        vless::Security::None,
    )
    .await
    .map_err(|e| format!("gateway failed: {e}"))?;

    // 3. Write the sing-box config + launch the bundled core as a sidecar.
    let cfg = vpn_config(&mode, gw_addr, &uuid_str);
    let cfg_path = std::env::temp_dir().join("weft-singbox.json");
    std::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg).unwrap())
        .map_err(|e| format!("write config: {e}"))?;

    let sidecar = app
        .shell()
        .sidecar("sing-box")
        .map_err(|e| format!("sidecar: {e}"))?;
    let (mut rx, child) = sidecar
        .args(["run", "-c", &cfg_path.to_string_lossy()])
        .spawn()
        .map_err(|e| format!("spawn sing-box: {e}"))?;

    // Pump the core's logs to the UI; note its exit.
    let app_log = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                CommandEvent::Stdout(b) | CommandEvent::Stderr(b) => {
                    let line = String::from_utf8_lossy(&b).trim_end().to_string();
                    if !line.is_empty() {
                        let _ = app_log.emit("singbox-log", line);
                    }
                }
                CommandEvent::Terminated(payload) => {
                    let _ = app_log.emit("singbox-exit", payload.code);
                    // The core died (e.g. TUN needs admin) — clear the session so the UI
                    // flips back to "off" instead of falsely showing "connected".
                    let st = app_log.state::<AppState>();
                    let mut inner = st.inner.lock().await;
                    inner.singbox = None;
                    if let Some(t) = inner.gateway_task.take() {
                        t.abort();
                    }
                    inner.net = None;
                    inner.counters = None;
                    inner.mode = None;
                    inner.inbound = None;
                    break;
                }
                _ => {}
            }
        }
    });

    inner.net = Some(net);
    inner.gateway_task = Some(gw_task);
    inner.singbox = Some(child);
    inner.counters = Some(counters);
    inner.mode = Some(mode.clone());
    inner.inbound = Some(if mode == "tun" {
        "system tunnel".into()
    } else {
        format!("127.0.0.1:{PROXY_PORT}")
    });
    Ok(snapshot(&inner))
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    let mut inner = state.inner.lock().await;
    if let Some(child) = inner.singbox.take() {
        let _ = child.kill();
    }
    if let Some(t) = inner.gateway_task.take() {
        t.abort();
    }
    inner.net = None; // Drop tears down relays + the client swarm task.
    inner.counters = None;
    inner.mode = None;
    inner.inbound = None;
    Ok(snapshot(&inner))
}

#[tauri::command]
async fn status(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    Ok(snapshot(&*state.inner.lock().await))
}

#[tauri::command]
async fn import_wallet(
    state: tauri::State<'_, AppState>,
    keypair: String,
) -> Result<Wallet, String> {
    // Accept a Solana CLI keypair JSON (a 64-byte array: 32 seed ‖ 32 pubkey).
    let bytes: Vec<u8> =
        serde_json::from_str(&keypair).map_err(|_| "expected a JSON byte array".to_string())?;
    if bytes.len() != 64 {
        return Err("keypair must be 64 bytes".into());
    }
    let address = bs58::encode(&bytes[32..64]).into_string();
    let mut inner = state.inner.lock().await;
    inner.wallet = Some(address.clone());
    Ok(Wallet {
        address: Some(address),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            status,
            import_wallet
        ])
        .run(tauri::generate_context!())
        .expect("error while running Weft VPN");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_config_has_mixed_inbound_and_vless_to_gateway() {
        let gw: SocketAddr = "127.0.0.1:51820".parse().unwrap();
        let c = vpn_config("proxy", gw, "b831e9b8-6e2f-4e7a-8c2d-1f3a5b7c9e10");
        assert_eq!(c["inbounds"][0]["type"], "mixed");
        assert_eq!(c["inbounds"][0]["listen_port"], PROXY_PORT);
        assert_eq!(c["outbounds"][0]["type"], "vless");
        assert_eq!(c["outbounds"][0]["server_port"], 51820);
        assert_eq!(c["outbounds"][0]["uuid"], "b831e9b8-6e2f-4e7a-8c2d-1f3a5b7c9e10");
        assert_eq!(c["route"]["final"], "weft");
    }

    #[test]
    fn tun_config_captures_all_and_bypasses_own_process() {
        let gw: SocketAddr = "127.0.0.1:51820".parse().unwrap();
        let c = vpn_config("tun", gw, "b831e9b8-6e2f-4e7a-8c2d-1f3a5b7c9e10");
        assert_eq!(c["inbounds"][0]["type"], "tun");
        assert_eq!(c["inbounds"][0]["auto_route"], true);
        // The first route rule must bypass our own process so the in-process exit doesn't loop.
        let rule0 = &c["route"]["rules"][0];
        assert_eq!(rule0["outbound"], "direct");
        let procs = rule0["process_name"].as_array().unwrap();
        assert!(procs.iter().any(|p| p == "weft-desktop"));
        assert_eq!(c["route"]["final"], "weft");
    }
}
