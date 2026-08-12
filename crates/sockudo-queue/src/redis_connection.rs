use async_trait::async_trait;
#[cfg(feature = "redis-cluster")]
use parking_lot::Mutex;
use redis::aio::ConnectionManager;
#[cfg(feature = "redis-cluster")]
use redis::cluster::{ClusterClient, ClusterClientBuilder};
#[cfg(feature = "redis-cluster")]
use redis::cluster_async::ClusterConnection;
#[cfg(feature = "redis-cluster")]
use sockudo_core::error::Error;
use sockudo_core::error::Result;
use sockudo_core::options::SentinelSpec;
use sockudo_core::queue::QueueBackendKind;
use sockudo_core::redis_client::RedisClient;
#[cfg(feature = "redis-cluster")]
use std::sync::Arc;
use std::time::Duration;

const MINIMUM_BLOCKING_WAIT_MS: u64 = 10;

/// Gives a blocking Redis command its finite server-side wait plus the normal
/// response budget. Matching those deadlines makes a successful nil response
/// race the client's timeout when the queue is idle.
pub(crate) fn blocking_response_timeout(
    response_timeout_ms: u64,
    worker_poll_interval_ms: u64,
) -> Option<Duration> {
    (response_timeout_ms > 0).then(|| {
        Duration::from_millis(worker_poll_interval_ms.max(MINIMUM_BLOCKING_WAIT_MS))
            .saturating_add(Duration::from_millis(response_timeout_ms))
    })
}

#[async_trait]
pub(crate) trait QueueRedisProvider: Clone + Send + Sync + 'static {
    type Connection: redis::aio::ConnectionLike + Clone + Send + 'static;

    async fn command_connection(&self) -> Result<Self::Connection>;
    async fn worker_connection(&self) -> Result<Self::Connection>;
    fn invalidate(&self);
    fn backend(&self) -> QueueBackendKind;
}

#[derive(Clone)]
pub(crate) struct StandaloneRedisProvider {
    client: RedisClient,
    worker_response_timeout: Option<Duration>,
}

impl StandaloneRedisProvider {
    pub(crate) async fn connect(
        url: &str,
        sentinel: Option<SentinelSpec>,
        worker_response_timeout: Option<Duration>,
    ) -> Result<Self> {
        Ok(Self {
            client: RedisClient::connect(url, sentinel).await?,
            worker_response_timeout,
        })
    }
}

#[async_trait]
impl QueueRedisProvider for StandaloneRedisProvider {
    type Connection = ConnectionManager;

    async fn command_connection(&self) -> Result<Self::Connection> {
        self.client.command_connection().await
    }

    async fn worker_connection(&self) -> Result<Self::Connection> {
        self.client
            .fresh_connection_manager_with_response_timeout(self.worker_response_timeout)
            .await
    }

    fn invalidate(&self) {
        self.client.invalidate();
    }

    fn backend(&self) -> QueueBackendKind {
        if self.client.is_sentinel() {
            QueueBackendKind::RedisSentinel
        } else {
            QueueBackendKind::Redis
        }
    }
}

#[cfg(feature = "redis-cluster")]
struct ClusterInner {
    command_client: ClusterClient,
    worker_client: ClusterClient,
    command: Mutex<Option<ClusterConnection>>,
}

#[cfg(feature = "redis-cluster")]
#[derive(Clone)]
pub(crate) struct ClusterRedisProvider {
    inner: Arc<ClusterInner>,
}

#[cfg(feature = "redis-cluster")]
impl ClusterRedisProvider {
    pub(crate) async fn connect(
        nodes: Vec<String>,
        request_timeout_ms: u64,
        worker_poll_interval_ms: u64,
    ) -> Result<Self> {
        if nodes.is_empty() {
            return Err(Error::Config(
                "Redis Cluster queue requires at least one seed node".to_string(),
            ));
        }
        let response_timeout =
            (request_timeout_ms > 0).then(|| Duration::from_millis(request_timeout_ms));
        let command_client = build_cluster_client(nodes.clone(), response_timeout)?;
        let worker_client = build_cluster_client(
            nodes,
            blocking_response_timeout(request_timeout_ms, worker_poll_interval_ms),
        )?;
        let connection = command_client
            .get_async_connection()
            .await
            .map_err(|error| {
                Error::Connection(format!("failed to connect to Redis Cluster: {error}"))
            })?;
        Ok(Self {
            inner: Arc::new(ClusterInner {
                command_client,
                worker_client,
                command: Mutex::new(Some(connection)),
            }),
        })
    }

