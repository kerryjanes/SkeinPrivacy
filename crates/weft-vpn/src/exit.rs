//! The real internet-egress exit. Implements [`weft_net::exit::Exit`]: it decodes the
//! [`crate::stream`] frames a VPN client sends, maintains one real `TcpStream` per logical
//! stream (keyed by the metered upstream peer + stream id), and pipes bytes to/from the
//! actual destination. This is what turns Weft from an echo into a working VPN exit.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use weft_net::exit::{Exit, ExitFuture};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::stream::{exit_err, ClientFrame, ExitFrame};

/// How long the exit holds a `Poll`/`Data` cell waiting for upstream bytes before returning
/// an (empty) reply — a long-poll. It returns immediately when data is ready, so active
/// flows aren't slowed; idle flows just hold one cell instead of re-polling every few ms,
/// which keeps the relay from being flooded by many idle connections. Exit I/O runs in its
/// own task (see the relay loop), so holding a cell this long never blocks other flows.
const READ_DEADLINE: Duration = Duration::from_millis(1500);
/// Cap on bytes returned to the client per `Poll` cell. The reverse path isn't fixed-size
/// (each hop only adds a 16-byte AEAD tag), so a large window keeps download throughput up:
/// download rate ≈ this / circuit-RTT.
const MAX_READ_PER_TICK: usize = 256 * 1024;
/// Connection establishment timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Egress policy: which destinations this exit is willing to dial.
#[derive(Default, Clone)]
pub struct EgressPolicy {
    /// If `Some`, only these IPs are dialable (the rest are blocked). `None` = allow all.
    allow: Option<Vec<IpAddr>>,
    /// Allow connecting to loopback (off by default — typically only enabled for tests).
    allow_loopback: bool,
    /// Allow private/link-local ranges (off by default).
    allow_private: bool,
}

impl EgressPolicy {
    /// Open egress to the whole public internet (the default for a real exit operator).
    pub fn open() -> Self {
        Self {
            allow: None,
            allow_loopback: false,
            allow_private: false,
        }
    }

    /// Restrict egress to an explicit allowlist (used by tests + scoped live runs).
    pub fn allowlist(ips: Vec<IpAddr>) -> Self {
        let allow_loopback = ips.iter().any(|ip| ip.is_loopback());
        Self {
            allow: Some(ips),
            allow_loopback,
            allow_private: true,
        }
    }

    fn permits(&self, dst: &SocketAddr) -> bool {
        let ip = dst.ip();
        if let Some(list) = &self.allow {
            if !list.contains(&ip) {
                return false;
            }
        }
        if ip.is_loopback() && !self.allow_loopback {
            return false;
        }
        if !self.allow_private && is_private(&ip) {
            return false;
        }
        true
    }
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_unspecified(),
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// A live upstream connection at the exit: a TCP stream or a connected UDP socket.
enum Conn {
    Tcp { sock: TcpStream, eof: bool },
    Udp { sock: tokio::net::UdpSocket },
}

/// How an exit pins its own egress so it bypasses any tunnel the same host is running:
/// the output interface (macOS `IP_BOUND_IF`) and/or the source address (so reply packets
/// route back correctly even when a full-tunnel route would otherwise hijack them).
#[derive(Default, Clone, Copy)]
pub struct EgressBind {
    pub if_index: Option<u32>,
    pub source: Option<IpAddr>,
}

impl EgressBind {
    fn is_set(&self) -> bool {
        self.if_index.is_some() || self.source.is_some()
    }
}

type ConnKey = ([u8; 32], u64);
/// Each live connection is behind its own async mutex, so different streams' I/O proceeds
/// concurrently while one stream's cells stay serialized (in order). The outer map is a
/// plain mutex held only briefly (lookup/insert/remove), never across I/O.
type ConnMap = std::sync::Mutex<HashMap<ConnKey, Arc<tokio::sync::Mutex<Conn>>>>;

use std::sync::Arc;

/// A stateful exit that dials real destinations for VPN clients. Uses interior mutability so
/// the relay can run many `handle` calls concurrently (one per connection).
#[derive(Default)]
pub struct InternetExit {
    policy: EgressPolicy,
    conns: ConnMap,
    max_conns: usize,
    bind: EgressBind,
}

impl InternetExit {
    pub fn new(policy: EgressPolicy) -> Self {
        Self {
            policy,
            conns: ConnMap::default(),
            max_conns: 4096,
            bind: EgressBind::default(),
        }
    }

    /// Pin outbound egress (interface + source address) so it bypasses tunnel routes.
    pub fn with_egress_bind(mut self, bind: EgressBind) -> Self {
        self.bind = bind;
        self
    }

