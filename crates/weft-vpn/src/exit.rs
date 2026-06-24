//! The real internet-egress exit. Implements [`weft_net::circuit_relay::CircuitExit`]: for
//! each circuit it decodes the [`crate::stream`] frames a VPN client sends, dials/maintains a
//! real `TcpStream` (or UDP socket) per logical stream, and runs a reader task per socket that
//! PUSHES bytes back through the circuit's reverse channel the moment they arrive. This is
//! what turns Weft from an echo into a working VPN exit.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use weft_net::circuit_relay::{CircuitExit, ExitChannels};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;

use crate::stream::{exit_err, ClientFrame, ExitFrame};

/// Cap on bytes read from a real connection per chunk before pushing a reverse cell. The
/// reverse path isn't fixed-size (each hop only adds a 16-byte AEAD tag), so a large window
/// keeps download throughput up.
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

/// A stateful exit that dials real destinations for VPN clients. Cheap to clone-share behind
/// an `Arc`; each opened circuit gets its own task + sockets.
#[derive(Default)]
pub struct InternetExit {
    policy: EgressPolicy,
    max_conns: usize,
}

impl InternetExit {
    pub fn new(policy: EgressPolicy) -> Self {
        Self {
            policy,
            max_conns: 4096,
        }
    }

    /// Connect TCP to `dst`.
    async fn connect(dst: SocketAddr) -> io::Result<TcpStream> {
        TcpStream::connect(dst).await
    }

    /// Create a UDP socket connected to `dst` (for DNS etc.).
    async fn connect_udp(dst: SocketAddr) -> io::Result<tokio::net::UdpSocket> {
        let any = if dst.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let sock = tokio::net::UdpSocket::bind(any).await?;
        sock.connect(dst).await?;
        Ok(sock)
    }
}

/// A circuit's open write side at the exit (the read side lives in a spawned reader task that
/// PUSHES bytes back through the circuit's reverse channel).
enum WriteHandle {
    Tcp(OwnedWriteHalf),
    Udp(std::sync::Arc<UdpSocket>),
}

