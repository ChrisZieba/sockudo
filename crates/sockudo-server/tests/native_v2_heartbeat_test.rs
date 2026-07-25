use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use sockudo_core::options::WebSocketConfig;
use sockudo_ws::axum_integration::WebSocketUpgrade;
use sockudo_ws::{Error as WebSocketError, Message};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const PONG_TIMEOUT_CLOSE_CODE: u16 = 4201;
const PONG_TIMEOUT_CLOSE_REASON: &[u8] = b"Pong reply not received in time";
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
struct HeartbeatState {
    events: mpsc::UnboundedSender<ServerEvent>,
}

#[derive(Debug, PartialEq, Eq)]
enum ServerEvent {
    Pong(Vec<u8>),
    HeartbeatTimeout,
    UnexpectedError(String),
}

struct TestServer {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
struct ServerFrame {
    opcode: u8,
    payload: Vec<u8>,
}

fn v2_heartbeat_test_config() -> sockudo_ws::Config {
    let websocket = WebSocketConfig {
        ping_interval: 1,
        idle_timeout: 0,
        ..WebSocketConfig::default()
    };
    let mut config = websocket.to_sockudo_ws_config_with_native_heartbeat(64, 2, true);

    assert!(config.auto_ping, "Protocol V2 must enable native auto-ping");
    assert_eq!(config.pong_timeout_close_code, PONG_TIMEOUT_CLOSE_CODE);
    assert_eq!(
        config.pong_timeout_close_reason.as_bytes(),
        PONG_TIMEOUT_CLOSE_REASON
    );

    // Runtime values are deliberately shortened for this integration test.
    // The production Pong deadline and close contract remain asserted above.
    config.ping_interval = 1;
    config.pong_timeout = 1;
    config.close_timeout = 1;
    config
}

async fn heartbeat_handler(
    State(state): State<HeartbeatState>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade
        .config(v2_heartbeat_test_config())
        .on_upgrade(move |socket| async move {
            let (mut reader, _writer) = socket.split();
            while let Some(result) = reader.next().await {
                let event = match result {
                    Ok(Message::Pong(payload)) => Some(ServerEvent::Pong(payload.to_vec())),
                    Err(WebSocketError::HeartbeatTimeout) => Some(ServerEvent::HeartbeatTimeout),
                    Err(error) => Some(ServerEvent::UnexpectedError(error.to_string())),
                    Ok(_) => None,
                };
                if let Some(event) = event
                    && state.events.send(event).is_err()
                {
                    break;
                }
            }
        })
}

async fn spawn_test_server() -> (TestServer, mpsc::UnboundedReceiver<ServerEvent>) {
    let (events, event_receiver) = mpsc::unbounded_channel();
    let state = HeartbeatState { events };
    let app = Router::new()
        .route("/app/test", get(heartbeat_handler))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let address = listener
        .local_addr()
        .expect("test server should have a local address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server should run");
    });

    (TestServer { address, task }, event_receiver)
}

async fn connect_raw_websocket(address: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("test client should connect");
    let request = format!(
        "GET /app/test?protocol=2 HTTP/1.1\r\n\
         Host: {address}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         \r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("WebSocket handshake should be written");
    stream
        .flush()
        .await
        .expect("WebSocket handshake should be flushed");

    let response = timeout(FRAME_TIMEOUT, async {
        let mut response = Vec::new();
        let mut byte = [0_u8; 1];
        while !response.ends_with(b"\r\n\r\n") {
            stream
                .read_exact(&mut byte)
                .await
                .expect("WebSocket handshake response should be readable");
            response.push(byte[0]);
        }
        response
    })
    .await
    .expect("timed out waiting for WebSocket handshake");
    let response = String::from_utf8(response).expect("handshake response should be UTF-8");
    assert!(
        response.starts_with("HTTP/1.1 101"),
        "expected switching protocols response, got: {response}"
    );

    stream
}

async fn read_server_frame(stream: &mut TcpStream) -> ServerFrame {
    timeout(FRAME_TIMEOUT, async {
        let mut header = [0_u8; 2];
        stream
            .read_exact(&mut header)
            .await
            .expect("server frame header should be readable");
        assert_ne!(header[0] & 0x80, 0, "server control frame must be final");
        assert_eq!(header[1] & 0x80, 0, "server frames must not be masked");

        let payload_length = usize::from(header[1] & 0x7f);
        assert!(
            payload_length <= 125,
            "control frame payload must fit in one byte"
        );
        let mut payload = vec![0; payload_length];
        stream
            .read_exact(&mut payload)
            .await
            .expect("server frame payload should be readable");

        ServerFrame {
            opcode: header[0] & 0x0f,
            payload,
        }
    })
    .await
    .expect("timed out waiting for server frame")
}

async fn send_masked_pong(stream: &mut TcpStream, payload: &[u8]) {
    let mask = [0x11, 0x22, 0x33, 0x44];
    let mut frame = Vec::with_capacity(6 + payload.len());
    frame.push(0x8a);
    frame.push(0x80 | payload.len() as u8);
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    stream
        .write_all(&frame)
        .await
        .expect("Pong frame should be written");
    stream.flush().await.expect("Pong frame should be flushed");
}

async fn next_server_event(events: &mut mpsc::UnboundedReceiver<ServerEvent>) -> ServerEvent {
    timeout(FRAME_TIMEOUT, events.recv())
        .await
        .expect("timed out waiting for server heartbeat event")
        .expect("server heartbeat event channel closed")
}

#[tokio::test]
async fn native_v2_heartbeat_accepts_matching_pong_and_keeps_connection_open() {
    let (server, mut events) = spawn_test_server().await;
    let mut client = connect_raw_websocket(server.address).await;

    let first_ping = read_server_frame(&mut client).await;
    assert_eq!(first_ping.opcode, 0x09);
    assert_eq!(first_ping.payload.len(), 8, "Ping must contain a nonce");
    send_masked_pong(&mut client, &first_ping.payload).await;
    assert_eq!(
        next_server_event(&mut events).await,
        ServerEvent::Pong(first_ping.payload.clone())
    );

    let second_ping = read_server_frame(&mut client).await;
    assert_eq!(
        second_ping.opcode, 0x09,
        "a matching Pong must keep the connection alive for the next Ping"
    );
    assert_eq!(second_ping.payload.len(), 8, "Ping must contain a nonce");
    assert_ne!(
        second_ping.payload, first_ping.payload,
        "each Ping must use a fresh nonce"
    );
    send_masked_pong(&mut client, &second_ping.payload).await;
    assert_eq!(
        next_server_event(&mut events).await,
        ServerEvent::Pong(second_ping.payload)
    );
}

#[tokio::test]
async fn native_v2_heartbeat_closes_nonresponsive_peer_with_4201() {
    let (server, mut events) = spawn_test_server().await;
    let mut client = connect_raw_websocket(server.address).await;

    let ping = read_server_frame(&mut client).await;
    assert_eq!(ping.opcode, 0x09);
    assert_eq!(ping.payload.len(), 8, "Ping must contain a nonce");

    let close = read_server_frame(&mut client).await;
    assert_eq!(close.opcode, 0x08);
    assert!(close.payload.len() >= 2, "Close frame must include a code");
    assert_eq!(
        u16::from_be_bytes([close.payload[0], close.payload[1]]),
        PONG_TIMEOUT_CLOSE_CODE
    );
    assert_eq!(&close.payload[2..], PONG_TIMEOUT_CLOSE_REASON);
    assert_eq!(
        next_server_event(&mut events).await,
        ServerEvent::HeartbeatTimeout
    );
}