    /// Connect TCP to `dst`, pinning egress per `bind` (interface + source) so it bypasses
    /// tunnel routes and returns route correctly.
    async fn connect(dst: SocketAddr, bind: EgressBind) -> io::Result<TcpStream> {
        #[cfg(target_os = "macos")]
        if bind.is_set() {
            use tokio::net::TcpSocket;
            let sock = if dst.is_ipv4() {
                TcpSocket::new_v4()?
            } else {
                TcpSocket::new_v6()?
            };
            let r = socket2::SockRef::from(&sock);
            if let Some(nz) = bind.if_index.and_then(std::num::NonZeroU32::new) {
                if dst.is_ipv4() {
                    r.bind_device_by_index_v4(Some(nz))?;
                } else {
                    r.bind_device_by_index_v6(Some(nz))?;
                }
            }
            if let Some(src) = bind.source {
                if src.is_ipv4() == dst.is_ipv4() {
                    sock.bind(SocketAddr::new(src, 0))?;
                }
            }
            return sock.connect(dst).await;
        }
        let _ = bind;
        TcpStream::connect(dst).await
    }

    /// Create a UDP socket connected to `dst` (for DNS etc.), pinned per `bind`.
    async fn connect_udp(dst: SocketAddr, bind: EgressBind) -> io::Result<tokio::net::UdpSocket> {
        #[cfg(target_os = "macos")]
        if bind.is_set() {
            use socket2::{Domain, Socket, Type};
            let domain = if dst.is_ipv4() {
                Domain::IPV4
            } else {
                Domain::IPV6
            };
            let s = Socket::new(domain, Type::DGRAM, None)?;
            if let Some(nz) = bind.if_index.and_then(std::num::NonZeroU32::new) {
                if dst.is_ipv4() {
                    s.bind_device_by_index_v4(Some(nz))?;
                } else {
                    s.bind_device_by_index_v6(Some(nz))?;
                }
            }
            let src: SocketAddr = match bind.source {
                Some(ip) if ip.is_ipv4() == dst.is_ipv4() => SocketAddr::new(ip, 0),
                _ if dst.is_ipv4() => "0.0.0.0:0".parse().unwrap(),
                _ => "[::]:0".parse().unwrap(),
            };
            s.bind(&src.into())?;
            s.set_nonblocking(true)?;
            let std_sock: std::net::UdpSocket = s.into();
            let sock = tokio::net::UdpSocket::from_std(std_sock)?;
            sock.connect(dst).await?;
            return Ok(sock);
        }
        let _ = bind;
        let any = if dst.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let sock = tokio::net::UdpSocket::bind(any).await?;
        sock.connect(dst).await?;
        Ok(sock)
    }

    /// Read whatever is currently available (up to a deadline / cap) into an `ExitFrame`.
    async fn drain(conn: &mut Conn) -> ExitFrame {
        match conn {
            Conn::Tcp { sock, eof } => {
                if *eof {
                    return ExitFrame::Eof;
                }
                let mut buf = vec![0u8; MAX_READ_PER_TICK];
                match tokio::time::timeout(READ_DEADLINE, sock.read(&mut buf)).await {
                    Ok(Ok(0)) => {
                        *eof = true;
                        ExitFrame::Eof
                    }
                    Ok(Ok(n)) => {
                        buf.truncate(n);
                        ExitFrame::Data(buf)
                    }
                    Ok(Err(_)) => ExitFrame::Err(exit_err::IO),
                    // Nothing ready within the deadline — return an empty chunk; client polls.
                    Err(_) => ExitFrame::Data(Vec::new()),
                }
            }
            Conn::Udp { sock } => {
                let mut buf = vec![0u8; 65_535];
                match tokio::time::timeout(READ_DEADLINE, sock.recv(&mut buf)).await {
                    Ok(Ok(n)) => {
                        buf.truncate(n);
                        ExitFrame::Data(buf)
                    }
                    Ok(Err(_)) => ExitFrame::Err(exit_err::IO),
                    Err(_) => ExitFrame::Data(Vec::new()),
                }
            }
        }
    }

    fn get_conn(&self, key: &ConnKey) -> Option<Arc<tokio::sync::Mutex<Conn>>> {
        self.conns.lock().unwrap().get(key).cloned()
    }

    fn drop_conn(&self, key: &ConnKey) {
        self.conns.lock().unwrap().remove(key);
    }

