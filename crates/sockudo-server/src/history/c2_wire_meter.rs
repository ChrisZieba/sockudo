//! Opt-in C2 diagnostics: transparent, bounded TCP forwarding with byte counts.
//!
//! Counters measure bytes successfully written to the opposite socket, including
//! driver protocol framing, query text, read requests, retries and TLS records
//! where applicable. They exclude TCP/IP headers. Snapshot after setup and
//! around the append phase; background connection traffic in that interval is
//! intentionally included. This proxy changes timing and is for separate byte
//! measurement runs, never for the original latency series.
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

const MAX_CONNECTIONS: usize = 32;
const COPY_BUFFER_BYTES: usize = 16 * 1024;

static PHASE_COUNTERS: Mutex<Weak<Counters>> = Mutex::new(Weak::new());
static PHASES_ENABLED: OnceLock<bool> = OnceLock::new();

/// Test-only component attribution for separate wire-volume runs.
pub(super) fn phase_snapshot() -> Option<(u64, u64)> {
    if !*PHASES_ENABLED.get_or_init(|| std::env::var_os("C2_WIRE_PHASES").is_some()) {
        return None;
    }
    let counters = PHASE_COUNTERS.lock().ok()?.upgrade()?;
    Some((
        counters.client_bytes.load(Ordering::Relaxed),
        counters.server_bytes.load(Ordering::Relaxed),
    ))
}

pub(super) fn report_phase(label: &'static str, before: Option<(u64, u64)>) {
    if let (Some((sent, received)), Some((after_sent, after_received))) = (before, phase_snapshot())
    {
        println!(
            "wire_phase,{label},client_bytes,{},server_bytes,{}",
            after_sent.saturating_sub(sent),
            after_received.saturating_sub(received)
        );
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct WireSnapshot {
    pub client_bytes: u64,
    pub server_bytes: u64,
    pub accepted_connections: u64,
    pub failed_connections: u64,
}

impl WireSnapshot {
    pub fn since(self, before: Self) -> Self {
        Self {
            client_bytes: self.client_bytes.saturating_sub(before.client_bytes),
            server_bytes: self.server_bytes.saturating_sub(before.server_bytes),
            accepted_connections: self
                .accepted_connections
                .saturating_sub(before.accepted_connections),
            failed_connections: self
                .failed_connections
                .saturating_sub(before.failed_connections),
        }
    }
}

#[derive(Default)]
struct Counters {
    client_bytes: AtomicU64,
    server_bytes: AtomicU64,
    accepted_connections: AtomicU64,
    failed_connections: AtomicU64,
}

/// One listener, at most 32 connection tasks and two 16 KiB copy buffers per
/// connection. Dropping the meter aborts the listener and every forwarding task.
pub(super) struct WireMeter {
    address: SocketAddr,
    counters: Arc<Counters>,
    task: JoinHandle<()>,
}

impl WireMeter {
    pub async fn start(target_port: u16) -> io::Result<Self> {
        let upstream = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, target_port));
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let counters = Arc::new(Counters::default());
        *PHASE_COUNTERS.lock().expect("wire phase registry poisoned") = Arc::downgrade(&counters);
        let task_counters = counters.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    result = connections.join_next(), if !connections.is_empty() => {
                        if let Some(Err(error)) = result {
                            task_counters.failed_connections.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(error = %error, "wire measurement forwarding task failed");
                        }
                    }
                    accepted = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                        let (client, _) = match accepted {
                            Ok(client) => client,
                            Err(error) => {
                                task_counters.failed_connections.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(error = %error, "wire measurement accept failed");
                                break;
                            }
                        };
                        task_counters.accepted_connections.fetch_add(1, Ordering::Relaxed);
                        let counters = task_counters.clone();
                        connections.spawn(async move {
                            if let Err(error) = forward_connection(client, upstream, &counters).await {
                                counters.failed_connections.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(error = %error, "wire measurement forwarding failed");
                            }
                        });
                    }
                }
            }
        });
        Ok(Self {
            address,
            counters,
            task,
        })
    }

    pub fn local_port(&self) -> u16 {
        self.address.port()
    }

    pub fn sent_bytes(&self) -> u64 {
        self.counters.client_bytes.load(Ordering::Relaxed)
    }

    pub fn received_bytes(&self) -> u64 {
        self.counters.server_bytes.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> WireSnapshot {
        WireSnapshot {
            client_bytes: self.counters.client_bytes.load(Ordering::Relaxed),
            server_bytes: self.counters.server_bytes.load(Ordering::Relaxed),
            accepted_connections: self.counters.accepted_connections.load(Ordering::Relaxed),
            failed_connections: self.counters.failed_connections.load(Ordering::Relaxed),
        }
    }
}

impl Drop for WireMeter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward_connection(
    client: TcpStream,
    upstream: SocketAddr,
    counters: &Counters,
) -> io::Result<()> {
    let server = TcpStream::connect(upstream).await?;
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    tokio::try_join!(
        copy_counted(client_read, server_write, &counters.client_bytes),
        copy_counted(server_read, client_write, &counters.server_bytes),
    )?;
    Ok(())
}

async fn copy_counted<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    counter: &AtomicU64,
) -> io::Result<()> {
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            writer.shutdown().await?;
            return Ok(());
        }
        let mut offset = 0;
        while offset < read {
            let written = writer.write(&buffer[offset..read]).await?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "proxy write stalled",
                ));
            }
            counter.fetch_add(written as u64, Ordering::Relaxed);
            offset += written;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wire_meter_counts_forwarded_bytes_in_both_directions() {
        let upstream = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let meter = WireMeter::start(upstream.local_addr().unwrap().port())
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut request = Vec::new();
            socket.read_to_end(&mut request).await.unwrap();
            assert_eq!(request, vec![127u8; COPY_BUFFER_BYTES + 3]);
            socket.write_all(b"response").await.unwrap();
        });
        let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, meter.local_port()))
            .await
            .unwrap();
        let before = meter.snapshot();
        client
            .write_all(&vec![127u8; COPY_BUFFER_BYTES + 3])
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"response");
        server.await.unwrap();
        let observed = meter.snapshot().since(before);
        assert_eq!(observed.client_bytes, COPY_BUFFER_BYTES as u64 + 3);
        assert_eq!(observed.server_bytes, 8);
        assert_eq!(observed.failed_connections, 0);
    }
}
