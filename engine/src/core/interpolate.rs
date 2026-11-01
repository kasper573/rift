use crate::core::units::Seconds;
use std::collections::VecDeque;
use std::marker::PhantomData;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

/// A client-side render value smoothly played back from a replicated [`Interpolate::Source`] that
/// only updates every few ticks. Implement it on the component your renderers read (a sprite
/// position, a facing); registering an [`InterpolatePlugin`] for it then keeps it filled in on its
/// own — one snapshot stream buffered per entity, advanced every frame. The provided methods suit
/// discrete values (a facing, an animation): they switch as their segment starts and never jump.
/// Continuous values (a position) override `interpolate` to blend, and `jumps` so playback cuts
/// to where the source jumped rather than playing through the jump.
pub trait Interpolate: Component<Mutability = Mutable> + Clone + PartialEq {
    /// The replicated component this value is sampled from.
    type Source: Component;

    /// Read the render value from a freshly replicated source.
    fn sample(source: &Self::Source) -> Self;

    /// The value `t` (0..=1) of the way from `self` toward `next`.
    fn interpolate(&self, next: &Self, _t: f32) -> Self {
        next.clone()
    }

    /// How many times the source has jumped rather than moved: a teleport, a respawn.
    fn jumps(_source: &Self::Source) -> Jumps {
        Jumps::default()
    }
}

/// A count of the times a value jumped rather than moved, kept by whatever makes it jump. However
/// far apart the frames that see it, a change in the count tells a jump from a quick move.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Jumps(pub u32);

impl Jumps {
    pub fn next(self) -> Jumps {
        Jumps(self.0.wrapping_add(1))
    }
}

/// Marks an entity whose render values the client simulates itself rather than plays back from
/// snapshots. Playback leaves it alone, so whatever predicts it must keep its render values filled in.
#[derive(Component)]
pub struct Predicted;

/// The systems that fill in render values each frame, whether played back or predicted. Whatever
/// draws from render values runs after them, so a frame never shows some of them a frame old.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderValueSystems;

pub struct SnapshotPlugin {
    pub period: Seconds,
}

impl Plugin for SnapshotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SnapshotPeriod(self.period));
    }
}

/// Drives one [`Interpolate`] type: attaches playback to every entity carrying its `Source` and
/// steps it each frame. Register one per render value, e.g.
/// `InterpolatePlugin::<RenderPosition>::default()`, next to the app's [`SnapshotPlugin`].
pub struct InterpolatePlugin<T>(PhantomData<T>);

impl<T> Default for InterpolatePlugin<T> {
    fn default() -> InterpolatePlugin<T> {
        InterpolatePlugin(PhantomData)
    }
}

impl<T: Interpolate> Plugin for InterpolatePlugin<T> {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive::<T>.in_set(RenderValueSystems));
    }
}

#[derive(Resource, Clone, Copy)]
struct SnapshotPeriod(Seconds);

type Playing<T> = (
    Ref<'static, <T as Interpolate>::Source>,
    &'static mut T,
    &'static mut Playback<T>,
);

fn drive<T: Interpolate>(
    time: Res<Time>,
    period: Res<SnapshotPeriod>,
    mut commands: Commands,
    spawned: Query<(Entity, &T::Source), Without<T>>,
    mut playing: Query<Playing<T>, Without<Predicted>>,
) {
    let dt = Seconds(time.delta_secs());
    for (entity, source) in &spawned {
        let at = T::sample(source);
        commands
            .entity(entity)
            .insert((at.clone(), Playback::new(at, T::jumps(source), period.0)));
    }
    for (source, mut render, mut playback) in &mut playing {
        if source.is_changed() {
            playback.push(T::sample(&source), T::jumps(&source));
        }
        *render = playback.advance(dt, period.0);
    }
}

