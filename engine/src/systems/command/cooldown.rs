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
