//! Resources as a client shows them: each standing as the prop it looks like under a plate naming
//! it and counting who works it, and a bar under every worker filling once an extraction.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::face::TextRole;
use ui::{float, ink, paint, slip};

use super::{
    Left, Resource, ResourceAfloat, ResourceId, ResourceMessages, StopGathering, Working, workers,
};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{ground_height, surface_height, window_point, world_point};
use crate::core::text::{self, LineText, TextLook};
use crate::core::time;
use crate::core::units::Meters;
use crate::systems::actor::plate::PlateLabel;
use crate::systems::actor::{Aloft, Hitbox};
use crate::systems::hud;
use crate::systems::input::escape::{self, Dismissal};
use crate::systems::input::map::InputAction;
use crate::systems::messages;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const BAR: egui::Vec2 = egui::Vec2::new(56.0, 5.0);
const BAR_DROP: f32 = 10.0;
const HINT_GAP: f32 = 8.0;

pub struct ResourceRenderPlugin;

impl Plugin for ResourceRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(dress)
            .add_systems(
                Update,
                stand
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (draw_bars, draw_hint)
                    .in_set(Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// The resource the pointer picks, if it is on one.
pub fn pointed_at(world: &mut World, pointed: &crate::core::render::Pointed) -> Option<Entity> {
    crate::systems::actor::target_at(world, pointed, None, |world, entity| {
        world.get::<Resource>(entity).is_some()
    })
}

/// Ends the work the player's character has under way, if any.
pub fn working(world: &World) -> bool {
    session::me(world).is_some_and(|me| world.get::<Working>(me).is_some())
}

pub fn stop(world: &mut World) -> bool {
    let working = working(world);
    if working {
        world.write_message(StopGathering);
    }
    working
}

/// How far through the extraction under way a worker is, from none to all of it.
pub fn progress(world: &World, working: &Working) -> Option<f32> {
    let now = time::server_now(world)?;
    let whole = working.until.since(working.since).0;
    let done = now.since(working.since).0;
    Some(if whole > 0.0 {
        (done / whole).clamp(0.0, 1.0)
    } else {
        1.0
    })
}

/// The plate over every standing resource: its name and who works it.
pub fn plates(world: &mut World) -> Vec<(Entity, PlateLabel, Pos<Meters>, Hitbox)> {
    let content = world.resource::<Content>().clone();
    let standing: Vec<(Entity, ResourceId, Pos<Meters>, Hitbox)> = world
        .query_filtered::<(Entity, &Resource, &Position, &Hitbox), With<Left>>()
        .iter(world)
        .map(|(entity, resource, at, hitbox)| (entity, resource.def, at.pos, *hitbox))
        .collect();
    standing
        .into_iter()
        .map(|(entity, def, at, hitbox)| {
            let row = def.get(&content);
            let working = workers(world, entity);
            let messages = &messages::of(&content).resource;
            let label = PlateLabel::resource(messages, &row.name, working, row.max_occupants);
            (entity, label, at, hitbox)
        })
        .collect()
}

/// What a plate says under a resource's name while it is worked.
pub fn plate_line(messages: &ResourceMessages, working: u32, room: u32) -> Option<String> {
    (working > 0).then(|| messages.working_of(room, working))
}

fn dress(
    add: On<Add<Resource>>,
    resources: Query<&Resource>,
    content: Res<Content>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(resource) = resources.get(add.entity) else {
        return;
    };
    let look = resource.def.get(&content).look.get(&content);
    let scene = crate::core::assets::model_scene(&assets, look.model.path());
    commands.entity(add.entity).insert((
        WorldAssetRoot(scene),
        Transform::default(),
        Visibility::default(),
    ));
}

/// Stands every resource where it is: on the ground, or on the water's surface for one that
/// lies on water, which is where its plate and the pointer find it too.
fn stand(world: &mut World) {
    let standing: Vec<(Entity, Pos<Meters>, bool)> = world
        .query_filtered::<(Entity, &Position, Has<ResourceAfloat>), With<Resource>>()
        .iter(world)
        .map(|(entity, position, afloat)| (entity, position.pos, afloat))
        .collect();
    for (entity, at, afloat) in standing {
        let height = if afloat {
            let surface = Aloft(surface_height(world, at));
            if world.get::<Aloft>(entity) != Some(&surface) {
                world.entity_mut(entity).insert(surface);
            }
            surface.0
        } else {
            ground_height(world, at)
        };
        let at = world_point(at, height);
        if let Some(mut transform) = world.get_mut::<Transform>(entity)
            && transform.translation != at
        {
            transform.translation = at;
        }
    }
}

fn draw_bars(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let workers: Vec<(Entity, Pos<Meters>, Working)> = world
        .query::<(Entity, &Position, Option<&RenderPosition>, &Working)>()
        .iter(world)
        .map(|(entity, at, drawn, working)| {
            (entity, drawn.map_or(at.pos, |drawn| drawn.0), *working)
        })
        .collect();
    for (worker, at, working) in workers {
        let Some(share) = progress(world, &working) else {
            continue;
        };
        let Some(feet) = window_point(world, at, Meters(0.0)) else {
            continue;
        };
        gui::world_area(world, ("work bar", worker))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(feet.x, feet.y + BAR_DROP))
            .show(&gui, |ui| {
                paint::progress_line(ui, BAR, share, ink::GOLD);
            });
    }
}

fn draw_hint(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(working) = session::me(world).and_then(|me| world.get::<Working>(me).copied()) else {
        return;
    };
    let Some(name) = world
        .get::<Resource>(working.resource)
        .map(|resource| resource.def.get(world.resource::<Content>()).name.clone())
    else {
        return;
    };
    let screen = gui.content_rect();
    let middle = screen.center().x;
    let floor = hud::drawn_rects(world)
        .into_iter()
        .filter(|rect| rect.x_range().contains(middle) && rect.center().y > screen.center().y)
        .map(|rect| rect.top())
        .fold(screen.bottom(), f32::min);
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).resource;
    let hint = if escape::dismisses(world) == Some(Dismissal::Work) {
        words.working_keyed(InputAction::Dismiss, name.as_str())
    } else {
        LineText::plain(words.working_unkeyed(name.as_str()))
    }
    .styled(&content);
    let world: &World = world;
    float::anchored(
        &gui,
        egui::Area::new(egui::Id::new("working hint"))
            .order(Band::Hud.order())
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(egui::pos2(middle, floor - HINT_GAP))
            .interactable(false),
        |ui| {
            slip::show(ui, |ui| {
                let look = TextLook::new(TextRole::Small.size(), ink::INK2);
                text::show(ui, world, &hint, look, None);
            });
        },
    );
}
