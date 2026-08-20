#![allow(unused_assignments)]
#![allow(unused_variables)]
#![allow(dead_code)]

use async_trait::async_trait;
use redis::{AsyncCommands, Client};
use sockudo_core::error::{Error, Result};
use sockudo_core::rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};
use sockudo_core::redis_client::RedisClient;
use tokio::sync::RwLock;
use tracing::warn;

/// Redis-based rate limiter implementation
pub struct RedisRateLimiter {
    /// Shared Redis provider used to establish a fresh connection for safe retry.
    client: RedisClient,
    /// Redis connection with automatic reconnection
    connection: RwLock<redis::aio::ConnectionManager>,
    /// Prefix for Redis keys
    prefix: String,
    /// Configuration for rate limiting
    config: RateLimitConfig,
}

impl RedisRateLimiter {
    /// Create a new Redis-based rate limiter
    pub async fn new(
        client: Client,
        prefix: String,
        max_requests: u32,
        window_secs: u64,
    ) -> Result<Self> {
        Self::with_config(
            client,
            prefix,
            RateLimitConfig {
                max_requests,
                window_secs,
                identifier: Some("redis".to_string()),
            },
        )
        .await
    }

    /// Create a new Redis-based rate limiter with a specific configuration
    pub async fn with_config(
        client: Client,
        prefix: String,
        config: RateLimitConfig,
    ) -> Result<Self> {
        let client = RedisClient::from_client(client).await?;
        Self::with_redis_client_config(client, prefix, config).await
    }

    /// Create a limiter from the shared standalone/Sentinel Redis provider.
    pub async fn with_redis_client(
        client: RedisClient,
        prefix: String,
        max_requests: u32,
        window_secs: u64,
    ) -> Result<Self> {
        Self::with_redis_client_config(
            client,
            prefix,
            RateLimitConfig {
                max_requests,
                window_secs,
                identifier: Some("redis".to_string()),
            },
        )
        .await
    }

    async fn with_redis_client_config(
        client: RedisClient,
        prefix: String,
        config: RateLimitConfig,
    ) -> Result<Self> {
        let connection = client.command_connection().await?;

        Ok(Self {
            client,
            connection: RwLock::new(connection),
            prefix,
            config,
        })
    }

    /// Get a key formatted with the prefix
    fn get_key(&self, key: &str) -> String {
        format!("{}:rl:{}", self.prefix, key)
    }

    /// Run sliding window rate limiting using Redis
    async fn run_sliding_window_check(
        &self,
        key: &str,
        increment: bool,
    ) -> Result<RateLimitResult> {
        let redis_key = self.get_key(key);
        let now_ms = crate::redis_window::current_time_ms();
        let window_start_ms = crate::redis_window::window_start_ms(now_ms, self.config.window_secs);
        let member = if increment {
            crate::redis_window::entry_member(now_ms)
        } else {
            String::new()
        };
        let request = crate::redis_window::SlidingWindowRequest {
            key: &redis_key,
            now_ms,
            window_start_ms,
            window_secs: self.config.window_secs,
            max_requests: self.config.max_requests,
            increment,
            member: &member,
        };

        let mut connection = { self.connection.read().await.clone() };
        let first_result = crate::redis_window::run_sliding_window(&mut connection, request).await;

        match first_result {
            Ok(result) => Ok(result),
            Err(error) if error.is_connection_dropped() => {
                warn!(
                    error = %error,
                    attempt_count = 1,
                    retryable = true,
                    "redis rate limiter command retry scheduled"
                );
                let replacement = self.client.fresh_connection_manager().await?;
                let mut retry_connection = replacement.clone();
                let result =
                    crate::redis_window::run_sliding_window(&mut retry_connection, request)
                        .await
                        .map_err(|retry_error| {
                            Error::Redis(format!(
                                "redis sliding-window command failed after reconnect: {retry_error}"
                            ))
                        })?;
                *self.connection.write().await = replacement;
                Ok(result)
            }
            Err(error) => Err(Error::Redis(format!(
                "redis sliding-window command failed: {error}"
            ))),
        }
    }
}

#[async_trait]
impl RateLimiter for RedisRateLimiter {
    async fn check(&self, key: &str) -> Result<RateLimitResult> {
        self.run_sliding_window_check(key, false).await
    }

    async fn increment(&self, key: &str) -> Result<RateLimitResult> {
        self.run_sliding_window_check(key, true).await
    }

    async fn reset(&self, key: &str) -> Result<()> {
        let redis_key = self.get_key(key);
        let mut conn = { self.connection.read().await.clone() };

        let _: () = conn
            .del(&redis_key)
            .await
            .map_err(|e| Error::Redis(format!("Failed to delete Redis key: {e}")))?;

        Ok(())
    }

    async fn get_remaining(&self, key: &str) -> Result<u32> {
        let result = self.check(key).await?;
        Ok(result.remaining)
    }
}
