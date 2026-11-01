use bevy::log::warn;
use bevy_ecs::prelude::*;

use super::{Notification, NotificationSent};
use crate::core::time::UnixMillis;

const BURST: f32 = 10.0;
const REFILL_PER_SECOND: f32 = 2.0;
const LOUD_AFTER: u32 = 20;

/// Whether `sent` goes out to `player` live: not while the same line is still up, nor past a burst.
pub fn admit(world: &mut World, player: Entity, sent: &NotificationSent, now: UnixMillis) -> bool {
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return false;
    };
    entity
        .entry::<LiveBudget>()
        .or_insert_with(|| LiveBudget::full(now))
        .into_mut()
        .admit(sent, now)
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
    fn full(now: UnixMillis) -> LiveBudget {
        LiveBudget {
            tokens: BURST,
            refilled: now,
            recent_lines: Vec::new(),
            flooded: false,
        }
    }

    fn admit(&mut self, sent: &NotificationSent, now: UnixMillis) -> bool {
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
        self.refill(now);
        if self.tokens < 1.0 {
            if !std::mem::replace(&mut self.flooded, true) {
                warn!(
                    "notifications flooded past a burst of {BURST} plus {REFILL_PER_SECOND} a second"
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

    fn refill(&mut self, now: UnixMillis) {
        let earned = now.since(self.refilled).0 * REFILL_PER_SECOND;
        self.tokens = (self.tokens + earned).min(BURST);
        self.refilled = now;
    }
}
