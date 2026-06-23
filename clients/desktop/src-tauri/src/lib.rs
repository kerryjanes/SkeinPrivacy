//! Weft VPN desktop backend. Bridges the React UI to the `weft-vpn` engine: Connect
//! spins up a self-contained Weft circuit (relays + a real internet exit) and a local
//! SOCKS5 proxy that tunnels app traffic through it; Disconnect tears it all down. Status
//! reports live throughput. A minimal wallet import shows the operator address.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use weft_vpn::client_engine::Counters;
use weft_vpn::exit::EgressPolicy;
use weft_vpn::{localnet, socks};
use tokio::sync::Mutex;

#[derive(Default)]
struct Inner {
    net: Option<localnet::LocalNet>,
    socks_task: Option<tokio::task::JoinHandle<()>>,
    socks_addr: Option<SocketAddr>,
    counters: Option<Counters>,
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
    socks_addr: Option<String>,
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

const HOPS: usize = 3;

fn snapshot(inner: &Inner) -> Status {
    let (up, down) = inner
        .counters
        .as_ref()
        .map(|c| (c.up.load(Ordering::Relaxed), c.down.load(Ordering::Relaxed)))
        .unwrap_or((0, 0));
    Status {
        state: if inner.net.is_some() { "on" } else { "off" }.into(),
        socks_addr: inner.socks_addr.map(|a| a.to_string()),
        hops: HOPS,
        exit_mode: if inner.net.is_some() {
            "open internet".into()
        } else {
            "—".into()
        },
        bytes_up: up,
        bytes_down: down,
    }
}

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    let mut inner = state.inner.lock().await;
    if inner.net.is_some() {
        return Ok(snapshot(&inner));
    }
    // Fresh /memory base per connect so reconnects never collide with a prior circuit.
    let base = 40_000 + state.base.fetch_add(1, Ordering::Relaxed) * 1000;
    let net = localnet::spawn(HOPS, EgressPolicy::open(), base, None)
        .await
        .map_err(|e| format!("circuit failed: {e}"))?;
    let counters = net.engine.counters();

    // Prefer the conventional SOCKS port; fall back to an ephemeral one.
    let listen: SocketAddr = "127.0.0.1:1080".parse().unwrap();
    let (addr, task) = match socks::serve(net.engine.clone(), listen).await {
        Ok(v) => v,
        Err(_) => socks::serve(net.engine.clone(), "127.0.0.1:0".parse().unwrap())
            .await
            .map_err(|e| format!("socks failed: {e}"))?,
    };

    inner.net = Some(net);
    inner.socks_task = Some(task);
    inner.socks_addr = Some(addr);
    inner.counters = Some(counters);
    Ok(snapshot(&inner))
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    let mut inner = state.inner.lock().await;
    if let Some(t) = inner.socks_task.take() {
        t.abort();
    }
    inner.net = None; // Drop tears down relays + the client swarm task
    inner.socks_addr = None;
    inner.counters = None;
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
