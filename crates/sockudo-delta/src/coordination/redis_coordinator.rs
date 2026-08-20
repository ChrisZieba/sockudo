use async_trait::async_trait;
use redis::AsyncCommands;
use redis::cluster::ClusterClientBuilder;
use redis::cluster_async::ClusterConnection;
use redis::cluster_read_routing::RandomReplicaStrategy;
use sockudo_core::delta_types::ClusterCoordinator;
use sockudo_core::error::{Error, Result};
use sockudo_core::options::{RedisTlsOptions, SentinelSpec};
use sockudo_core::redis_client::{RedisClient, RedisClientOptions, configure_cluster_builder};
use std::sync::Arc;
use tracing::debug;

enum RedisCoordinationConnection {
    Standard(redis::aio::ConnectionManager),
    Cluster(ClusterConnection),
}

impl RedisCoordinationConnection {
    async fn incr(&mut self, key: &str, value: u32) -> redis::RedisResult<u32> {
        match self {
            Self::Standard(conn) => conn.incr(key, value).await,
            Self::Cluster(conn) => conn.incr(key, value).await,
        }
    }

    async fn expire(&mut self, key: &str, ttl_seconds: i64) -> redis::RedisResult<()> {
        match self {
            Self::Standard(conn) => conn.expire(key, ttl_seconds).await,
            Self::Cluster(conn) => conn.expire(key, ttl_seconds).await,
        }
    }

    async fn set(&mut self, key: &str, value: u32) -> redis::RedisResult<()> {
        match self {
            Self::Standard(conn) => conn.set(key, value).await,
            Self::Cluster(conn) => conn.set(key, value).await,
        }
    }

    async fn del(&mut self, key: &str) -> redis::RedisResult<()> {
        match self {
            Self::Standard(conn) => conn.del(key).await,
            Self::Cluster(conn) => conn.del(key).await,
        }
    }

    async fn get(&mut self, key: &str) -> redis::RedisResult<Option<u32>> {
        match self {
            Self::Standard(conn) => conn.get(key).await,
            Self::Cluster(conn) => conn.get(key).await,
        }
    }
}

/// Redis-based cluster coordinator for delta interval synchronization
pub struct RedisClusterCoordinator {
    connection: Arc<tokio::sync::Mutex<RedisCoordinationConnection>>,
    prefix: String,
    ttl_seconds: u64,
    backend_name: &'static str,
}

impl RedisClusterCoordinator {
    /// Create a new Redis cluster coordinator
    pub async fn new(redis_url: &str, prefix: Option<&str>) -> Result<Self> {
        Self::new_with_connection_options(redis_url, None, RedisTlsOptions::default(), prefix).await
    }

    /// Create a coordinator using direct TLS or a native Sentinel topology.
    pub async fn new_with_connection_options(
        redis_url: &str,
        sentinel: Option<SentinelSpec>,
        tls: RedisTlsOptions,
        prefix: Option<&str>,
    ) -> Result<Self> {
        let client = RedisClient::connect_with_options(
            redis_url,
            RedisClientOptions {
                sentinel,
                tls,
                response_timeout: None,
            },
        )
        .await?;
        let connection = client.command_connection().await?;

        Ok(Self {
            connection: Arc::new(tokio::sync::Mutex::new(
                RedisCoordinationConnection::Standard(connection),
            )),
            prefix: prefix.unwrap_or("sockudo").to_string(),
            ttl_seconds: 300,
            backend_name: "redis",
        })
    }

    /// Create a new Redis Cluster coordinator from seed nodes.
    pub async fn new_cluster(nodes: Vec<String>, prefix: Option<&str>) -> Result<Self> {
        Self::new_cluster_with_tls(nodes, RedisTlsOptions::default(), prefix).await
    }

    /// Create a Redis Cluster coordinator with data-plane TLS settings.
    pub async fn new_cluster_with_tls(
        nodes: Vec<String>,
        tls: RedisTlsOptions,
        prefix: Option<&str>,
    ) -> Result<Self> {
        let builder = ClusterClientBuilder::new(nodes)
            .retries(3)
            .read_routing_strategy(RandomReplicaStrategy);
        let client = configure_cluster_builder(builder, &tls)
            .await?
            .build()
            .map_err(|e| Error::Redis(format!("Failed to create Redis Cluster client: {}", e)))?;

        let connection = client
            .get_async_connection()
            .await
            .map_err(|e| Error::Redis(format!("Failed to connect to Redis Cluster: {}", e)))?;

        Ok(Self {
            connection: Arc::new(tokio::sync::Mutex::new(
                RedisCoordinationConnection::Cluster(connection),
            )),
            prefix: prefix.unwrap_or("sockudo").to_string(),
            ttl_seconds: 300,
            backend_name: "redis_cluster",
        })
    }

    fn get_key(&self, app_id: &str, channel: &str, conflation_key: &str) -> String {
        format!(
            "{}:delta_count:{}:{}:{}",
            self.prefix, app_id, channel, conflation_key
        )
    }
}

#[async_trait]
impl ClusterCoordinator for RedisClusterCoordinator {
    fn backend_name(&self) -> &'static str {
        self.backend_name
    }

    async fn increment_and_check(
        &self,
        app_id: &str,
        channel: &str,
        conflation_key: &str,
        interval: u32,
    ) -> Result<(bool, u32)> {
        let key = self.get_key(app_id, channel, conflation_key);
        let mut conn = self.connection.lock().await;

        let count: u32 = conn
            .incr(&key, 1)
            .await
            .map_err(|e| Error::Redis(format!("Failed to increment counter: {}", e)))?;

        // Set TTL on first increment
        if count == 1 {
            let _: () = conn
                .expire(&key, self.ttl_seconds as i64)
                .await
                .map_err(|e| Error::Redis(format!("Failed to set TTL: {}", e)))?;
        }

        let should_send_full = count >= interval;

        if should_send_full {
            debug!(
                app_id,
                channel,
                count,
                interval,
                outcome = "full",
                "cluster coordination: full message triggered"
            );

            let _: () = conn
                .set(&key, 0)
                .await
                .map_err(|e| Error::Redis(format!("Failed to reset counter: {}", e)))?;

            let _: () = conn
                .expire(&key, self.ttl_seconds as i64)
                .await
                .map_err(|e| Error::Redis(format!("Failed to refresh TTL: {}", e)))?;

            Ok((true, interval))
        } else {
            debug!(
                app_id,
                channel,
                count,
                interval,
                outcome = "delta",
                "cluster coordination: delta message"
            );
            Ok((false, count))
        }
    }

    async fn reset_counter(&self, app_id: &str, channel: &str, conflation_key: &str) -> Result<()> {
        let key = self.get_key(app_id, channel, conflation_key);
        let mut conn = self.connection.lock().await;

        let _: () = conn
            .del(&key)
            .await
            .map_err(|e| Error::Redis(format!("Failed to delete counter: {}", e)))?;

        debug!(app_id, channel, "cluster coordination: reset counter");
        Ok(())
    }

    async fn get_counter(&self, app_id: &str, channel: &str, conflation_key: &str) -> Result<u32> {
        let key = self.get_key(app_id, channel, conflation_key);
        let mut conn = self.connection.lock().await;

        let count: Option<u32> = conn
            .get(&key)
            .await
            .map_err(|e| Error::Redis(format!("Failed to get counter: {}", e)))?;

        Ok(count.unwrap_or(0))
    }
}

impl Clone for RedisClusterCoordinator {
    fn clone(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
            prefix: self.prefix.clone(),
            ttl_seconds: self.ttl_seconds,
            backend_name: self.backend_name,
        }
    }
}