/// Per-entity playback of one render value's snapshot stream, smoothing over jittery arrival. Each
/// snapshot covers exactly one stream period of source time and is played back over that span, so
/// playback runs at the source's true speed no matter how unevenly snapshots arrive. `from`/`to`
/// bracket the segment currently playing and `elapsed` is how far playback has travelled into it;
/// reaching `to` pulls the next snapshot from the queue, so playback always heads toward a real
/// future sample — linear, never eased, and kept a snapshot or two behind so one late arrival can't
/// stall it. A snapshot of a source that jumped cuts straight to it.
#[derive(Component)]
struct Playback<T: Interpolate> {
    queue: VecDeque<T>,
    from: T,
    to: T,
    elapsed: Seconds,
    lead: Seconds,
    jumps: Jumps,
}

impl<T: Interpolate> Playback<T> {
    fn new(at: T, jumps: Jumps, period: Seconds) -> Playback<T> {
        Playback {
            queue: VecDeque::new(),
            from: at.clone(),
            to: at,
            elapsed: Seconds(0.0),
            lead: target_lead(period),
            jumps,
        }
    }

    fn push(&mut self, snapshot: T, jumps: Jumps) {
        let jumped = std::mem::replace(&mut self.jumps, jumps) != jumps;
        if snapshot == self.to && !jumped {
            return;
        }
        if jumped || self.queue.len() >= MAX_QUEUE {
            self.queue.clear();
            self.from = snapshot.clone();
            self.to = snapshot;
            self.elapsed = Seconds(0.0);
        } else {
            self.queue.push_back(snapshot);
        }
    }

    fn advance(&mut self, dt: Seconds, period: Seconds) -> T {
        let lead = (period - self.elapsed) + period * self.queue.len() as f32;
        let kept = 0.5_f32.powf(dt.ratio(period * SMOOTHING_SNAPSHOTS));
        self.lead += (lead - self.lead) * (1.0 - kept);
        let deviation = (self.lead - target_lead(period)).ratio(period);
        let speed = (1.0 + STEER * deviation).clamp(0.5, 2.0);
        self.elapsed += dt * speed;
        while self.elapsed >= period
            && let Some(next) = self.queue.pop_front()
        {
            self.from = std::mem::replace(&mut self.to, next);
            self.elapsed -= period;
        }
        if self.queue.is_empty() {
            self.elapsed = self.elapsed.min(period);
        }
        self.from.interpolate(&self.to, self.elapsed.ratio(period))
    }
}

/// Snapshots playback aims to keep buffered ahead of the one playing. This is the jitter margin: the
/// next snapshot can arrive up to `TARGET` snapshots late without playback running dry and stalling.
/// Playback lags the source by roughly `TARGET + 1` snapshots in exchange.
const TARGET: usize = 1;

/// How hard playback speed is nudged per snapshot of deviation from the target lead. Running a touch
/// slow when the buffer is short (so a late snapshot still lands in time) and a touch fast when it
/// has piled up keeps the buffer — and thus the playback delay — steady instead of drifting or
/// stalling.
const STEER: f32 = 0.3;

/// The half-life, in snapshots, of the smoothing applied to the buffered lead before it steers
/// playback speed. Snapshots arrive as discrete bumps, so the raw lead sawtooths; smoothing steers
/// off the average instead, keeping playback speed flat rather than wobbling once per snapshot.
const SMOOTHING_SNAPSHOTS: f32 = 2.0;

/// Past this many queued snapshots the consumer has fallen so far behind (a long stall, a
/// backgrounded tab) that draining smoothly is pointless; playback resyncs straight to the newest
/// snapshot.
const MAX_QUEUE: usize = 6;

/// Seconds of buffered playback the speed steering aims to keep ahead of the playhead. The lead
/// swings by one snapshot as a segment plays out, so the midpoint of that swing — not
/// `TARGET * period` — is the speed-neutral set point.
fn target_lead(period: Seconds) -> Seconds {
    period * (TARGET as f32 + 0.5)
}
