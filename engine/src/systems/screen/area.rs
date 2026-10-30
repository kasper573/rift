use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::dusk;
use ui::face::TextRole;
use ui::ink::Ground;

use super::ClientScreen;
use crate::core::audio::soundscape::Soundscape;
use crate::core::content::Content;
use crate::core::geometry::Radians;
use crate::core::gui::{self, Band};
use crate::core::interpolate::RenderValueSystems;
use crate::core::math::Pos;
use crate::core::platform::ClientPlatform;
use crate::core::render::transition::{
    self, ActiveTransition, ScreenTransitionPhase, WorldViewSystems,
};
use crate::core::render::world::readying;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::actor::Actor;
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::input::map::{self, InputAction};
use crate::systems::movement::RenderPosition;
use crate::systems::player::session::{self, MyCharacter};
use crate::systems::player::{ARRIVAL_PATIENCE, AreaDrawn};

pub struct AreaScreenPlugin;

impl Plugin for AreaScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Crossing>()
            .add_systems(
                Update,
                (
                    carry_view_across
                        .after(RenderValueSystems)
                        .before(WorldViewSystems),
                    play_area_soundscape.in_set(WorldViewSystems),
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                death_notice
                    .in_set(Band::Notices)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnEnter(ClientScreen::Area), restart_patience)
            .add_systems(OnExit(ClientScreen::Area), (silence_soundscape, leave_area));
    }
}

#[derive(Resource, Default, Clone, Copy)]
struct Crossing {
    last_seen: Option<Pose>,
    held: Option<Held>,
}

#[derive(Clone, Copy)]
struct Pose {
    character: Entity,
    at: Pos<Meters>,
    heading: Radians,
    area: AreaId,
}

#[derive(Clone, Copy)]
struct Held {
    since: Seconds,
    pose: Pose,
    /// The first frame that draws a newly shown area is a long one. The view is revealed only
    /// after it, so that frame passes under the full cover.
    rendered: bool,
}

/// The view is revealed when no crossing is held under the veil and no transition plays over it.
pub fn settled(world: &World) -> bool {
    world.resource::<Crossing>().held.is_none()
        && world.resource::<ActiveTransition>().phase() == ScreenTransitionPhase::Idle
}

pub fn carry_view_across(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Crossing { last_seen, held } = *world.resource::<Crossing>();
    let (last_seen, held) = match session::me(world) {
        Some(character) => match (pose_of(world, character), held, last_seen) {
            (None, _, _) => (last_seen, held),
            (Some(pose), None, Some(seen)) if seen.area != pose.area => {
                transition::freeze(world, seen.at, seen.heading);
                (Some(pose), Some(Held::new(now, seen)))
            }
            (Some(pose), None, Some(seen)) if seen.character == pose.character => {
                (Some(pose), None)
            }
            (Some(pose), None, _) => (Some(pose), Some(Held::new(now, pose))),
            (Some(pose), Some(held), _) => {
                let drawable = area::render::shown(world) == Some(pose.area) && !readying(world);
                if now - held.since > ARRIVAL_PATIENCE || (drawable && held.rendered) {
                    reveal(world, pose);
                    (Some(pose), None)
                } else {
                    (
                        Some(pose),
                        Some(Held {
                            rendered: drawable,
                            ..held
                        }),
                    )
                }
            }
        },
        None => match (held, last_seen) {
            (Some(held), _) if now - held.since > ARRIVAL_PATIENCE => {
                transition::reveal(world, held.pose.at);
                (None, None)
            }
            (None, Some(pose)) if session::my_id(world).is_some() => {
                transition::freeze(world, pose.at, pose.heading);
                (None, Some(Held::new(now, pose)))
            }
            _ => (None, held),
        },
    };
    *world.resource_mut::<Crossing>() = Crossing { last_seen, held };
}

/// Shows the area the character stands in, and tells whoever follows the client's status, a
/// screen reader among them, where the player has arrived.
fn reveal(world: &mut World, pose: Pose) {
    transition::reveal(world, pose.at);
    world.write_message(AreaDrawn);
    let content = world.resource::<Content>().clone();
    world
        .resource::<ClientPlatform>()
        .0
        .announce(&pose.area.get(&content).name);
}

fn restart_patience(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    if let Some(held) = world.resource_mut::<Crossing>().held.as_mut() {
        held.since = now;
    }
}

impl Held {
    fn new(since: Seconds, pose: Pose) -> Held {
        Held {
            since,
            pose,
            rendered: false,
        }
    }
}

fn pose_of(world: &World, character: Entity) -> Option<Pose> {
    let entity = world.get_entity(character).ok()?;
    Some(Pose {
        character,
        at: entity.get::<RenderPosition>()?.0,
        heading: entity.get::<Actor>()?.facing,
        area: entity.get::<AreaTag>()?.area,
    })
}

/// Leaving the area scene keeps the last view under the veil, so a lost connection or a content
/// deploy shows the loading and reconnect cards over a frozen world and reveals the next one from
/// where the character lands.
fn leave_area(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Crossing { last_seen, held } = *world.resource::<Crossing>();
    let held = match (last_seen, held) {
        (Some(pose), None) => {
            transition::freeze(world, pose.at, pose.heading);
            Some(Held::new(now, pose))
        }
        (Some(_), Some(held)) => Some(held),
        (None, _) => {
            transition::abort(world);
            None
        }
    };
    *world.resource_mut::<Crossing>() = Crossing {
        last_seen: None,
        held,
    };
}

fn play_area_soundscape(
    viewpoint: Res<MyCharacter>,
    areas: Query<&AreaTag>,
    content: Res<Content>,
    mut soundscape: ResMut<Soundscape>,
) {
    let Some(area_id) = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .map(|tag| tag.area)
    else {
        return;
    };
    let zones = &area::load(&content, area_id).soundscape;
    if soundscape.0 != *zones {
        soundscape.0.clone_from(zones);
    }
}

fn silence_soundscape(mut soundscape: ResMut<Soundscape>) {
    soundscape.0.clear();
}

fn death_notice(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    if !session::is_dead(world) {
        return;
    }
    let line = StyledLine::new(vec![
        TextPiece::text("You died! Press "),
        map::input(InputAction::Respawn),
        TextPiece::text(" to respawn"),
    ]);
    let area = egui::Area::new(egui::Id::new("death"))
        .order(Band::Notices.order())
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .interactable(false);
    float::anchored(&gui, area, |ui| {
        dusk::frame().show(ui, |ui| {
            let look = TextLook::new(TextRole::Body.size(), dusk::TEXT).on(Ground::Dusk);
            text::show(ui, world, &line, look, None);
        });
    });
}
