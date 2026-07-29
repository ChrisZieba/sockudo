use serde::{Deserialize, Serialize};
use sockudo_protocol::ProtocolVersion;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelLimits {
    pub max_name_length: u32,
    pub cache_ttl: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EventLimits {
    pub max_channels_at_once: u32,
    pub max_name_length: u32,
    pub max_payload_in_kb: u32,
    pub max_batch_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PresenceConfig {
    pub max_members_per_channel: u32,
    pub max_member_size_in_kb: u32,
    pub update_rate_limit_per_member_per_second: u32,
    /// Protocol V1/Pusher-compatible abrupt-disconnect presence lease.
    pub ungraceful_timeout_seconds: u64,
    /// Protocol V2 abrupt-disconnect presence lease.
    pub v2_ungraceful_timeout_seconds: u64,
}

impl Default for ChannelLimits {
    fn default() -> Self {
        Self {
            max_name_length: 200,
            cache_ttl: 3600,
        }
    }
}

impl Default for EventLimits {
    fn default() -> Self {
        Self {
            max_channels_at_once: 100,
            max_name_length: 200,
            max_payload_in_kb: 100,
            max_batch_size: 10,
        }
    }
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            max_members_per_channel: 100,
            max_member_size_in_kb: 2,
            update_rate_limit_per_member_per_second: 10,
            ungraceful_timeout_seconds: 0,
            v2_ungraceful_timeout_seconds: 15,
        }
    }
}

impl PresenceConfig {
    pub fn ungraceful_timeout_for_protocol(&self, protocol_version: ProtocolVersion) -> u64 {
        match protocol_version {
            ProtocolVersion::V1 => self.ungraceful_timeout_seconds,
            ProtocolVersion::V2 => self.v2_ungraceful_timeout_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_timeout_defaults_preserve_v1_and_protect_v2_recovery() {
        let config = PresenceConfig::default();

        assert_eq!(
            config.ungraceful_timeout_for_protocol(ProtocolVersion::V1),
            0
        );
        assert_eq!(
            config.ungraceful_timeout_for_protocol(ProtocolVersion::V2),
            15
        );
    }

    #[test]
    fn presence_timeout_can_be_overridden_per_protocol() {
        let config = PresenceConfig {
            ungraceful_timeout_seconds: 4,
            v2_ungraceful_timeout_seconds: 9,
            ..PresenceConfig::default()
        };

        assert_eq!(
            config.ungraceful_timeout_for_protocol(ProtocolVersion::V1),
            4
        );
        assert_eq!(
            config.ungraceful_timeout_for_protocol(ProtocolVersion::V2),
            9
        );
    }
}
