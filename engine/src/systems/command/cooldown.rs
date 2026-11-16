//! How long after a command runs before the same character may send it again.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::TerminalCommandId;
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;

/// When each command a character sent with a cooldown may be sent again.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CommandCooldowns(pub BTreeMap<TerminalCommandId, UnixMillis>);

impl CommandCooldowns {
    /// How long until `command` may be sent again: nothing once it may.
    pub fn left(&self, command: TerminalCommandId, now: UnixMillis) -> Option<Seconds> {
        self.0
            .get(&command)
            .filter(|&&ready| ready > now)
            .map(|ready| ready.since(now))
    }

    /// Starts the cooldown of `command`, sent at `now`, forgetting those that ran out.
    pub fn start(&mut self, command: TerminalCommandId, now: UnixMillis, cooldown: Seconds) {
        self.0.retain(|_, ready| *ready > now);
        self.0.insert(command, now.after(cooldown));
    }
}

/// `span` as a player reads a wait: to the minute above a minute, to the second below it, and
/// never none while something still waits.
pub fn worded(span: Seconds) -> String {
    const MINUTE: f32 = 60.0;
    if span.0 < MINUTE {
        return format!("{}s", span.0.ceil().max(1.0) as u32);
    }
    let minutes = (span.0 / MINUTE).ceil() as u32;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}
