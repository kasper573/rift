use bevy::log::warn;
use bevy_ecs::prelude::*;

use schemars::JsonSchema;

use super::{Notification, NotificationSent};
use crate::core::time::UnixMillis;
use crate::systems::GameSettings;

/// How many notifications a player is told live: more than this is dropped from the live feed, so
/// a flood can't bury what matters.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationBudget {
    /// How many notifications a player can be told in a burst, from rest.
    pub burst: f32,
    /// How many notifications a second the player's budget earns back, up to the burst.
    pub refill_per_second: f32,
}

const LOUD_AFTER: u32 = 20;

/// Whether `sent` goes out to `player` live: not while the same line is still up, nor past a burst.
pub fn admit(world: &mut World, player: Entity, sent: &NotificationSent, now: UnixMillis) -> bool {
    let budget = world.resource::<GameSettings>().notices.budget;
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return false;
    };
    entity
        .entry::<LiveBudget>()
        .or_insert_with(|| LiveBudget::full(budget, now))
        .into_mut()
        .admit(sent, budget, now)
}

#[derive(Component)]
pub struct LiveBudget {
    tokens: f32,
    refilled: UnixMillis,
    recent_lines: Vec<RecentLine>,
    flooded: bool,
}

struct RecentLine {
    notification: Notification,
    speaker: Option<Entity>,
    up_until: UnixMillis,
    repeats: u32,
}

impl LiveBudget {
    fn full(budget: NotificationBudget, now: UnixMillis) -> LiveBudget {
        LiveBudget {
            tokens: budget.burst,
            refilled: now,
            recent_lines: Vec::new(),
            flooded: false,
        }
    }

    fn admit(
        &mut self,
        sent: &NotificationSent,
        budget: NotificationBudget,
        now: UnixMillis,
    ) -> bool {
        let (notification, speaker) = (&sent.notification, sent.actor);
        self.recent_lines
            .retain(|recent_line| now < recent_line.up_until);
        let repeated = self.recent_lines.iter_mut().find(|recent_line| {
            recent_line.notification == *notification && recent_line.speaker == speaker
        });
        if let Some(recent_line) = repeated {
            recent_line.repeats += 1;
            if recent_line.repeats == LOUD_AFTER {
                warn!("{notification:?} repeated {LOUD_AFTER} times while up");
            }
            return false;
        }
        self.refill(budget, now);
        if self.tokens < 1.0 {
            if !std::mem::replace(&mut self.flooded, true) {
                warn!(
                    "notifications flooded past a burst of {} plus {} a second",
                    budget.burst, budget.refill_per_second
                );
            }
            return false;
        }
        self.flooded = false;
        self.tokens -= 1.0;
        self.recent_lines.push(RecentLine {
            notification: notification.clone(),
            speaker,
            up_until: now.after(sent.stays),
            repeats: 0,
        });
        true
    }

    fn refill(&mut self, budget: NotificationBudget, now: UnixMillis) {
        let earned = now.since(self.refilled).0 * budget.refill_per_second;
        self.tokens = (self.tokens + earned).min(budget.burst);
        self.refilled = now;
    }
}
