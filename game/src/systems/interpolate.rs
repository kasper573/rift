use std::collections::VecDeque;

use crate::core::math::{Direction, Pos};
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::actor::{Action, Actor};
use crate::systems::movement::Position;
use crate::systems::{REPLICATION_INTERVAL, TICK_HZ};
use bevy::prelude::*;

/// Seconds of server time covered by one replication snapshot. Every snapshot is played back over
/// exactly this span, so motion runs at the server's true speed no matter how jittery its arrival was.
const SNAPSHOT: f32 = REPLICATION_INTERVAL as f32 / TICK_HZ.0;

/// Snapshots playback aims to keep buffered ahead of the one on screen. This is the jitter margin: the
/// next snapshot can arrive up to `TARGET` snapshots late without the actor running out of path and
/// stalling. The render lags the server by roughly `TARGET + 1` snapshots in exchange.
const TARGET: usize = 1;

/// Seconds of buffered motion playback steers toward keeping ahead of the playhead. The lead swings by
/// one snapshot as a segment plays out, so the midpoint of that swing — not `TARGET * SNAPSHOT` — is
/// the speed-neutral set point.
const TARGET_LEAD: f32 = (TARGET as f32 + 0.5) * SNAPSHOT;

/// How hard playback speed is nudged per snapshot of deviation from `TARGET`. Running a touch slow
/// when the buffer is short (so a late packet still lands in time) and a touch fast when it has piled
/// up keeps the buffer — and thus the render delay — steady instead of drifting or stalling.
const STEER: f32 = 0.3;

/// Per-frame smoothing applied to the buffered lead before it steers playback speed. Snapshots arrive
/// as discrete bumps, so the raw lead sawtooths; smoothing steers off the average instead, keeping the
/// on-screen speed flat rather than wobbling once per snapshot.
const SMOOTH: f32 = 0.05;

/// Past this many queued snapshots the client has fallen so far behind (a long stall or a backgrounded
/// tab) that draining smoothly is pointless; playback resyncs straight to the newest snapshot.
const MAX_QUEUE: usize = 6;

/// A jump larger than this between consecutive snapshots is a teleport (portal, respawn) and is
/// snapped rather than interpolated.
const SNAP: Tiles = Tiles(2.0);

pub struct InterpolatePlugin;

impl Plugin for InterpolatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, interpolate);
    }
}

/// The position to render an actor at. The authoritative [`Position`] only updates every
/// [`REPLICATION_INTERVAL`] ticks, so snapshots are buffered and played back one after another at the
/// server's constant speed — linear, never eased, and kept a snapshot or two behind so a late packet
/// can't stall it. Every client-side depiction of an actor (sprite, camera, health bar, hitbox,
/// positional audio) reads this instead of [`Position`].
#[derive(Component, Clone, Copy)]
pub struct RenderPosition(pub Pos<Tiles>);

/// The facing and animation to render an actor with. The server recomputes these alongside its
/// movement every tick, so they ride the same buffered timeline as [`RenderPosition`]: the legs go
/// idle, and the heading turns, exactly as the on-screen position stops or changes course — not the
/// snapshot earlier they would if read live, which reads as the actor sliding or drifting.
#[derive(Component, Clone, Copy)]
pub struct RenderActor {
    pub dir: Direction,
    pub action: Action,
}

type Spawned<'w, 's> =
    Query<'w, 's, (Entity, &'static Position, &'static Actor), Without<RenderPosition>>;

type Moving<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, Position>,
        Ref<'static, Actor>,
        &'static mut RenderPosition,
        &'static mut RenderActor,
        &'static mut Playback,
    ),
>;

fn interpolate(time: Res<Time>, mut commands: Commands, spawned: Spawned, mut moving: Moving) {
    let dt = time.delta_secs();
    for (entity, position, actor) in &spawned {
        let frame = Frame::new(position, actor);
        commands.entity(entity).insert((
            RenderPosition(frame.pos),
            RenderActor {
                dir: frame.dir,
                action: frame.action,
            },
            Playback::new(frame),
        ));
    }
    for (position, actor, mut render, mut render_actor, mut playback) in &mut moving {
        if position.is_changed() || actor.is_changed() {
            playback.push(Frame::new(&position, &actor));
        }
        render.0 = playback.advance(dt);
        render_actor.dir = playback.to.dir;
        render_actor.action = playback.to.action;
    }
}

/// One replicated snapshot's worth of an actor: where it is and how it is posed. Position interpolates
/// across a frame; facing and action switch at the frame boundary.
#[derive(Clone, Copy, PartialEq)]
struct Frame {
    pos: Pos<Tiles>,
    dir: Direction,
    action: Action,
}

impl Frame {
    fn new(position: &Position, actor: &Actor) -> Frame {
        Frame {
            pos: position.pos,
            dir: actor.dir,
            action: actor.action,
        }
    }
}

/// A queue of snapshots awaiting playback. `from`/`to` bracket the segment currently on screen and
/// `elapsed` is how far playback has travelled into it; reaching `to` pulls the next snapshot from the
/// queue, so the actor always walks toward a real future point rather than chasing the latest one.
#[derive(Component)]
struct Playback {
    queue: VecDeque<Frame>,
    from: Pos<Tiles>,
    to: Frame,
    elapsed: f32,
    lead: f32,
}

impl Playback {
    fn new(at: Frame) -> Playback {
        Playback {
            queue: VecDeque::new(),
            from: at.pos,
            to: at,
            elapsed: 0.0,
            lead: TARGET_LEAD,
        }
    }

    fn push(&mut self, frame: Frame) {
        if frame == self.to {
            return;
        }
        if self.to.pos.distance(frame.pos) > SNAP || self.queue.len() >= MAX_QUEUE {
            self.queue.clear();
            self.from = frame.pos;
            self.to = frame;
            self.elapsed = 0.0;
        } else {
            self.queue.push_back(frame);
        }
    }

    fn advance(&mut self, dt: f32) -> Pos<Tiles> {
        let lead = (SNAPSHOT - self.elapsed) + self.queue.len() as f32 * SNAPSHOT;
        self.lead += (lead - self.lead) * SMOOTH;
        let speed = (1.0 + STEER * (self.lead - TARGET_LEAD) / SNAPSHOT).clamp(0.5, 2.0);
        self.elapsed += dt * speed;
        while self.elapsed >= SNAPSHOT && !self.queue.is_empty() {
            self.from = self.to.pos;
            self.to = self.queue.pop_front().expect("non-empty checked above");
            self.elapsed -= SNAPSHOT;
        }
        if self.queue.is_empty() {
            self.elapsed = self.elapsed.min(SNAPSHOT);
        }
        let t = self.elapsed / SNAPSHOT;
        self.from + (self.to.pos - self.from) * t
    }
}
