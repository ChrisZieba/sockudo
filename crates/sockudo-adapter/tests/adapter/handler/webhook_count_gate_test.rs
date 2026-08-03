use ahash::AHashMap;
use async_trait::async_trait;
use sockudo_adapter::ConnectionManager;
use sockudo_adapter::connection_manager::{ChannelSocketCount, HorizontalAdapterInterface};
use sockudo_adapter::handler::ConnectionHandler;
use sockudo_adapter::handler::subscription_management::SubscriptionResult;
use sockudo_adapter::handler::types::SubscriptionRequest;
use sockudo_app::memory_app_manager::MemoryAppManager;
use sockudo_core::app::{App, AppManager, AppPolicy};
use sockudo_core::channel::PresenceMemberInfo;
use sockudo_core::error::Result;
use sockudo_core::namespace::Namespace;
use sockudo_core::options::ServerOptions;
use sockudo_core::webhook_types::{Webhook, WebhookFilter};
use sockudo_core::websocket::{SocketId, WebSocketRef};
use sockudo_protocol::messages::{MessageData, PusherMessage};
use sockudo_queue::manager::QueueManagerFactory;
use sockudo_webhook::integration::{QueueManager, WebhookConfig, WebhookIntegration};
use sockudo_ws::axum_integration::WebSocketWriter;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// No-op [`ConnectionManager`] that counts calls to the channel-socket-count
/// methods. Everything else delegates to [`crate::mocks::connection_handler_mock::MockAdapter`]'s
/// behavior (all no-ops) without importing it, keeping this file self-contained.
struct CountingAdapter {
    count_query_calls: Arc<AtomicUsize>,
}

