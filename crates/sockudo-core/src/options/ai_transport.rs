use super::*;
use serde::{Deserialize, Serialize};
use sockudo_protocol::messages::{
    AI_HEADER_STEER_CODEC_MESSAGE_IDS_MAX_BYTES, AI_TRANSPORT_VALUE_MAX_BYTES, AiHeaderLimits,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiTransportConfig {
    pub enabled: bool,
    pub channels: Vec<AiTransportChannelConfig>,
    pub max_accumulated_message_bytes: usize,
    pub max_appends_per_message: usize,
    pub max_open_streaming_messages_per_channel: usize,
    /// Ceiling for a single `extras.ai.transport` value, in bytes.
    pub max_transport_value_bytes: usize,
    /// Ceiling for `steer-codec-message-ids`, in bytes.
    ///
    /// Separate from the scalar budget because the value is a JSON array whose
    /// length grows with the number of steers an agent drained in one step
    /// attempt. Raising this only helps if the publishing SDK's own per-stamp id
    /// cap is raised with it — they are two halves of the same limit.
    pub max_steer_codec_message_ids_bytes: usize,
    pub rollup: AiTransportRollupConfig,
}

impl AiTransportConfig {
    /// Ceilings for AI header validation, as configured.
    #[inline]
    pub fn header_limits(&self) -> AiHeaderLimits {
        AiHeaderLimits {
            transport_value_max_bytes: self.max_transport_value_bytes,
            steer_codec_message_ids_max_bytes: self.max_steer_codec_message_ids_bytes,
        }
    }

    #[inline]
    pub fn matches_channel(&self, channel: &str) -> bool {
        self.enabled
            && self
                .channels
                .iter()
                .any(|entry| entry.matches_channel(channel))
    }

    pub(super) fn validate_deployment_matrix(
        &self,
        adapter: &AdapterConfig,
        cache: &CacheConfig,
        history: &HistoryConfig,
        versioned_messages: &VersionedMessagesConfig,
    ) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        // Only reject values *below* the protocol floor: raising a ceiling is
        // permissive, lowering it silently rejects traffic that used to pass.
        if self.max_transport_value_bytes < AI_TRANSPORT_VALUE_MAX_BYTES {
            return Err(format!(
                "ai_transport.max_transport_value_bytes must be at least {AI_TRANSPORT_VALUE_MAX_BYTES}"
            ));
        }
        if self.max_steer_codec_message_ids_bytes < AI_HEADER_STEER_CODEC_MESSAGE_IDS_MAX_BYTES {
            return Err(format!(
                "ai_transport.max_steer_codec_message_ids_bytes must be at least {AI_HEADER_STEER_CODEC_MESSAGE_IDS_MAX_BYTES}"
            ));
        }

        if !history.enabled {
            return Err("ai_transport.enabled requires history.enabled".to_string());
        }
        if !versioned_messages.enabled {
            return Err("ai_transport.enabled requires versioned_messages.enabled".to_string());
        }

        if adapter.driver != AdapterDriver::Local {
            if history.backend == HistoryBackend::Memory {
                return Err(
                    "ai_transport horizontal deployments require a shared history backend; memory history is local-only".to_string(),
                );
            }
            if versioned_messages.driver == VersionStoreDriver::Memory {
                return Err(
                    "ai_transport horizontal deployments require a shared version_store driver; memory version store is local-only".to_string(),
                );
            }
            if matches!(cache.driver, CacheDriver::Memory | CacheDriver::None) {
                return Err(
                    "ai_transport horizontal deployments require a shared cache driver for orphan ownership; memory/none cache is local-only".to_string(),
                );
            }
        }

        Ok(())
    }
}

impl Default for AiTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channels: Vec::new(),
            max_accumulated_message_bytes: 1024 * 1024,
            max_appends_per_message: 4096,
            max_open_streaming_messages_per_channel: 1024,
            // Reproduce the protocol ceilings exactly: a default that lowered
            // either one would start rejecting traffic that previously passed.
            max_transport_value_bytes: AI_TRANSPORT_VALUE_MAX_BYTES,
            max_steer_codec_message_ids_bytes: AI_HEADER_STEER_CODEC_MESSAGE_IDS_MAX_BYTES,
            rollup: AiTransportRollupConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiTransportRollupConfig {
    pub enabled: bool,
    pub default_window_ms: u64,
    pub min_window_ms: u64,
    pub max_window_ms: u64,
    pub orphan_ttl_ms: u64,
    pub wheel_tick_ms: u64,
    pub shards: usize,
}

impl Default for AiTransportRollupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_window_ms: 40,
            min_window_ms: 0,
            max_window_ms: 500,
            orphan_ttl_ms: 60_000,
            wheel_tick_ms: 5,
            shards: 64,
        }
    }
}

impl AiTransportRollupConfig {
    #[inline]
    pub fn allows_window(&self, window_ms: u64) -> bool {
        matches!(window_ms, 0 | 20 | 40 | 100 | 500)
            && window_ms >= self.min_window_ms
            && window_ms <= self.max_window_ms
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct AiTransportChannelConfig {
    pub prefix: String,
}

impl AiTransportChannelConfig {
    #[inline]
    pub fn matches_channel(&self, channel: &str) -> bool {
        !self.prefix.is_empty() && channel.starts_with(&self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::{AdapterDriver, CacheDriver, HistoryBackend, ServerOptions, VersionStoreDriver};

    fn ai_transport_options() -> ServerOptions {
        let mut options = ServerOptions::default();
        options.ai_transport.enabled = true;
        options
            .ai_transport
            .channels
            .push(super::AiTransportChannelConfig {
                prefix: "private-ai-".to_string(),
            });
        options.history.enabled = true;
        options.versioned_messages.enabled = true;
        options
    }

    #[test]
    fn ai_transport_allows_single_node_memory_development_matrix() {
        let mut options = ai_transport_options();
        options.mode = "development".to_owned();
        options.adapter.driver = AdapterDriver::Local;
        options.history.backend = HistoryBackend::Memory;
        options.versioned_messages.driver = VersionStoreDriver::Memory;

        assert!(options.validate().is_ok());
    }

    #[test]
    fn ai_transport_rejects_horizontal_memory_state_matrix() {
        let mut options = ai_transport_options();
        options.adapter.driver = AdapterDriver::Redis;
        options.history.backend = HistoryBackend::Memory;
        options.versioned_messages.driver = VersionStoreDriver::Postgres;
        options.cache.driver = CacheDriver::Redis;

        let error = options.validate().unwrap_err();
        assert!(
            error.contains("shared history backend"),
            "unexpected error: {error}"
        );

        options.history.backend = HistoryBackend::Postgres;
        options.versioned_messages.driver = VersionStoreDriver::Memory;

        let error = options.validate().unwrap_err();
        assert!(
            error.contains("shared version_store driver"),
            "unexpected error: {error}"
        );

        options.versioned_messages.driver = VersionStoreDriver::Postgres;
        options.cache.driver = CacheDriver::Memory;

        let error = options.validate().unwrap_err();
        assert!(
            error.contains("shared cache driver"),
            "unexpected error: {error}"
        );
    }
}