    async fn build_connection(client: &ClusterClient) -> Result<ClusterConnection> {
        client.get_async_connection().await.map_err(|error| {
            Error::Connection(format!("failed to connect to Redis Cluster: {error}"))
        })
    }
}

#[cfg(feature = "redis-cluster")]
#[async_trait]
impl QueueRedisProvider for ClusterRedisProvider {
    type Connection = ClusterConnection;

    async fn command_connection(&self) -> Result<Self::Connection> {
        if let Some(connection) = self.inner.command.lock().as_ref() {
            return Ok(connection.clone());
        }
        let connection = Self::build_connection(&self.inner.command_client).await?;
        *self.inner.command.lock() = Some(connection.clone());
        Ok(connection)
    }

    async fn worker_connection(&self) -> Result<Self::Connection> {
        Self::build_connection(&self.inner.worker_client).await
    }

    fn invalidate(&self) {
        *self.inner.command.lock() = None;
    }

    fn backend(&self) -> QueueBackendKind {
        QueueBackendKind::RedisCluster
    }
}

#[cfg(feature = "redis-cluster")]
fn build_cluster_client(
    nodes: Vec<String>,
    response_timeout: Option<Duration>,
) -> Result<ClusterClient> {
    let builder = ClusterClientBuilder::new(nodes).overall_response_timeout(response_timeout);
    let builder = match response_timeout {
        Some(timeout) => builder.response_timeout(timeout),
        None => builder,
    };
    builder
        .build()
        .map_err(|error| Error::Config(format!("failed to create Redis Cluster client: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_timeout_includes_server_wait_and_response_budget() {
        assert_eq!(
            blocking_response_timeout(500, 500),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn blocking_timeout_uses_same_minimum_as_notification_wait() {
        assert_eq!(
            blocking_response_timeout(500, 1),
            Some(Duration::from_millis(510))
        );
    }

    #[test]
    fn blocking_timeout_can_be_disabled() {
        assert_eq!(blocking_response_timeout(0, 500), None);
    }

    #[tokio::test]
    #[ignore = "requires SOCKUDO_REDIS_QUEUE_TEST_URL"]
    async fn blocking_worker_connection_outlives_empty_redis_wait() {
        let url = std::env::var("SOCKUDO_REDIS_QUEUE_TEST_URL")
            .expect("SOCKUDO_REDIS_QUEUE_TEST_URL is required");
        let provider =
            StandaloneRedisProvider::connect(&url, None, blocking_response_timeout(500, 500))
                .await
                .expect("Redis queue provider should connect");
        let mut connection = provider
            .worker_connection()
            .await
            .expect("worker connection should connect");
        let key = format!(
            "sockudo_queue_timeout_test:{}",
            uuid::Uuid::new_v4().simple()
        );

        let result = redis::cmd("BLPOP")
            .arg(key)
            .arg(0.5)
            .query_async::<Option<(String, String)>>(&mut connection)
            .await;

        assert_eq!(
            result.expect("empty blocking wait should not time out"),
            None
        );
    }

    #[cfg(feature = "redis-cluster")]
    #[tokio::test]
    #[ignore = "requires SOCKUDO_REDIS_CLUSTER_QUEUE_TEST_NODES"]
    async fn blocking_cluster_worker_connection_outlives_empty_redis_wait() {
        let nodes = std::env::var("SOCKUDO_REDIS_CLUSTER_QUEUE_TEST_NODES")
            .expect("SOCKUDO_REDIS_CLUSTER_QUEUE_TEST_NODES is required")
            .split(',')
            .map(str::to_string)
            .collect();
        let provider = ClusterRedisProvider::connect(nodes, 500, 500)
            .await
            .expect("Redis Cluster queue provider should connect");
        let mut connection = provider
            .worker_connection()
            .await
            .expect("cluster worker connection should connect");
        let key = format!(
            "sockudo_queue_timeout_test:{{{}}}",
            uuid::Uuid::new_v4().simple()
        );

        let result = redis::cmd("BLPOP")
            .arg(key)
            .arg(0.5)
            .query_async::<Option<(String, String)>>(&mut connection)
            .await;

        assert_eq!(
            result.expect("empty cluster blocking wait should not time out"),
            None
        );
    }
}
