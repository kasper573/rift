use std::time::Duration;

use bevy_replicon::prelude::{Channel, RepliconChannels};
use renet2::{ChannelConfig, SendType};

// A byte-for-byte copy of the trait `bevy_replicon_renet2` exposes under the same name and signature.
// We carry it ourselves only because that crate is still pinned to bevy 0.18; once it publishes a
// bevy-0.19 build this trait can be deleted and the import swapped to
// `bevy_replicon_renet2::RenetChannelsExt` with no call-site changes (server and client already use
// the upstream `channels.server_configs()` / `channels.client_configs()` idiom).
pub trait RenetChannelsExt {
    fn server_configs(&self) -> Vec<ChannelConfig>;
    fn client_configs(&self) -> Vec<ChannelConfig>;
}

impl RenetChannelsExt for RepliconChannels {
    fn server_configs(&self) -> Vec<ChannelConfig> {
        configs(self.server_channels())
    }

    fn client_configs(&self) -> Vec<ChannelConfig> {
        configs(self.client_channels())
    }
}

fn configs(channels: &[Channel]) -> Vec<ChannelConfig> {
    assert!(
        channels.len() <= u8::MAX as usize,
        "channel count must fit in a u8 id"
    );
    channels
        .iter()
        .enumerate()
        .map(|(index, channel)| ChannelConfig {
            channel_id: index as u8,
            max_memory_usage_bytes: 5 * 1024 * 1024,
            send_type: match channel {
                Channel::Unreliable => SendType::Unreliable {
                    ordered_reliable_substrate: false,
                },
                Channel::Unordered => SendType::ReliableUnordered {
                    resend_time: Duration::from_millis(300),
                },
                Channel::Ordered => SendType::ReliableOrdered {
                    resend_time: Duration::from_millis(300),
                },
            },
        })
        .collect()
}
