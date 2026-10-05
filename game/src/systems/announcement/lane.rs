use std::collections::HashSet;
use std::time::Duration;

use bevy::prelude::*;

use super::{Announcement, Announcements, Announcer};
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;

pub struct AnnouncementLanePlugin;

impl Plugin for AnnouncementLanePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LaneState>()
            .add_message::<AnnouncementSeen>()
            .add_systems(OnExit(GameScene::Area), forget)
            .add_systems(
                Update,
                follow_announcements
                    .run_if(in_state(GameScene::Area))
                    .before(ui::UiReactive),
            );
    }
}

#[derive(Message, Clone, Debug)]
pub struct AnnouncementSeen {
    pub announcement: Announcement,
    pub missed: bool,
}

#[derive(Resource, Default)]
struct LaneState {
    following: Option<Entity>,
    showing: Option<(u32, Duration)>,
    recorded: HashSet<u32>,
}

fn forget(mut state: ResMut<LaneState>) {
    *state = LaneState::default();
}

fn follow_announcements(world: &mut World) {
    let seen = world.resource::<Viewpoint>().0;
    let announcements = seen
        .and_then(|seen| world.get::<Announcements>(seen))
        .cloned()
        .unwrap_or_default();
    let now = world.resource::<Time>().elapsed();
    let mut fresh = Vec::new();
    let mut state = world.resource_mut::<LaneState>();
    if state.following != seen {
        *state = LaneState {
            following: seen,
            ..LaneState::default()
        };
    }
    if let Some(shown) = &announcements.showing
        && state.showing.map(|(nth, _)| nth) != Some(shown.nth)
    {
        state.showing = Some((shown.nth, now));
        if state.recorded.insert(shown.nth) {
            fresh.push(AnnouncementSeen {
                announcement: shown.announcement.clone(),
                missed: false,
            });
        }
    }
    for missed in &announcements.missed {
        if state.recorded.insert(missed.nth) {
            fresh.push(AnnouncementSeen {
                announcement: missed.announcement.clone(),
                missed: true,
            });
        }
    }
    let started = state.showing.map_or(now, |(_, at)| at);
    for seen in fresh {
        if !seen.missed {
            world.write_message(PlaySfx {
                id: SfxId::UiChime,
                place: SfxPlace::Interface,
            });
        }
        world.write_message(seen);
    }
    let head = announcements
        .showing
        .as_ref()
        .map(|shown| card(shown.nth, &shown.announcement));
    let next = announcements
        .next
        .as_ref()
        .map(|next| card(next.nth, &next.announcement));
    let remaining = announcements.showing.as_ref().map_or(0.0, |shown| {
        1.0 - now.saturating_sub(started).as_secs_f32() / shown.shows.0.max(0.01)
    });
    let mut lanes = world.query::<&mut ui::AnnouncementLane>();
    for mut lane in lanes.iter_mut(world) {
        let keys = |head: Option<&ui::Announcement>, next: Option<&ui::Announcement>| {
            (head.map(|head| head.key), next.map(|next| next.key))
        };
        if keys(lane.head.as_ref(), lane.next.as_ref()) != keys(head.as_ref(), next.as_ref()) {
            lane.head = head.clone();
            lane.next = next.clone();
        }
        if lane.remaining != remaining {
            lane.remaining = remaining;
        }
    }
}

fn card(nth: u32, announcement: &Announcement) -> ui::Announcement {
    let (speaker, kind) = match &announcement.by {
        Announcer::Npc(_) => (
            Some(announcement.by.name().to_owned()),
            ui::AnnouncementKind::Speech,
        ),
        Announcer::Narrator(_) => (
            Some(announcement.by.name().to_owned()),
            ui::AnnouncementKind::Narration,
        ),
        Announcer::Server => (None, ui::AnnouncementKind::System),
    };
    ui::Announcement {
        key: u64::from(nth),
        speaker,
        text: announcement.text.rich(),
        kind,
    }
}
