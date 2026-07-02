use std::collections::VecDeque;
use std::marker::PhantomData;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::time::Seconds;

/// A client-side render value smoothly played back from a replicated [`Interpolate::Source`] that
/// only updates every few ticks. Implement it on the component your renderers read (a sprite
/// position, a facing); registering an [`InterpolatePlugin`] for it then keeps it filled in on its
/// own — one snapshot stream buffered per entity, advanced every frame. The provided methods suit
/// discrete values (a facing, an animation): they switch as their segment starts and never snap.
/// Continuous values (a position) override `interpolate` to blend and `discontinuous` to snap over
/// jumps too large to play through.
pub trait Interpolate: Component<Mutability = Mutable> + Clone + PartialEq {
    /// The replicated component this value is sampled from.
    type Source: Component;

    /// Source time covered by one snapshot — the span each is played back over. Match it to the
    /// cadence `Source` is replicated at.
    const INTERVAL: Seconds;

    /// Read the render value from a freshly replicated source.
    fn sample(source: &Self::Source) -> Self;

    /// The value `t` (0..=1) of the way from `self` toward `next`.
    fn interpolate(&self, next: &Self, _t: f32) -> Self {
        next.clone()
    }

    /// Whether `self` to `next` is a discontinuity (a teleport, a respawn) that playback must snap
    /// over instead of interpolating through.
    fn discontinuous(&self, _next: &Self) -> bool {
        false
    }
}

/// Drives one [`Interpolate`] type: attaches playback to every entity carrying its `Source` and
/// steps it each frame. Register one per render value, e.g.
/// `InterpolatePlugin::<RenderPosition>::default()`.
pub struct InterpolatePlugin<T>(PhantomData<T>);

impl<T> Default for InterpolatePlugin<T> {
    fn default() -> InterpolatePlugin<T> {
        InterpolatePlugin(PhantomData)
    }
}

impl<T: Interpolate> Plugin for InterpolatePlugin<T> {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive::<T>);
    }
}

type Playing<T> = (
    Ref<'static, <T as Interpolate>::Source>,
    &'static mut T,
    &'static mut Playback<T>,
);

fn drive<T: Interpolate>(
    time: Res<Time>,
    mut commands: Commands,
    spawned: Query<(Entity, &T::Source), Without<T>>,
    mut playing: Query<Playing<T>>,
) {
    let dt = Seconds(time.delta_secs());
    for (entity, source) in &spawned {
        let at = T::sample(source);
        commands
            .entity(entity)
            .insert((at.clone(), Playback::new(at)));
    }
    for (source, mut render, mut playback) in &mut playing {
        if source.is_changed() {
            playback.push(T::sample(&source));
        }
        *render = playback.advance(dt);
    }
}

/// Per-entity playback of one render value's snapshot stream, smoothing over jittery arrival. Each
/// snapshot covers exactly `T::INTERVAL` of source time and is played back over that span, so
/// playback runs at the source's true speed no matter how unevenly snapshots arrive. `from`/`to`
/// bracket the segment currently playing and `elapsed` is how far playback has travelled into it;
/// reaching `to` pulls the next snapshot from the queue, so playback always heads toward a real
/// future sample — linear, never eased, and kept a snapshot or two behind so one late arrival can't
/// stall it.
#[derive(Component)]
struct Playback<T: Interpolate> {
    queue: VecDeque<T>,
    from: T,
    to: T,
    elapsed: Seconds,
    lead: Seconds,
}

impl<T: Interpolate> Playback<T> {
    fn new(at: T) -> Playback<T> {
        Playback {
            queue: VecDeque::new(),
            from: at.clone(),
            to: at,
            elapsed: Seconds(0.0),
            lead: target_lead(T::INTERVAL),
        }
    }

    fn push(&mut self, snapshot: T) {
        if snapshot == self.to {
            return;
        }
        if self.to.discontinuous(&snapshot) || self.queue.len() >= MAX_QUEUE {
            self.queue.clear();
            self.from = snapshot.clone();
            self.to = snapshot;
            self.elapsed = Seconds(0.0);
        } else {
            self.queue.push_back(snapshot);
        }
    }

    fn advance(&mut self, dt: Seconds) -> T {
        let lead = (T::INTERVAL - self.elapsed) + T::INTERVAL * self.queue.len() as f32;
        self.lead += (lead - self.lead) * SMOOTH;
        let deviation = (self.lead - target_lead(T::INTERVAL)).ratio(T::INTERVAL);
        let speed = (1.0 + STEER * deviation).clamp(0.5, 2.0);
        self.elapsed += dt * speed;
        while self.elapsed >= T::INTERVAL
            && let Some(next) = self.queue.pop_front()
        {
            self.from = std::mem::replace(&mut self.to, next);
            self.elapsed -= T::INTERVAL;
        }
        if self.queue.is_empty() {
            self.elapsed = self.elapsed.min(T::INTERVAL);
        }
        self.from
            .interpolate(&self.to, self.elapsed.ratio(T::INTERVAL))
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

/// Per-advance smoothing applied to the buffered lead before it steers playback speed. Snapshots
/// arrive as discrete bumps, so the raw lead sawtooths; smoothing steers off the average instead,
/// keeping playback speed flat rather than wobbling once per snapshot.
const SMOOTH: f32 = 0.05;

/// Past this many queued snapshots the consumer has fallen so far behind (a long stall, a
/// backgrounded tab) that draining smoothly is pointless; playback resyncs straight to the newest
/// snapshot.
const MAX_QUEUE: usize = 6;

/// Seconds of buffered playback the speed steering aims to keep ahead of the playhead. The lead
/// swings by one snapshot as a segment plays out, so the midpoint of that swing — not
/// `TARGET * interval` — is the speed-neutral set point.
fn target_lead(interval: Seconds) -> Seconds {
    interval * (TARGET as f32 + 0.5)
}
