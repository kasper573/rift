use std::collections::HashSet;
use std::time::Duration;

use bevy::prelude::*;

use super::announcement::{AnnouncementId, Announcements};
use super::history::{self, HistoryEntry};
use super::text::LineText;
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;

pub struct AnnouncementLanePlugin;

impl Plugin for AnnouncementLanePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LaneState>()
            .add_systems(OnExit(GameScene::Area), forget)
            .add_systems(
                Update,
                follow_announcements
                    .run_if(in_state(GameScene::Area))
                    .before(ui::UiReactive),
            );
    }
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
    if let Some(shown) = announcements.showing
        && state.showing.map(|(nth, _)| nth) != Some(shown.nth)
    {
        state.showing = Some((shown.nth, now));
        if state.recorded.insert(shown.nth) {
            fresh.push((shown.id, false));
        }
    }
    for missed in &announcements.missed {
        if state.recorded.insert(missed.nth) {
            fresh.push((missed.id, true));
        }
    }
    let started = state.showing.map_or(now, |(_, at)| at);
    for &(id, missed) in &fresh {
        let def = id.get();
        history::record(
            world,
            HistoryEntry::Announced {
                who: def.by.name().to_owned(),
                text: LineText::of(def.text),
                missed,
            },
        );
        if !missed {
            world.write_message(PlaySfx {
                id: SfxId::UiChime,
                place: SfxPlace::Interface,
            });
        }
    }
    let head = announcements.showing.map(|shown| (shown.nth, shown.id));
    let next = announcements.next.map(|next| (next.nth, next.id));
    let remaining = announcements.showing.map_or(0.0, |shown| {
        1.0 - now.saturating_sub(started).as_secs_f32() / shown.shows.0.max(0.01)
    });
    let mut lanes = world.query::<&mut ui::AnnouncementLane>();
    for mut lane in lanes.iter_mut(world) {
        let keys = |lane: &ui::AnnouncementLane| {
            (
                lane.head.as_ref().map(|head| head.key),
                lane.next.as_ref().map(|next| next.key),
            )
        };
        if keys(&lane) != (head.map(key), next.map(key)) {
            lane.head = head.map(card);
            lane.next = next.map(card);
        }
        if lane.remaining != remaining {
            lane.remaining = remaining;
        }
    }
}

fn key((nth, _): (u32, AnnouncementId)) -> u64 {
    u64::from(nth)
}

fn card((nth, id): (u32, AnnouncementId)) -> ui::Announcement {
    let def = id.get();
    ui::Announcement {
        key: u64::from(nth),
        speaker: Some(def.by.name().to_owned()),
        text: LineText::of(def.text).rich(),
        narration: def.by.narrates(),
    }
}
