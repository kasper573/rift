//! The marks on the ground that show what the pointer and the player are about: a ring under what
//! the pointer would take, a ring that fades where a walk was asked for, a ring under the player
//! and one under whoever they are engaged with.

use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::decal::{DecalLook, GroundDecal};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::actor::Hitbox;
use crate::systems::combat::{self, Relation};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session;
use crate::systems::reach::Engaged;
use crate::systems::screen::ClientScreen;

pub const FRIENDLY: Color = Color::srgba(1.0, 0.85, 0.4, 0.9);
pub const HOSTILE: Color = Color::srgba(1.0, 0.3, 0.25, 0.9);
const SELF: Color = Color::srgba(0.55, 0.8, 1.0, 0.7);
const HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.75);
const WALK: Color = Color::srgba(1.0, 1.0, 1.0, 0.9);
const HOVER_SIZE: Meters = Meters(0.7);
const RING_WIDTH: f32 = 0.12;
const DASHES: u32 = 8;
/// How long the mark of an asked walk lingers.
const WALK_FADE: Seconds = Seconds(0.6);

pub fn plugin(app: &mut App) {
    app.init_resource::<HoverMark>()
        .add_systems(
            PostUpdate,
            (show_hover, fade_walks, ring_me, ring_engaged)
                .in_set(WorldViewSystems)
                .run_if(in_state(ClientScreen::Area)),
        )
        .add_systems(OnExit(ClientScreen::Area), clear);
}

/// The mark the gestures lay under what the pointer rests on this frame.
#[derive(Resource, Default)]
pub struct HoverMark(pub Option<GroundDecal>);

/// A dashed ring on the ground at `at`, as the pointer hovers over a place.
pub fn hover_ring(at: Pos<Meters>) -> GroundDecal {
    GroundDecal {
        at,
        size: HOVER_SIZE,
        yaw: default(),
        look: DecalLook::Ring {
            color: HOVER,
            width: RING_WIDTH,
            dashes: DASHES,
        },
    }
}

/// A ring under an actor as wide as its body, in `color`.
pub fn under(world: &World, actor: Entity, color: Color) -> Option<GroundDecal> {
    let at = world
        .get::<RenderPosition>(actor)
        .map(|drawn| drawn.0)
        .or_else(|| world.get::<Position>(actor).map(|position| position.pos))?;
    let radius = world
        .get::<Hitbox>(actor)
        .map_or(Meters(0.35), |hitbox| hitbox.0.radius);
    Some(GroundDecal {
        at,
        size: Meters(radius.0 * 2.0 + 0.3),
        yaw: default(),
        look: DecalLook::Ring {
            color,
            width: RING_WIDTH,
            dashes: 0,
        },
    })
}

/// Lays the mark of a walk asked for to `at`, which fades as the walk sets off.
pub fn walk_to(world: &mut World, at: Pos<Meters>) {
    let born = Seconds(world.resource::<Time>().elapsed_secs());
    world.spawn((
        WalkMark { born },
        GroundDecal {
            look: DecalLook::Ring {
                color: WALK,
                width: RING_WIDTH,
                dashes: DASHES,
            },
            ..hover_ring(at)
        },
    ));
}

#[derive(Component)]
struct Hover;

#[derive(Component)]
struct WalkMark {
    born: Seconds,
}

#[derive(Component)]
struct MeRing;

#[derive(Component)]
struct EngagedRing;

fn show_hover(mark: Res<HoverMark>, mut hover: Query<Entity, With<Hover>>, mut commands: Commands) {
    let shown = hover.single_mut().ok();
    match (&mark.0, shown) {
        (Some(decal), Some(entity)) => {
            commands.entity(entity).insert(decal.clone());
        }
        (Some(decal), None) => {
            commands.spawn((Hover, decal.clone()));
        }
        (None, Some(entity)) => {
            commands.entity(entity).despawn();
        }
        (None, None) => {}
    }
}

fn fade_walks(
    time: Res<Time>,
    mut marks: Query<(Entity, &WalkMark, &mut GroundDecal)>,
    mut commands: Commands,
) {
    let now = Seconds(time.elapsed_secs());
    for (entity, mark, mut decal) in &mut marks {
        let age = (now - mark.born).ratio(WALK_FADE);
        if age >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let DecalLook::Ring { color, .. } = &mut decal.look {
            color.set_alpha(WALK.alpha() * (1.0 - age));
        }
    }
}

fn ring_me(world: &mut World) {
    let decal = session::my_character(world)
        .map(|me| me.id())
        .and_then(|me| under(world, me, SELF));
    keep::<MeRing>(world, decal, MeRing);
}

fn ring_engaged(world: &mut World) {
    let decal = session::my_character(world)
        .map(|me| me.id())
        .and_then(|me| {
            let target = world.get::<Engaged>(me)?.0;
            let color = if combat::relation(world, me, target) == Relation::Enemy {
                HOSTILE
            } else {
                FRIENDLY
            };
            under(world, target, color)
        });
    keep::<EngagedRing>(world, decal, EngagedRing);
}

/// Keeps the one marked entity carrying `decal`, or none when there is nothing to mark.
fn keep<Tag: Component>(world: &mut World, decal: Option<GroundDecal>, tag: Tag) {
    let shown = world
        .query_filtered::<Entity, With<Tag>>()
        .single(world)
        .ok();
    match (decal, shown) {
        (Some(decal), Some(entity)) => {
            world.entity_mut(entity).insert(decal);
        }
        (Some(decal), None) => {
            world.spawn((tag, decal));
        }
        (None, Some(entity)) => {
            world.despawn(entity);
        }
        (None, None) => {}
    }
}

fn clear(world: &mut World) {
    let marks: Vec<Entity> = world
        .query_filtered::<
            Entity,
            Or<(With<Hover>, With<WalkMark>, With<MeRing>, With<EngagedRing>)>,
        >()
        .iter(world)
        .collect();
    for entity in marks {
        world.despawn(entity);
    }
    world.resource_mut::<HoverMark>().0 = None;
}