    async fn handle_frame(&self, client: [u8; 32], frame: ClientFrame) -> ExitFrame {
        match frame {
            ClientFrame::Open {
                stream_id,
                dst,
                udp,
            } => {
                if !self.policy.permits(&dst) {
                    return ExitFrame::Err(exit_err::POLICY_BLOCKED);
                }
                if self.conns.lock().unwrap().len() >= self.max_conns {
                    return ExitFrame::Err(exit_err::IO);
                }
                let conn = if udp {
                    match Self::connect_udp(dst, self.bind).await {
                        Ok(sock) => Conn::Udp { sock },
                        Err(_) => return ExitFrame::Err(exit_err::CONNECT_FAILED),
                    }
                } else {
                    match tokio::time::timeout(CONNECT_TIMEOUT, Self::connect(dst, self.bind)).await
                    {
                        Ok(Ok(sock)) => {
                            let _ = sock.set_nodelay(true);
                            Conn::Tcp { sock, eof: false }
                        }
                        Ok(Err(e)) => {
                            eprintln!("[weft-vpn] exit: connect {dst} failed: {e}");
                            return ExitFrame::Err(exit_err::CONNECT_FAILED);
                        }
                        Err(_) => {
                            eprintln!("[weft-vpn] exit: connect {dst} timed out");
                            return ExitFrame::Err(exit_err::CONNECT_FAILED);
                        }
                    }
                };
                self.conns
                    .lock()
                    .unwrap()
                    .insert((client, stream_id), Arc::new(tokio::sync::Mutex::new(conn)));
                ExitFrame::Data(Vec::new()) // nothing to read yet
            }
            ClientFrame::Data {
                stream_id, data, ..
            } => {
                let key = (client, stream_id);
                let Some(arc) = self.get_conn(&key) else {
                    return ExitFrame::Err(exit_err::NO_STREAM);
                };
                if !data.is_empty() {
                    let mut conn = arc.lock().await;
                    let w = match &mut *conn {
                        Conn::Tcp { sock, .. } => sock.write_all(&data).await,
                        Conn::Udp { sock } => sock.send(&data).await.map(|_| ()),
                    };
                    if w.is_err() {
                        self.drop_conn(&key);
                        return ExitFrame::Err(exit_err::IO);
                    }
                }
                // Upload only: `Poll` cells drain the downstream direction independently.
                ExitFrame::Data(Vec::new())
            }
            ClientFrame::Poll { stream_id } => {
                let key = (client, stream_id);
                let Some(arc) = self.get_conn(&key) else {
                    return ExitFrame::Err(exit_err::NO_STREAM);
                };
                let out = {
                    let mut conn = arc.lock().await;
                    Self::drain(&mut conn).await
                };
                if matches!(out, ExitFrame::Eof | ExitFrame::Err(_)) {
                    self.drop_conn(&key);
                }
                out
            }
            ClientFrame::Close { stream_id } => {
                self.drop_conn(&(client, stream_id));
                ExitFrame::Eof
            }
        }
    }
}

impl Exit for InternetExit {
    fn handle<'a>(
        &'a self,
        client: [u8; 32],
        _dest: [u8; 32],
        payload: &'a [u8],
    ) -> ExitFuture<'a> {
        Box::pin(async move {
            let reply = match ClientFrame::decode(payload) {
                Some(frame) => self.handle_frame(client, frame).await,
                None => ExitFrame::Err(exit_err::BAD_FRAME),
            };
            reply.encode()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn exit_dials_a_real_origin_and_streams_bytes() {
        // A local "internet" origin that echoes an HTTP-ish response after reading a line.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = s.read(&mut buf).await;
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\nhello").await.unwrap();
        });

        let exit = InternetExit::new(EgressPolicy::allowlist(vec![origin.ip()]));
        let client = [9u8; 32];

        // Open → connect.
        let r = exit
            .handle(
                client,
                [0u8; 32],
                &ClientFrame::Open {
                    stream_id: 1,
                    dst: origin,
                    udp: false,
                }
                .encode(),
            )
            .await;
        assert!(matches!(ExitFrame::decode(&r).unwrap(), ExitFrame::Data(_)));

        // Data → write request, read response.
        let req = ClientFrame::Data {
            stream_id: 1,
            seq: 0,
            data: b"GET /\r\n".to_vec(),
        };
        // The origin may not have flushed within one tick; poll until we see the body.
        let mut got = Vec::new();
        let first = exit.handle(client, [0u8; 32], &req.encode()).await;
        if let ExitFrame::Data(d) = ExitFrame::decode(&first).unwrap() {
            got.extend_from_slice(&d);
        }
        for _ in 0..50 {
            if got.windows(5).any(|w| w == b"hello") {
                break;
            }
            let p = exit
                .handle(
                    client,
                    [0u8; 32],
                    &ClientFrame::Poll { stream_id: 1 }.encode(),
                )
                .await;
            match ExitFrame::decode(&p).unwrap() {
                ExitFrame::Data(d) => got.extend_from_slice(&d),
                ExitFrame::Eof => break,
                ExitFrame::Err(e) => panic!("exit err {e}"),
            }
        }
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("200 OK") && text.contains("hello"),
            "got: {text}"
        );
    }

    #[tokio::test]
    async fn policy_blocks_disallowed_destinations() {
        let exit = InternetExit::new(EgressPolicy::allowlist(vec!["127.0.0.1".parse().unwrap()]));
        let blocked: SocketAddr = "8.8.8.8:53".parse().unwrap();
        let r = exit
            .handle(
                [0u8; 32],
                [0u8; 32],
                &ClientFrame::Open {
                    stream_id: 1,
                    dst: blocked,
                    udp: false,
                }
                .encode(),
            )
            .await;
        assert_eq!(
            ExitFrame::decode(&r).unwrap(),
            ExitFrame::Err(exit_err::POLICY_BLOCKED)
        );
    }
}
