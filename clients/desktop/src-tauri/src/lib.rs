//! Weft VPN desktop backend.
//!
//! Weft is a token-gated VPN: your access is metered and paid for in $WEFT (0.1 WEFT/GB). The
//! flow is wallet → **provision** → connect:
//!   1. Import your Solana wallet (its $WEFT balance is your traffic budget).
//!   2. `provision` asks the node's control plane for YOUR personal VLESS link, gated by that
//!      balance. The control plane meters it and cuts you off once you've used what your $WEFT
//!      pays for — restoring you when you top up.
//!   3. `connect` launches the bundled **sing-box** core through that personal link. Two modes —
//!      **1-hop** (direct, fast) and **multihop** (routed through Tor at the node, max privacy).
//! sing-box exposes a local mixed (SOCKS5 + HTTP) proxy on 127.0.0.1 — no admin needed.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

/// Local proxy port (mixed SOCKS5 + HTTP). Point your browser/OS at `127.0.0.1:<PROXY_PORT>`.
const PROXY_PORT: u16 = 2080;

/// The launch node's control plane (mints personal links + meters them). In the full network the
/// client picks a node from the on-chain registry; pinned here for the launch node.
const CONTROL_PLANE: &str = "https://vpn.weftnetwork.net:8089";

/// Connection parameters parsed from a provisioned `vless://` link.
#[derive(Default, Clone)]
struct NodeLink {
    host: String,
    hop1_port: u16,
    hopn_port: u16,
    uuid: String,
    pbk: String,
    sid: String,
    sni: String,
}

#[derive(Default)]
struct Inner {
    singbox: Option<CommandChild>,
    mode: Option<String>, // "1hop" | "multihop"
    wallet: Option<String>,
    node: Option<NodeLink>, // provisioned personal link
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
    wallet: Option<String>,
    provisioned: bool,
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
        wallet: inner.wallet.clone(),
        provisioned: inner.node.is_some(),
    }
}

/// Parse `vless://UUID@HOST:PORT?pbk=…&sid=…&sni=…#tag` into its pieces.
fn parse_vless(link: &str) -> Option<(String, String, u16, std::collections::HashMap<String, String>)> {
    let rest = link.strip_prefix("vless://")?;
    let (uuid, after) = rest.split_once('@')?;
    let (authority, query) = after.split_once('?')?;
    let (host, port) = authority.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    let query = query.split('#').next().unwrap_or(query);
    let mut params = std::collections::HashMap::new();
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            params.insert(k.to_string(), v.to_string());
        }
    }
    Some((uuid.to_string(), host.to_string(), port, params))
}

/// Build the sing-box config: a local mixed proxy → VLESS + Reality to the node through the user's
/// personal link. `multihop` targets the Tor-routed port (no vision flow); `1hop` is direct vision.
fn vpn_config(node: &NodeLink, mode: &str) -> Value {
    let multihop = mode == "multihop";
    let mut outbound = json!({
        "type": "vless",
        "tag": "weft",
        "server": node.host,
        "server_port": if multihop { node.hopn_port } else { node.hop1_port },
        "uuid": node.uuid,
        "tls": {
            "enabled": true,
            "server_name": node.sni,
            "utls": { "enabled": true, "fingerprint": "firefox" },
            "reality": { "enabled": true, "public_key": node.pbk, "short_id": node.sid }
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

/// Ask the control plane for this wallet's personal link + current quota/usage. Idempotent —
/// also refreshes the balance, so the UI can poll it to show remaining budget.
#[tauri::command]
async fn provision(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let wallet = {
        let inner = state.inner.lock().await;
        inner.wallet.clone().ok_or("import a wallet first")?
    };
    let body = json!({ "wallet": wallet });
    let resp = reqwest::Client::new()
        .post(format!("{CONTROL_PLANE}/provision"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("control plane unreachable: {e}"))?;
    let status: Value = resp.json().await.map_err(|e| format!("bad response: {e}"))?;
    if let Some(err) = status.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }
    let one_hop = status["links"]["oneHop"]
        .as_str()
        .ok_or("no link in response")?;
    let (uuid, host, hop1, params) = parse_vless(one_hop).ok_or("could not parse link")?;
    let hopn = status["links"]["multiHop"]
        .as_str()
        .and_then(parse_vless_port)
        .unwrap_or(hop1);
    let node = NodeLink {
        host,
        hop1_port: hop1,
        hopn_port: hopn,
        uuid,
        pbk: params.get("pbk").cloned().unwrap_or_default(),
        sid: params.get("sid").cloned().unwrap_or_default(),
        sni: params.get("sni").cloned().unwrap_or_default(),
    };
    state.inner.lock().await.node = Some(node);
    Ok(status)
}

fn parse_vless_port(link: &str) -> Option<u16> {
    parse_vless(link).map(|(_, _, p, _)| p)
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
    let node = inner
        .node
        .clone()
        .ok_or("not provisioned — import a wallet and provision first")?;

    let cfg = vpn_config(&node, &mode);
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
            import_wallet,
            provision
        ])
        .run(tauri::generate_context!())
        .expect("error while running Weft VPN");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeLink {
        NodeLink {
            host: "vpn.weftnetwork.net".into(),
            hop1_port: 443,
            hopn_port: 8443,
            uuid: "u-123".into(),
            pbk: "PBK".into(),
            sid: "SID".into(),
            sni: "ya.ru".into(),
        }
    }

    #[test]
    fn parses_a_personal_vless_link() {
        let link = "vless://abc-uuid@vpn.weftnetwork.net:443?flow=xtls-rprx-vision&type=tcp&security=reality&pbk=THEPBK&sid=THESID&sni=ya.ru#Weft-1hop";
        let (uuid, host, port, params) = parse_vless(link).unwrap();
        assert_eq!(uuid, "abc-uuid");
        assert_eq!(host, "vpn.weftnetwork.net");
        assert_eq!(port, 443);
        assert_eq!(params.get("pbk").unwrap(), "THEPBK");
        assert_eq!(params.get("sni").unwrap(), "ya.ru");
    }

    #[test]
    fn one_hop_config_is_direct_vision_reality_with_personal_uuid() {
        let c = vpn_config(&node(), "1hop");
        assert_eq!(c["inbounds"][0]["type"], "mixed");
        let out = &c["outbounds"][0];
        assert_eq!(out["server_port"], 443);
        assert_eq!(out["uuid"], "u-123");
        assert_eq!(out["flow"], "xtls-rprx-vision");
        assert_eq!(out["tls"]["reality"]["public_key"], "PBK");
        assert_eq!(out["tls"]["server_name"], "ya.ru");
    }

    #[test]
    fn multihop_targets_tor_port_without_vision_flow() {
        let c = vpn_config(&node(), "multihop");
        let out = &c["outbounds"][0];
        assert_eq!(out["server_port"], 8443);
        assert!(out["flow"].is_null(), "multihop must not set the vision flow");
    }
}
