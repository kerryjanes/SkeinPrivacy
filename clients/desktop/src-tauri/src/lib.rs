//! Weft VPN desktop backend. Connect launches the bundled **sing-box** core (the engine
//! V2Box / Happ / Hiddify wrap) as a sidecar, configured to tunnel through a Weft node via
//! **VLESS + Reality**. Two modes — **1-hop** (direct, fast) and **multihop** (routed through
//! the Tor network at the node, maximum privacy). sing-box provides OS capture as a local
//! **proxy** (mixed SOCKS5 + HTTP on 127.0.0.1, no admin). Disconnect kills the core.

use serde::Serialize;
use serde_json::json;
use tauri::{Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

/// Local proxy port (mixed SOCKS5 + HTTP). Point your browser/OS at `127.0.0.1:<PROXY_PORT>`.
const PROXY_PORT: u16 = 2080;

// The Weft node this app connects through (a deployed VLESS + Reality node). In the full
// network these come from the on-chain registry; pinned here for the launch node.
const NODE_HOST: &str = "vpn.weftnetwork.net";
const NODE_UUID: &str = "b5ced6eb-0cba-4001-9679-65f8ba69e74b";
const NODE_PBK: &str = "ag8kOu7UmNIFxKVdjiasZMc2Vj9OtST3PwcFqh1CmWw";
const NODE_SID: &str = "4ce4af1305de920f";
const NODE_SNI: &str = "ya.ru";
const HOP1_PORT: u16 = 443; // direct VLESS+Reality (vision flow)
const HOPN_PORT: u16 = 8443; // VLESS+Reality routed through Tor at the node (no flow)

#[derive(Default)]
struct Inner {
    singbox: Option<CommandChild>,
    mode: Option<String>, // "1hop" | "multihop"
    wallet: Option<String>,
}

#[derive(Default)]
struct AppState {
    inner: Mutex<Inner>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    state: String,
    mode: Option<String>,
    inbound: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Wallet {
    address: Option<String>,
}

fn snapshot(inner: &Inner) -> Status {
    let on = inner.singbox.is_some();
    Status {
        state: if on { "on" } else { "off" }.into(),
        mode: inner.mode.clone(),
        inbound: if on {
            Some(format!("127.0.0.1:{PROXY_PORT}"))
        } else {
            None
        },
    }
}

/// Build the sing-box config: a local mixed (SOCKS5 + HTTP) proxy → VLESS + Reality to the
/// Weft node. `multihop` targets the node's Tor-routed port (plain Reality, no vision flow —
/// vision can't splice through the node's onward SOCKS hop); `1hop` is the direct vision flow.
fn vpn_config(mode: &str) -> serde_json::Value {
    let multihop = mode == "multihop";
    let mut outbound = json!({
        "type": "vless",
        "tag": "weft",
        "server": NODE_HOST,
        "server_port": if multihop { HOPN_PORT } else { HOP1_PORT },
        "uuid": NODE_UUID,
        "tls": {
            "enabled": true,
            "server_name": NODE_SNI,
            "utls": { "enabled": true, "fingerprint": "firefox" },
            "reality": { "enabled": true, "public_key": NODE_PBK, "short_id": NODE_SID }
        }
    });
    if !multihop {
        outbound["flow"] = json!("xtls-rprx-vision");
    }
    json!({
        "log": { "level": "warn" },
        "inbounds": [ {
            "type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": PROXY_PORT
        } ],
        "outbounds": [ outbound, { "type": "direct", "tag": "direct" } ],
        "route": { "final": "weft" }
    })
}

#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mode: Option<String>,
) -> Result<Status, String> {
    let mode = mode.unwrap_or_else(|| "1hop".into());
    if mode != "1hop" && mode != "multihop" {
        return Err(format!("unknown mode '{mode}' (want 1hop|multihop)"));
    }
    let mut inner = state.inner.lock().await;
    if inner.singbox.is_some() {
        return Ok(snapshot(&inner));
    }

    // Write the sing-box config + launch the bundled core as a sidecar.
    let cfg = vpn_config(&mode);
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

    // Pump the core's logs to the UI; clear the session if it exits.
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
                    let st = app_log.state::<AppState>();
                    let mut inner = st.inner.lock().await;
                    inner.singbox = None;
                    inner.mode = None;
                    break;
                }
                _ => {}
            }
        }
    });

    inner.singbox = Some(child);
    inner.mode = Some(mode);
    Ok(snapshot(&inner))
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    let mut inner = state.inner.lock().await;
    if let Some(child) = inner.singbox.take() {
        let _ = child.kill();
    }
    inner.mode = None;
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
    fn one_hop_config_is_direct_vision_reality_on_443() {
        let c = vpn_config("1hop");
        assert_eq!(c["inbounds"][0]["type"], "mixed");
        assert_eq!(c["inbounds"][0]["listen_port"], PROXY_PORT);
        let out = &c["outbounds"][0];
        assert_eq!(out["type"], "vless");
        assert_eq!(out["server_port"], HOP1_PORT);
        assert_eq!(out["flow"], "xtls-rprx-vision");
        assert_eq!(out["tls"]["reality"]["enabled"], true);
        assert_eq!(out["tls"]["server_name"], NODE_SNI);
        assert_eq!(c["route"]["final"], "weft");
    }

    #[test]
    fn multihop_config_targets_tor_port_without_vision_flow() {
        let c = vpn_config("multihop");
        let out = &c["outbounds"][0];
        assert_eq!(out["server_port"], HOPN_PORT);
        assert!(out["flow"].is_null(), "multihop must not set the vision flow");
        assert_eq!(out["tls"]["reality"]["enabled"], true);
    }
}
