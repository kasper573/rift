use bevy::prelude::*;
use world::core::math::Pos;
use world::core::tiling::{TilePos, Tiles};
use world::systems::actor::Actor;
use world::systems::movement::Position;

/// How fast the rendered position chases the replicated one, per second. Tuned so motion stays
/// smooth between the sub-sim-rate replication snapshots without a perceptible lag.
const CHASE_RATE: f32 = 15.0;

/// A jump larger than this is a teleport (portal, respawn) and is snapped rather than interpolated.
const SNAP: Tiles = Tiles(2.0);

pub struct InterpolatePlugin;

impl Plugin for InterpolatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, interpolate);
    }
}

/// The position to render an actor at. Each frame it chases the authoritative, replicated
/// [`Position`] rather than snapping to it, so actors move smoothly even though the server replicates
/// below the simulation rate. Every client-side depiction of an actor — sprite, camera, health bar,
/// hitbox, positional audio — reads this instead of [`Position`] so they all stay consistent; the
/// interpolation here is the only place the two are bridged.
#[derive(Component, Clone, Copy)]
pub struct RenderPosition(pub Pos<Tiles>);

fn interpolate(
    time: Res<Time>,
    mut commands: Commands,
    mut actors: Query<(Entity, &Position, Option<&mut RenderPosition>), With<Actor>>,
) {
    let alpha = 1.0 - (-time.delta_secs() * CHASE_RATE).exp();
    for (entity, position, render) in &mut actors {
        match render {
            Some(mut render) => {
                render.0 = if render.0.distance(position.pos) > SNAP {
                    position.pos
                } else {
                    render.0 + (position.pos - render.0) * alpha
                };
            }
            None => {
                commands.entity(entity).insert(RenderPosition(position.pos));
            }
        }
    }
}
