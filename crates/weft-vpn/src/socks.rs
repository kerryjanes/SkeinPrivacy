//! A minimal SOCKS5 proxy front-end. Each accepted CONNECT becomes a flow that the
//! [`ClientEngine`] tunnels to its destination through a fresh onion circuit. This is the
//! no-admin VPN mode: point a browser/OS at `socks5://127.0.0.1:<port>` and its traffic
//! egresses at a Weft exit node. (Domain targets are resolved client-side for now;
//! exit-side DNS is a documented follow-up.)

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpListener, TcpStream};

use crate::client_engine::ClientEngine;

/// Bind the SOCKS5 listener and serve in the background. Returns the bound address and the
/// accept-loop task handle (abort it to stop the proxy).
pub async fn serve(
    engine: Arc<ClientEngine>,
    listen: SocketAddr,
) -> io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(listen).await?;
    let local = listener.local_addr()?;
    let task = tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let eng = engine.clone();
            tokio::spawn(async move {
                let _ = handle(eng, sock).await;
            });
        }
    });
    Ok((local, task))
}

async fn handle(engine: Arc<ClientEngine>, mut sock: TcpStream) -> io::Result<()> {
    // Greeting: VER, NMETHODS, METHODS…
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Err(io::Error::other("not socks5"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    // Reply: no authentication required.
    sock.write_all(&[0x05, 0x00]).await?;

    // Request: VER, CMD, RSV, ATYP, ADDR, PORT.
    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    if req[1] != 0x01 {
        // Only CONNECT is supported; reply "command not supported".
        sock.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Ok(());
    }
    let dst = match read_target(&mut sock, req[3]).await {
        Ok(Some(addr)) => addr,
        _ => {
            // Address type not supported / unresolvable.
            sock.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(());
        }
    };

    // Success reply (BND.ADDR/PORT = 0.0.0.0:0 — we don't expose a bind addr).
    sock.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;

    let seed: u64 = rand::thread_rng().gen();
    engine.tunnel(dst, seed, sock).await
}

async fn read_target(sock: &mut TcpStream, atyp: u8) -> io::Result<Option<SocketAddr>> {
    match atyp {
        0x01 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            let port = read_port(sock).await?;
            Ok(Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(a)), port)))
        }
        0x04 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            let port = read_port(sock).await?;
            Ok(Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(a)), port)))
        }
        0x03 => {
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await?;
            let mut host = vec![0u8; len[0] as usize];
            sock.read_exact(&mut host).await?;
            let port = read_port(sock).await?;
            let host = String::from_utf8_lossy(&host).to_string();
            Ok(lookup_host((host, port)).await?.next())
        }
        _ => Ok(None),
    }
}

async fn read_port(sock: &mut TcpStream) -> io::Result<u16> {
    let mut p = [0u8; 2];
    sock.read_exact(&mut p).await?;
    Ok(u16::from_be_bytes(p))
}
