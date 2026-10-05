pub mod lane;

use std::borrow::Cow;
use std::collections::VecDeque;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::data;
use crate::systems::player::Players;
use crate::systems::rule::{Outcome, RuleContext};
use crate::systems::text::{LineText, Span};

pub use crate::data::announcement::Id as AnnouncementId;

const MISSED_KEPT: usize = 4;
const READING_BASE: Seconds = Seconds(2.5);
const READING_PER_WORD: Seconds = Seconds(0.35);
const READING_MAX: Seconds = Seconds(8.0);
const SERVER_LASTS: Seconds = Seconds(60.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Announcements>()
        .init_resource::<Broadcasts>();
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Announcement {
    pub by: Announcer,
    pub text: LineText,
    pub lasts: Seconds,
    pub urgent: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Announcer {
    Npc(data::npc::Id),
    Narrator(Cow<'static, str>),
    Server,
}

impl Announcer {
    pub const fn narrator(caption: &'static str) -> Announcer {
        Announcer::Narrator(Cow::Borrowed(caption))
    }

    pub fn name(&self) -> &str {
        match self {
            Announcer::Npc(npc) => npc.get().display_name,
            Announcer::Narrator(caption) => caption,
            Announcer::Server => "Server",
        }
    }
}

impl Announcement {
    pub fn server(message: impl Into<String>) -> Announcement {
        Announcement {
            by: Announcer::Server,
            text: LineText::plain(message),
            lasts: SERVER_LASTS,
            urgent: true,
        }
    }

    pub fn shows(&self) -> Seconds {
        let words = self.text.words().split_whitespace().count();
        Seconds((READING_BASE.0 + READING_PER_WORD.0 * words as f32).min(READING_MAX.0))
    }
}

pub struct AnnouncementDef {
    pub by: Announcer,
    pub text: &'static [Span],
    pub lasts: Seconds,
    pub urgent: bool,
}

impl AnnouncementDef {
    pub fn announcement(&self) -> Announcement {
        Announcement {
            by: self.by.clone(),
            text: LineText::of(self.text),
            lasts: self.lasts,
            urgent: self.urgent,
        }
    }
}

pub struct Announce(pub AnnouncementId);

impl Outcome for Announce {
    fn apply(&self, ctx: &mut RuleContext) {
        announce(ctx.world, ctx.player, self.0.get().announcement());
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Announcements {
    pub showing: Option<ShownAnnouncement>,
    pub next: Option<QueuedAnnouncement>,
    pub missed: Vec<QueuedAnnouncement>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShownAnnouncement {
    pub nth: u32,
    pub announcement: Announcement,
    pub shows: Seconds,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QueuedAnnouncement {
    pub nth: u32,
    pub announcement: Announcement,
}

#[derive(Component, Clone, Debug, Default)]
pub struct AnnouncementQueue {
    counted: u32,
    showing: Option<(QueuedAnnouncement, UnixMillis)>,
    waiting: VecDeque<(QueuedAnnouncement, UnixMillis)>,
    missed: VecDeque<QueuedAnnouncement>,
}

#[derive(Resource, Default)]
pub struct Broadcasts(Vec<Announcement>);

pub fn announce(world: &mut World, player: Entity, announcement: Announcement) {
    let now = world.resource::<WallClock>().now;
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return;
    };
    let mut queue = entity.entry::<AnnouncementQueue>().or_default().into_mut();
    queue.counted += 1;
    let expires = now.after(announcement.lasts);
    let urgent = announcement.urgent;
    let queued = QueuedAnnouncement {
        nth: queue.counted,
        announcement,
    };
    let behind = if urgent {
        queue
            .waiting
            .iter()
            .take_while(|(waiting, _)| waiting.announcement.urgent)
            .count()
    } else {
        queue.waiting.len()
    };
    queue.waiting.insert(behind, (queued, expires));
    refresh(world, player, true);
}

pub fn broadcast(world: &mut World, announcement: Announcement) {
    world.resource_mut::<Broadcasts>().0.push(announcement);
}

pub fn relay(worlds: &mut [&mut World]) {
    let raised: Vec<Announcement> = worlds
        .iter_mut()
        .flat_map(|world| std::mem::take(&mut world.resource_mut::<Broadcasts>().0))
        .collect();
    for announcement in &raised {
        announce_everywhere(worlds, announcement);
    }
}

pub fn announce_everywhere(worlds: &mut [&mut World], announcement: &Announcement) {
    for world in worlds.iter_mut() {
        let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
        for player in players {
            announce(world, player, announcement.clone());
        }
    }
}

pub fn advance(world: &mut World, queues: &mut QueryState<(Entity, &AnnouncementQueue)>) {
    let busy: Vec<Entity> = queues
        .iter(world)
        .filter(|(_, queue)| queue.busy())
        .map(|(player, _)| player)
        .collect();
    for player in busy {
        refresh(world, player, false);
    }
}

fn refresh(world: &mut World, player: Entity, changed: bool) {
    let now = world.resource::<WallClock>().now;
    let Some(mut queue) = world.get_mut::<AnnouncementQueue>(player) else {
        return;
    };
    if queue.advance(now) || changed {
        let view = queue.view();
        world.entity_mut(player).insert(view);
    }
}

impl AnnouncementQueue {
    fn busy(&self) -> bool {
        self.showing.is_some() || !self.waiting.is_empty()
    }

    fn advance(&mut self, now: UnixMillis) -> bool {
        let mut changed = false;
        if self
            .showing
            .as_ref()
            .is_some_and(|(_, until)| *until <= now)
        {
            self.showing = None;
            changed = true;
        }
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|(_, expires)| *expires <= now);
        self.waiting = kept.into();
        for (queued, _) in expired {
            self.missed.push_back(queued);
            changed = true;
        }
        while self.missed.len() > MISSED_KEPT {
            self.missed.pop_front();
        }
        if self.showing.is_none()
            && let Some((queued, _)) = self.waiting.pop_front()
        {
            let until = now.after(queued.announcement.shows());
            self.showing = Some((queued, until));
            changed = true;
        }
        changed
    }

    fn view(&self) -> Announcements {
        Announcements {
            showing: self.showing.as_ref().map(|(queued, _)| ShownAnnouncement {
                nth: queued.nth,
                shows: queued.announcement.shows(),
                announcement: queued.announcement.clone(),
            }),
            next: self.waiting.front().map(|(queued, _)| queued.clone()),
            missed: self.missed.iter().cloned().collect(),
        }
    }
}

/// Announce a message to every player in every area.
#[bevy_terminal::command(name = "announce", access = crate::systems::account::role::is_admin)]
fn announce_command(
    world: &mut World,
    _ctx: &bevy_terminal::CommandCtx,
    message: bevy_terminal::RestOfLine,
) -> Result<String, String> {
    broadcast(world, Announcement::server(message.0));
    Ok("announced in every area".to_owned())
}