impl CountingAdapter {
    fn new() -> Self {
        Self {
            count_query_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl ConnectionManager for CountingAdapter {
    async fn init(&self) {}
    async fn get_namespace(&self, _app_id: &str) -> Option<Arc<Namespace>> {
        None
    }
    async fn add_socket(
        &self,
        _socket_id: SocketId,
        _socket: WebSocketWriter,
        _app_id: &str,
        _app_manager: Arc<dyn AppManager + Send + Sync>,
        _buffer_config: sockudo_core::websocket::WebSocketBufferConfig,
        _protocol_version: sockudo_protocol::ProtocolVersion,
        _wire_format: sockudo_protocol::WireFormat,
        _echo_messages: bool,
        _append_mode: sockudo_protocol::AppendMode,
    ) -> Result<()> {
        Ok(())
    }
    async fn get_connection(&self, _socket_id: &SocketId, _app_id: &str) -> Option<WebSocketRef> {
        None
    }
    async fn remove_connection(&self, _socket_id: &SocketId, _app_id: &str) -> Result<()> {
        Ok(())
    }
    async fn send_message(
        &self,
        _app_id: &str,
        _socket_id: &SocketId,
        _message: PusherMessage,
    ) -> Result<()> {
        Ok(())
    }
    async fn send(
        &self,
        _channel: &str,
        _message: PusherMessage,
        _except: Option<&SocketId>,
        _app_id: &str,
        _start_time_ms: Option<f64>,
    ) -> Result<()> {
        Ok(())
    }
    async fn get_channel_members(
        &self,
        _app_id: &str,
        _channel: &str,
    ) -> Result<AHashMap<String, PresenceMemberInfo>> {
        Ok(AHashMap::new())
    }
    async fn get_channel_sockets(&self, _app_id: &str, _channel: &str) -> Result<Vec<SocketId>> {
        Ok(Vec::new())
    }
    async fn remove_channel(&self, _app_id: &str, _channel: &str) {}
    async fn is_in_channel(
        &self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<bool> {
        Ok(false)
    }
    async fn get_user_sockets(&self, _user_id: &str, _app_id: &str) -> Result<Vec<WebSocketRef>> {
        Ok(Vec::new())
    }
    async fn cleanup_connection(&self, _app_id: &str, _ws: WebSocketRef) {}
    async fn terminate_connection(&self, _app_id: &str, _user_id: &str) -> Result<()> {
        Ok(())
    }
    async fn add_channel_to_sockets(&self, _app_id: &str, _channel: &str, _socket_id: &SocketId) {}
    async fn get_channel_socket_count_info(
        &self,
        _app_id: &str,
        _channel: &str,
    ) -> ChannelSocketCount {
        self.count_query_calls.fetch_add(1, Ordering::SeqCst);
        ChannelSocketCount {
            count: 0,
            complete: true,
        }
    }
    async fn get_channel_socket_count(&self, _app_id: &str, _channel: &str) -> usize {
        self.count_query_calls.fetch_add(1, Ordering::SeqCst);
        0
    }
    async fn get_local_channel_socket_count(&self, _app_id: &str, _channel: &str) -> usize {
        0
    }
    async fn add_to_channel(
        &self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<(bool, bool)> {
        Ok((false, false))
    }
    async fn remove_from_channel(
        &self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<(bool, bool)> {
        Ok((false, false))
    }
    async fn get_presence_member(
        &self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Option<PresenceMemberInfo> {
        None
    }
    async fn terminate_user_connections(&self, _app_id: &str, _user_id: &str) -> Result<()> {
        Ok(())
    }
    async fn force_reconnect_user(&self, _app_id: &str, _user_id: &str) -> Result<()> {
        Ok(())
    }
    async fn add_user(&self, _ws: WebSocketRef) -> Result<()> {
        Ok(())
    }
    async fn remove_user(&self, _ws: WebSocketRef) -> Result<()> {
        Ok(())
    }
    async fn get_channels_with_socket_count(
        &self,
        _app_id: &str,
    ) -> Result<AHashMap<String, usize>> {
        Ok(AHashMap::new())
    }
    async fn get_sockets_count(&self, _app_id: &str) -> Result<usize> {
        Ok(0)
    }
    async fn get_namespaces(&self) -> Result<Vec<(String, Arc<Namespace>)>> {
        Ok(Vec::new())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    async fn remove_user_socket(
        &self,
        _user_id: &str,
        _socket_id: &SocketId,
        _app_id: &str,
    ) -> Result<()> {
        Ok(())
    }
    async fn count_user_connections_in_channel(
        &self,
        _user_id: &str,
        _app_id: &str,
        _channel: &str,
        _excluding_socket: Option<&SocketId>,
    ) -> Result<usize> {
        Ok(0)
    }
    async fn check_health(&self) -> Result<()> {
        Ok(())
    }
    fn get_node_id(&self) -> String {
        "counting-mock".to_string()
    }
    fn as_horizontal_adapter(&self) -> Option<&dyn HorizontalAdapterInterface> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_presence_filtered_app(app_id: &str) -> App {
    App::from_policy(
        app_id.to_string(),
        format!("{app_id}-key"),
        format!("{app_id}-secret"),
        true,
        AppPolicy {
            webhooks: Some(vec![Webhook {
                event_types: vec![
                    "channel_occupied".to_string(),
                    "channel_vacated".to_string(),
                    "subscription_count".to_string(),
                ],
                filter: Some(WebhookFilter {
                    channel_prefix: Some("presence-".to_string()),
                    ..Default::default()
                }),
                ..Webhook::default()
            }]),
            ..AppPolicy::default()
        },
    )
}

struct Harness {
    handler: ConnectionHandler,
    counting_cm: Arc<CountingAdapter>,
    app: App,
}

async fn build_harness(app_id: &str) -> Harness {
    let app = make_presence_filtered_app(app_id);
    let app_manager = Arc::new(MemoryAppManager::new());
    app_manager.create_app(app.clone()).await.unwrap();

    let counting_cm = Arc::new(CountingAdapter::new());

    let driver = QueueManagerFactory::create("memory", None, None, None, None)
        .await
        .expect("failed to create memory queue for test");
    let queue_manager = Arc::new(QueueManager::new(driver));
    let webhook_integration = Arc::new(
        WebhookIntegration::new(
            WebhookConfig {
                enabled: true,
                ..Default::default()
            },
            app_manager.clone() as Arc<dyn AppManager + Send + Sync>,
            Some(queue_manager),
        )
        .await
        .expect("failed to create webhook integration for test"),
    );

    let handler = ConnectionHandler::builder(
        app_manager as Arc<dyn AppManager + Send + Sync>,
        counting_cm.clone() as Arc<dyn ConnectionManager + Send + Sync>,
        Arc::new(crate::mocks::connection_handler_mock::MockCacheManager::new()),
        ServerOptions::default(),
    )
    .webhook_integration(webhook_integration)
    .build();

    Harness {
        handler,
        counting_cm,
        app,
    }
}

fn make_subscription_result() -> SubscriptionResult {
    SubscriptionResult {
        success: true,
        auth_error: None,
        member: None,
        channel_connections: Some(1),
    }
}

fn make_subscription_request(channel: &str) -> SubscriptionRequest {
    SubscriptionRequest {
        channel: channel.to_string(),
        auth: None,
        channel_data: None,
        #[cfg(feature = "tag-filtering")]
        tags_filter: None,
        #[cfg(feature = "tag-filtering")]
        predicate: None,
        #[cfg(feature = "delta")]
        delta: None,
        rewind: None,
        event_name_filter: None,
        annotation_subscribe: false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_presence_subscribe_skips_count_query() {
    let harness = build_harness("non-presence-sub").await;
    let socket_id = SocketId::new();

    let before = harness.counting_cm.count_query_calls.load(Ordering::SeqCst);

    for channel in &["private-orders", "orders", "private-updates"] {
        harness
            .handler
            .handle_post_subscription(
                &socket_id,
                &harness.app,
                &make_subscription_request(channel),
                &make_subscription_result(),
                None,
            )
            .await
            .ok();
    }

    let after = harness.counting_cm.count_query_calls.load(Ordering::SeqCst);
    assert_eq!(
        after, before,
        "count queries must be zero for non-presence channels with presence-only webhook filter"
    );
}

#[tokio::test]
async fn presence_subscribe_triggers_count_query() {
    let harness = build_harness("presence-sub").await;
    let socket_id = SocketId::new();

    let before = harness.counting_cm.count_query_calls.load(Ordering::SeqCst);

    harness
        .handler
        .handle_post_subscription(
            &socket_id,
            &harness.app,
            &make_subscription_request("presence-lobby"),
            &make_subscription_result(),
            None,
        )
        .await
        .ok();

    let after = harness.counting_cm.count_query_calls.load(Ordering::SeqCst);
    assert!(
        after > before,
        "at least one count query must fire for presence-lobby (got {} calls)",
        after - before
    );
}

#[tokio::test]
async fn non_presence_unsubscribe_skips_count_query() {
    let harness = build_harness("non-presence-unsub").await;
    let socket_id = SocketId::new();

    for channel in &["private-orders", "orders"] {
        let msg = PusherMessage {
            channel: Some(channel.to_string()),
            event: Some("pusher:unsubscribe".to_string()),
            data: Some(MessageData::String(
                sonic_rs::json!({"channel": channel}).to_string(),
            )),
            name: None,
            user_id: None,
            tags: None,
            sequence: None,
            conflation_key: None,
            message_id: None,
            stream_id: None,
            serial: None,
            idempotency_key: None,
            extras: None,
            delta_sequence: None,
            delta_conflation_key: None,
        };

        harness
            .handler
            .handle_unsubscribe(&socket_id, &msg, &harness.app)
            .await
            .ok();
    }

    let after = harness.counting_cm.count_query_calls.load(Ordering::SeqCst);
    assert_eq!(
        after, 0,
        "count queries must be zero for non-presence unsubscribe with presence-only webhook filter"
    );
}
