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

/// How long the exit waits for upstream bytes before returning an (empty) reply, so the
/// relay loop is never blocked for long on one cell. The client `Poll`s to drain more.
const READ_DEADLINE: Duration = Duration::from_millis(25);
/// Cap on bytes returned to the client per cell (the reverse path isn't fixed-size, but we
/// bound it to keep replies sane).
const MAX_READ_PER_TICK: usize = 32 * 1024;
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

struct Conn {
    sock: TcpStream,
    eof: bool,
}

/// A stateful exit that dials real destinations for VPN clients.
#[derive(Default)]
pub struct InternetExit {
    policy: EgressPolicy,
    conns: HashMap<(/*client*/ [u8; 32], /*stream*/ u64), Conn>,
    max_conns: usize,
    /// Optional interface index to bind egress to (macOS `IP_BOUND_IF`). Needed when the
    /// exit runs on the same host as a TUN client, so the exit's own connections bypass the
    /// tunnel's routes instead of looping back in.
    bind_if: Option<u32>,
}

impl InternetExit {
    pub fn new(policy: EgressPolicy) -> Self {
        Self {
            policy,
            conns: HashMap::new(),
            max_conns: 4096,
            bind_if: None,
        }
    }

    /// Bind outbound connections to a specific interface index (see [`InternetExit::bind_if`]).
    pub fn with_bind_interface(mut self, idx: Option<u32>) -> Self {
        self.bind_if = idx;
        self
    }

    /// Connect to `dst`, optionally pinning egress to `bind_if` so it bypasses tunnel routes.
    async fn connect(dst: SocketAddr, bind_if: Option<u32>) -> io::Result<TcpStream> {
        match bind_if {
            #[cfg(target_os = "macos")]
            Some(idx) if idx != 0 => {
                use tokio::net::TcpSocket;
                let sock = if dst.is_ipv4() {
                    TcpSocket::new_v4()?
                } else {
                    TcpSocket::new_v6()?
                };
                if let Some(nz) = std::num::NonZeroU32::new(idx) {
                    let r = socket2::SockRef::from(&sock);
                    if dst.is_ipv4() {
                        r.bind_device_by_index_v4(Some(nz))?;
                    } else {
                        r.bind_device_by_index_v6(Some(nz))?;
                    }
                }
                sock.connect(dst).await
            }
            _ => TcpStream::connect(dst).await,
        }
    }

    /// Read whatever is currently available (up to a deadline / cap) into an `ExitFrame`.
    async fn drain(conn: &mut Conn) -> ExitFrame {
        if conn.eof {
            return ExitFrame::Eof;
        }
        let mut buf = vec![0u8; MAX_READ_PER_TICK];
        match tokio::time::timeout(READ_DEADLINE, conn.sock.read(&mut buf)).await {
            Ok(Ok(0)) => {
                conn.eof = true;
                ExitFrame::Eof
            }
            Ok(Ok(n)) => {
                buf.truncate(n);
                ExitFrame::Data(buf)
            }
            Ok(Err(_)) => ExitFrame::Err(exit_err::IO),
            // Nothing ready within the deadline — return an empty chunk; the client polls.
            Err(_) => ExitFrame::Data(Vec::new()),
        }
    }

    async fn handle_frame(&mut self, client: [u8; 32], frame: ClientFrame) -> ExitFrame {
        match frame {
            ClientFrame::Open { stream_id, dst } => {
                if !self.policy.permits(&dst) {
                    return ExitFrame::Err(exit_err::POLICY_BLOCKED);
                }
                if self.conns.len() >= self.max_conns {
                    return ExitFrame::Err(exit_err::IO);
                }
                match tokio::time::timeout(CONNECT_TIMEOUT, Self::connect(dst, self.bind_if)).await
                {
                    Ok(Ok(sock)) => {
                        let _ = sock.set_nodelay(true);
                        self.conns
                            .insert((client, stream_id), Conn { sock, eof: false });
                        // No data to read yet right after connect.
                        ExitFrame::Data(Vec::new())
                    }
                    _ => ExitFrame::Err(exit_err::CONNECT_FAILED),
                }
            }
            ClientFrame::Data {
                stream_id, data, ..
            } => {
                let Some(conn) = self.conns.get_mut(&(client, stream_id)) else {
                    return ExitFrame::Err(exit_err::NO_STREAM);
                };
                if !data.is_empty() {
                    if let Err(_e) = conn.sock.write_all(&data).await {
                        self.conns.remove(&(client, stream_id));
                        return ExitFrame::Err(exit_err::IO);
                    }
                }
                let out = Self::drain(conn).await;
                if matches!(out, ExitFrame::Eof | ExitFrame::Err(_)) {
                    self.conns.remove(&(client, stream_id));
                }
                out
            }
            ClientFrame::Poll { stream_id } => {
                let Some(conn) = self.conns.get_mut(&(client, stream_id)) else {
                    return ExitFrame::Err(exit_err::NO_STREAM);
                };
                let out = Self::drain(conn).await;
                if matches!(out, ExitFrame::Eof | ExitFrame::Err(_)) {
                    self.conns.remove(&(client, stream_id));
                }
                out
            }
            ClientFrame::Close { stream_id } => {
                self.conns.remove(&(client, stream_id));
                ExitFrame::Eof
            }
        }
    }
}

impl Exit for InternetExit {
    fn handle<'a>(
        &'a mut self,
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

        let mut exit = InternetExit::new(EgressPolicy::allowlist(vec![origin.ip()]));
        let client = [9u8; 32];

        // Open → connect.
        let r = exit
            .handle(
                client,
                [0u8; 32],
                &ClientFrame::Open {
                    stream_id: 1,
                    dst: origin,
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
        let mut exit =
            InternetExit::new(EgressPolicy::allowlist(vec!["127.0.0.1".parse().unwrap()]));
        let blocked: SocketAddr = "8.8.8.8:53".parse().unwrap();
        let r = exit
            .handle(
                [0u8; 32],
                [0u8; 32],
                &ClientFrame::Open {
                    stream_id: 1,
                    dst: blocked,
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