/// Streaming (push) exit over a persistent circuit. Each circuit gets its own task that owns
/// the live sockets and a reader per socket; readers push [`ExitFrame`]-encoded bytes into the
/// reverse channel the moment the real connection produces them — no client `Poll`, no
/// per-cell round-trip. The [`EgressPolicy`] is enforced identically to the legacy path.
impl CircuitExit for InternetExit {
    fn open(&self, _client: [u8; 32]) -> ExitChannels {
        let (fwd_tx, mut fwd_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (rev_tx, rev_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let policy = self.policy.clone();
        let max_conns = self.max_conns;

        tokio::spawn(async move {
            let mut streams: HashMap<u64, WriteHandle> = HashMap::new();
            while let Some(payload) = fwd_rx.recv().await {
                let Some(frame) = ClientFrame::decode(&payload) else {
                    let _ = rev_tx.send(ExitFrame::Err(exit_err::BAD_FRAME).encode());
                    continue;
                };
                match frame {
                    ClientFrame::Open { stream_id, dst, udp } => {
                        if !policy.permits(&dst) {
                            let _ = rev_tx.send(ExitFrame::Err(exit_err::POLICY_BLOCKED).encode());
                            continue;
                        }
                        if streams.len() >= max_conns {
                            let _ = rev_tx.send(ExitFrame::Err(exit_err::IO).encode());
                            continue;
                        }
                        if udp {
                            match Self::connect_udp(dst).await {
                                Ok(sock) => {
                                    let sock = std::sync::Arc::new(sock);
                                    streams.insert(stream_id, WriteHandle::Udp(sock.clone()));
                                    tokio::spawn(udp_reader(sock, rev_tx.clone()));
                                }
                                Err(_) => {
                                    let _ = rev_tx
                                        .send(ExitFrame::Err(exit_err::CONNECT_FAILED).encode());
                                }
                            }
                        } else {
                            match tokio::time::timeout(CONNECT_TIMEOUT, Self::connect(dst)).await {
                                Ok(Ok(sock)) => {
                                    let _ = sock.set_nodelay(true);
                                    let (r, w) = sock.into_split();
                                    streams.insert(stream_id, WriteHandle::Tcp(w));
                                    tokio::spawn(tcp_reader(r, rev_tx.clone()));
                                }
                                Ok(Err(e)) => {
                                    eprintln!("[weft-vpn] exit: connect {dst} failed: {e}");
                                    let _ = rev_tx
                                        .send(ExitFrame::Err(exit_err::CONNECT_FAILED).encode());
                                }
                                Err(_) => {
                                    eprintln!("[weft-vpn] exit: connect {dst} timed out");
                                    let _ = rev_tx
                                        .send(ExitFrame::Err(exit_err::CONNECT_FAILED).encode());
                                }
                            }
                        }
                    }
                    ClientFrame::Data { stream_id, data, .. } => {
                        if data.is_empty() {
                            continue;
                        }
                        let Some(h) = streams.get_mut(&stream_id) else {
                            let _ = rev_tx.send(ExitFrame::Err(exit_err::NO_STREAM).encode());
                            continue;
                        };
                        let w = match h {
                            WriteHandle::Tcp(sock) => sock.write_all(&data).await,
                            WriteHandle::Udp(sock) => sock.send(&data).await.map(|_| ()),
                        };
                        if w.is_err() {
                            streams.remove(&stream_id);
                            let _ = rev_tx.send(ExitFrame::Err(exit_err::IO).encode());
                        }
                    }
                    // No polling in the push model; a stray Poll is a no-op.
                    ClientFrame::Poll { .. } => {}
                    ClientFrame::Close { stream_id } => {
                        streams.remove(&stream_id); // dropping the write half ends the reader
                    }
                }
            }
        });

        ExitChannels {
            forward: fwd_tx,
            reverse: rev_rx,
        }
    }
}

/// Read a TCP connection to EOF, pushing each chunk as an `ExitFrame::Data` and finishing with
/// `Eof`. Stops when the circuit's reverse channel is dropped (circuit torn down).
async fn tcp_reader(mut r: tokio::net::tcp::OwnedReadHalf, rev_tx: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buf = vec![0u8; MAX_READ_PER_TICK];
    loop {
        match r.read(&mut buf).await {
            Ok(0) => {
                let _ = rev_tx.send(ExitFrame::Eof.encode());
                break;
            }
            Ok(n) => {
                if rev_tx.send(ExitFrame::Data(buf[..n].to_vec()).encode()).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = rev_tx.send(ExitFrame::Err(exit_err::IO).encode());
                break;
            }
        }
    }
}

/// Push each received UDP datagram as an `ExitFrame::Data`.
async fn udp_reader(sock: std::sync::Arc<UdpSocket>, rev_tx: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buf = vec![0u8; 65_535];
    loop {
        match sock.recv(&mut buf).await {
            Ok(n) => {
                if rev_tx.send(ExitFrame::Data(buf[..n].to_vec()).encode()).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = rev_tx.send(ExitFrame::Err(exit_err::IO).encode());
                break;
            }
        }
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
        let ExitChannels { forward, mut reverse } = exit.open([9u8; 32]);

        // Open the stream, then send the request — the exit PUSHES the response back.
        forward
            .send(ClientFrame::Open { stream_id: 1, dst: origin, udp: false }.encode())
            .unwrap();
        forward
            .send(ClientFrame::Data { stream_id: 1, seq: 0, data: b"GET /\r\n".to_vec() }.encode())
            .unwrap();

        let mut got = Vec::new();
        while let Some(payload) = tokio::time::timeout(Duration::from_secs(5), reverse.recv())
            .await
            .expect("no pushed reverse data")
        {
            match ExitFrame::decode(&payload).unwrap() {
                ExitFrame::Data(d) => got.extend_from_slice(&d),
                ExitFrame::Eof => break,
                ExitFrame::Err(e) => panic!("exit err {e}"),
            }
            if got.windows(5).any(|w| w == b"hello") {
                break;
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
        let ExitChannels { forward, mut reverse } = exit.open([0u8; 32]);
        let blocked: SocketAddr = "8.8.8.8:53".parse().unwrap();
        forward
            .send(ClientFrame::Open { stream_id: 1, dst: blocked, udp: false }.encode())
            .unwrap();
        let payload = tokio::time::timeout(Duration::from_secs(5), reverse.recv())
            .await
            .expect("no reverse frame")
            .expect("channel closed");
        assert_eq!(
            ExitFrame::decode(&payload).unwrap(),
            ExitFrame::Err(exit_err::POLICY_BLOCKED)
        );
    }
}
