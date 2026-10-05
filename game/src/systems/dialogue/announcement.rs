use std::collections::VecDeque;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::text::Span;
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::data;
use crate::systems::rule::{Outcome, RuleContext};

pub use crate::data::announcement::Id as AnnouncementId;

const MISSED_KEPT: usize = 4;
const READING_BASE: Seconds = Seconds(2.5);
const READING_PER_WORD: Seconds = Seconds(0.35);
const READING_MAX: Seconds = Seconds(8.0);

pub struct AnnouncementDef {
    pub by: Announcer,
    pub text: &'static [Span],
    pub lasts: Seconds,
    pub urgent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcer {
    Npc(data::npc::Id),
    Narrator(&'static str),
}

impl Announcer {
    pub fn name(self) -> &'static str {
        match self {
            Announcer::Npc(npc) => npc.get().display_name,
            Announcer::Narrator(caption) => caption,
        }
    }

    pub fn narrates(self) -> bool {
        matches!(self, Announcer::Narrator(_))
    }
}

impl AnnouncementDef {
    pub fn shows(&self) -> Seconds {
        let words = self
            .text
            .iter()
            .map(|span| span.text.split_whitespace().count())
            .sum::<usize>();
        Seconds((READING_BASE.0 + READING_PER_WORD.0 * words as f32).min(READING_MAX.0))
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Announcements {
    pub showing: Option<ShownAnnouncement>,
    pub next: Option<QueuedAnnouncement>,
    pub missed: Vec<QueuedAnnouncement>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ShownAnnouncement {
    pub nth: u32,
    pub id: AnnouncementId,
    pub shows: Seconds,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct QueuedAnnouncement {
    pub nth: u32,
    pub id: AnnouncementId,
}

#[derive(Component, Clone, Debug, Default)]
pub struct AnnouncementQueue {
    counted: u32,
    showing: Option<(QueuedAnnouncement, UnixMillis)>,
    waiting: VecDeque<(QueuedAnnouncement, UnixMillis)>,
    missed: VecDeque<QueuedAnnouncement>,
}

pub struct Announce(pub AnnouncementId);

impl Outcome for Announce {
    fn apply(&self, ctx: &mut RuleContext) {
        announce(ctx.world, ctx.player, self.0);
    }
}

pub fn announce(world: &mut World, player: Entity, id: AnnouncementId) {
    let now = world.resource::<WallClock>().now;
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return;
    };
    let mut queue = entity.entry::<AnnouncementQueue>().or_default().into_mut();
    queue.counted += 1;
    let queued = (
        QueuedAnnouncement {
            nth: queue.counted,
            id,
        },
        now.after(id.get().lasts),
    );
    let behind = if id.get().urgent {
        queue
            .waiting
            .iter()
            .take_while(|(waiting, _)| waiting.id.get().urgent)
            .count()
    } else {
        queue.waiting.len()
    };
    queue.waiting.insert(behind, queued);
    refresh(world, player);
}

pub fn advance(world: &mut World, queues: &mut QueryState<Entity, With<AnnouncementQueue>>) {
    let players: Vec<Entity> = queues.iter(world).collect();
    for player in players {
        refresh(world, player);
    }
}

fn refresh(world: &mut World, player: Entity) {
    let now = world.resource::<WallClock>().now;
    let Some(mut queue) = world.get_mut::<AnnouncementQueue>(player) else {
        return;
    };
    queue.advance(now);
    let view = queue.view();
    if world.get::<Announcements>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

impl AnnouncementQueue {
    fn advance(&mut self, now: UnixMillis) {
        if self.showing.is_some_and(|(_, until)| until <= now) {
            self.showing = None;
        }
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|&(_, expires)| expires <= now);
        self.waiting = kept.into();
        for (queued, _) in expired {
            self.missed.push_back(queued);
        }
        while self.missed.len() > MISSED_KEPT {
            self.missed.pop_front();
        }
        if self.showing.is_none()
            && let Some((queued, _)) = self.waiting.pop_front()
        {
            self.showing = Some((queued, now.after(queued.id.get().shows())));
        }
    }

    fn view(&self) -> Announcements {
        Announcements {
            showing: self.showing.map(|(queued, _)| ShownAnnouncement {
                nth: queued.nth,
                id: queued.id,
                shows: queued.id.get().shows(),
            }),
            next: self.waiting.front().map(|&(queued, _)| queued),
            missed: self.missed.iter().copied().collect(),
        }
    }
}
