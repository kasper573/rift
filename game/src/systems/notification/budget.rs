use bevy::log::warn;
use bevy_ecs::prelude::*;

use super::Notification;
use crate::core::time::{Seconds, UnixMillis};

const REPEAT_WINDOW: Seconds = Seconds(10.0);
const BURST: f32 = 10.0;
const REFILL_PER_SECOND: f32 = 2.0;
const LOUD_AFTER: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    Live,
    Repeat,
    Flooded,
}

pub fn admit(
    world: &mut World,
    player: Entity,
    notification: &Notification,
    speaker: Option<Entity>,
    now: UnixMillis,
) -> Admit {
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return Admit::Flooded;
    };
    entity
        .entry::<LiveBudget>()
        .or_insert_with(|| LiveBudget::full(now))
        .into_mut()
        .admit(notification, speaker, now)
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
    sent_at: UnixMillis,
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

    fn admit(
        &mut self,
        notification: &Notification,
        speaker: Option<Entity>,
        now: UnixMillis,
    ) -> Admit {
        self.recent_lines
            .retain(|recent_line| now.since(recent_line.sent_at) < REPEAT_WINDOW);
        let repeated = self.recent_lines.iter_mut().find(|recent_line| {
            recent_line.notification == *notification && recent_line.speaker == speaker
        });
        if let Some(recent_line) = repeated {
            recent_line.repeats += 1;
            if recent_line.repeats == LOUD_AFTER {
                warn!(
                    "{notification:?} repeated {LOUD_AFTER} times within {}s",
                    REPEAT_WINDOW.0
                );
            }
            return Admit::Repeat;
        }
        self.refill(now);
        if self.tokens < 1.0 {
            if !std::mem::replace(&mut self.flooded, true) {
                warn!(
                    "notifications flooded past a burst of {BURST} plus {REFILL_PER_SECOND} a second"
                );
            }
            return Admit::Flooded;
        }
        self.flooded = false;
        self.tokens -= 1.0;
        self.recent_lines.push(RecentLine {
            notification: notification.clone(),
            speaker,
            sent_at: now,
            repeats: 0,
        });
        Admit::Live
    }

    fn refill(&mut self, now: UnixMillis) {
        let earned = now.since(self.refilled).0 * REFILL_PER_SECOND;
        self.tokens = (self.tokens + earned).min(BURST);
        self.refilled = now;
    }
}
