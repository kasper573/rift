use bevy::prelude::*;
use world::core::math::Pos;
use world::core::tiling::{TilePos, Tiles};
use world::systems::actor::Actor;
use world::systems::movement::Position;
use world::systems::{REPLICATION_INTERVAL, TICK_HZ};

/// Nominal seconds between replication snapshots — the playback span for the first segment, before a
/// real inter-snapshot time has been measured.
const SNAPSHOT: f32 = REPLICATION_INTERVAL as f32 / TICK_HZ.0;

/// A jump larger than this is a teleport (portal, respawn) and is snapped rather than interpolated.
const SNAP: Tiles = Tiles(2.0);

pub struct InterpolatePlugin;

impl Plugin for InterpolatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, interpolate);
    }
}

/// The position to render an actor at. The authoritative [`Position`] only updates every
/// [`REPLICATION_INTERVAL`] ticks, so each new snapshot starts a straight segment from the current
/// rendered point to the new one, replayed at *constant* velocity over the time between snapshots —
/// no easing, so motion reads as walking rather than sliding. Every client-side depiction of an actor
/// (sprite, camera, health bar, hitbox, positional audio) reads this instead of [`Position`].
#[derive(Component, Clone, Copy)]
pub struct RenderPosition(pub Pos<Tiles>);

/// The straight segment [`RenderPosition`] is currently traversing: where it began, when, and the
/// span to cover it in (the measured gap to the previous snapshot, so playback matches the real rate).
#[derive(Component, Clone, Copy)]
struct Segment {
    from: Pos<Tiles>,
    start: f32,
    span: f32,
}

type Spawned<'w, 's> =
    Query<'w, 's, (Entity, &'static Position), (With<Actor>, Without<RenderPosition>)>;

fn interpolate(
    time: Res<Time>,
    mut commands: Commands,
    spawned: Spawned,
    mut moving: Query<(Ref<Position>, &mut RenderPosition, &mut Segment), With<Actor>>,
) {
    let now = time.elapsed_secs();
    for (entity, position) in &spawned {
        commands.entity(entity).insert((
            RenderPosition(position.pos),
            Segment {
                from: position.pos,
                start: now,
                span: SNAPSHOT,
            },
        ));
    }
    for (position, mut render, mut segment) in &mut moving {
        if position.is_changed() {
            segment.span = (now - segment.start).clamp(SNAPSHOT * 0.5, SNAPSHOT * 2.0);
            segment.start = now;
            segment.from = if render.0.distance(position.pos) > SNAP {
                position.pos
            } else {
                render.0
            };
        }
        let t = ((now - segment.start) / segment.span).clamp(0.0, 1.0);
        render.0 = segment.from + (position.pos - segment.from) * t;
    }
}
