use crate::mocks::connection_handler_mock::MockCacheManager;
use sockudo_adapter::ConnectionManager;
use sockudo_adapter::handler::ConnectionHandler;
use sockudo_adapter::local_adapter::LocalAdapter;
use sockudo_app::memory_app_manager::MemoryAppManager;
use sockudo_core::app::{App, AppManager, AppPolicy};
use sockudo_core::options::ServerOptions;
use sockudo_core::websocket::{SocketId, WebSocketBufferConfig};
use sockudo_protocol::{AppendMode, ProtocolVersion, WireFormat};
use sockudo_ws::axum_integration::{WebSocket, WebSocketWriter};
use sockudo_ws::client::WebSocketClient;
use sockudo_ws::{Config as WsConfig, Http1, Stream as WsStream, WebSocketStream};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

const APP_ID: &str = "activity-timeout-test";
type TestClient = WebSocketStream<WsStream<Http1>>;

struct Harness {
    handler: ConnectionHandler,
    adapter: Arc<LocalAdapter>,
    app_manager: Arc<MemoryAppManager>,
    app: App,
}

async fn create_test_pair() -> (WebSocketWriter, TestClient) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        sockudo_ws::handshake::server_handshake(&mut stream)
            .await
            .unwrap();
        let socket = WebSocket::from_tcp(stream, WsConfig::default());
        let (mut reader, writer) = socket.split();
        tokio::spawn(async move {
            while let Some(message) = reader.next().await {
                if message.is_err() {
                    break;
                }
            }
        });
        writer
    });

    let stream = TcpStream::connect(address).await.unwrap();
    let client = WebSocketClient::<Http1>::new(WsConfig::default());
    let (client, _): (TestClient, _) = client
        .connect(stream, &address.to_string(), "/", None)
        .await
        .unwrap();

    (server.await.unwrap(), client)
}

async fn build_harness() -> Harness {
    let app = App::from_policy(
        APP_ID.to_string(),
        "activity-timeout-key".to_string(),
        "activity-timeout-secret".to_string(),
        true,
        AppPolicy::default(),
    );
    let app_manager = Arc::new(MemoryAppManager::new());
    app_manager.create_app(app.clone()).await.unwrap();

    let adapter = Arc::new(LocalAdapter::new());
    adapter.init().await;
    let handler = ConnectionHandler::builder(
        app_manager.clone() as Arc<dyn AppManager + Send + Sync>,
        adapter.clone() as Arc<dyn ConnectionManager + Send + Sync>,
        Arc::new(MockCacheManager::new()),
        ServerOptions::default(),
    )
    .local_adapter(adapter.clone())
    .build();

    Harness {
        handler,
        adapter,
        app_manager,
        app,
    }
}

async fn add_socket(
    harness: &Harness,
    protocol_version: ProtocolVersion,
) -> (SocketId, TestClient) {
    let socket_id = SocketId::new();
    let (writer, client) = create_test_pair().await;
    harness
        .adapter
        .add_socket(
            socket_id,
            writer,
            &harness.app.id,
            harness.app_manager.clone() as Arc<dyn AppManager + Send + Sync>,
            WebSocketBufferConfig::default(),
            protocol_version,
            WireFormat::Json,
            true,
            AppendMode::Delta,
        )
        .await
        .unwrap();
    (socket_id, client)
}

#[tokio::test]
async fn initial_activity_timeout_task_is_installed_only_for_v1() {
    let harness = build_harness().await;
    let (v2_socket_id, _v2_client) = add_socket(&harness, ProtocolVersion::V2).await;
    let (v1_socket_id, _v1_client) = add_socket(&harness, ProtocolVersion::V1).await;

    harness
        .handler
        .setup_initial_timeouts(&v2_socket_id, &harness.app)
        .await
        .unwrap();
    harness
        .handler
        .setup_initial_timeouts(&v1_socket_id, &harness.app)
        .await
        .unwrap();

    let v2_connection = harness
        .adapter
        .get_connection(&v2_socket_id, APP_ID)
        .await
        .unwrap();
    let v2_has_legacy_timeout = v2_connection
        .inner
        .lock()
        .await
        .state
        .timeouts
        .activity_timeout_handle
        .is_some();
    assert!(
        !v2_has_legacy_timeout,
        "V2 must not spawn the application-level timeout task"
    );

    let v1_connection = harness
        .adapter
        .get_connection(&v1_socket_id, APP_ID)
        .await
        .unwrap();
    let v1_timeout_is_running = v1_connection
        .inner
        .lock()
        .await
        .state
        .timeouts
        .activity_timeout_handle
        .as_ref()
        .is_some_and(|handle| !handle.is_finished());
    assert!(
        v1_timeout_is_running,
        "V1 must retain the Pusher-compatible application timeout task"
    );

    harness
        .handler
        .clear_activity_timeout(APP_ID, &v1_socket_id)
        .await
        .unwrap();
}
