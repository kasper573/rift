use std::collections::VecDeque;
use std::marker::PhantomData;

use bevy_app::{App, Plugin, PreUpdate, Update};
use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use bevy_replicon::client::server_mutate_ticks::MutateTickReceived;
use bevy_replicon::prelude::{ClientSystems, RepliconTick};
use bevy_replicon::shared::replication::track_mutate_messages::TrackAppExt;
use bevy_time::{Real, Time};

use crate::core::time::Seconds;

/// Part of the shared protocol: the server then sends every replication tick, even one without
/// changes, so clients can tell a late tick from a quiet one.
pub fn register(app: &mut App) {
    app.track_mutate_messages();
}

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

pub struct SnapshotPlugin {
    pub period: Seconds,
}

impl Plugin for SnapshotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SnapshotClock::new(self.period))
            .add_systems(PreUpdate, time_arrivals.after(ClientSystems::Receive));
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
        app.add_systems(Update, drive::<T>);
    }
}

/// How far behind its schedule the replication stream has recently run. Playback buffers that much
/// more ahead of the playhead, so a stall like the last one is ridden out instead of frozen through
/// and then fast-forwarded. The extra delay drains away again once ticks keep arriving on time.
/// Lateness only counts in frames the client itself kept up with: a stalled client delays every
/// arrival it reads, and buffering more wouldn't help it.
#[derive(Resource)]
struct SnapshotClock {
    period: Seconds,
    last: Option<(RepliconTick, Seconds)>,
    lag: Seconds,
}

impl SnapshotClock {
    fn new(period: Seconds) -> SnapshotClock {
        SnapshotClock {
            period,
            last: None,
            lag: Seconds(0.0),
        }
    }

    fn arrived(&mut self, tick: RepliconTick, at: Seconds, frame: Seconds) {
        let client_kept_up = frame <= self.period;
        if client_kept_up
            && let Some((last, last_at)) = self.last
            && tick.get() == last.get().wrapping_add(1)
        {
            let late = at - last_at - self.period;
            if late > self.lag {
                self.lag = Seconds(late.0.min(MAX_LAG.0));
            }
        }
        self.last = Some((tick, at));
    }

    fn settle(&mut self, dt: Seconds) {
        self.lag = self.lag * 0.5f32.powf(dt.ratio(LAG_HALF_LIFE));
    }

    /// Seconds of buffered playback the speed steering aims to keep ahead of the playhead. The lead
    /// swings by one snapshot as a segment plays out, so the midpoint of that swing — not
    /// `TARGET * period` — is the speed-neutral set point.
    fn target_lead(&self) -> Seconds {
        self.period * (TARGET as f32 + 0.5) + self.lag
    }

    fn max_queue(&self) -> usize {
        self.target_lead().ratio(self.period).ceil() as usize + RESYNC_BACKLOG
    }
}

fn time_arrivals(
    mut received: MessageReader<MutateTickReceived>,
    time: Res<Time<Real>>,
    mut clock: ResMut<SnapshotClock>,
) {
    let frame = Seconds(time.delta_secs());
    clock.settle(frame);
    let now = Seconds(time.elapsed_secs());
    for tick in received.read() {
        clock.arrived(tick.tick, now, frame);
    }
}

type Playing<T> = (
    Ref<'static, <T as Interpolate>::Source>,
    &'static mut T,
    &'static mut Playback<T>,
);

fn drive<T: Interpolate>(
    time: Res<Time>,
    clock: Res<SnapshotClock>,
    mut commands: Commands,
    spawned: Query<(Entity, &T::Source), Without<T>>,
    mut playing: Query<Playing<T>>,
) {
    let dt = Seconds(time.delta_secs());
    for (entity, source) in &spawned {
        let at = T::sample(source);
        commands
            .entity(entity)
            .insert((at.clone(), Playback::new(at, &clock)));
    }
    for (source, mut render, mut playback) in &mut playing {
        if source.is_changed() {
            playback.push(T::sample(&source), &clock);
        }
        *render = playback.advance(dt, &clock);
    }
}

/// Per-entity playback of one render value's snapshot stream, smoothing over jittery arrival. Each
/// snapshot covers exactly one stream period of source time and is played back over that span, so
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
    fn new(at: T, clock: &SnapshotClock) -> Playback<T> {
        Playback {
            queue: VecDeque::new(),
            from: at.clone(),
            to: at,
            elapsed: Seconds(0.0),
            lead: clock.target_lead(),
        }
    }

    fn push(&mut self, snapshot: T, clock: &SnapshotClock) {
        if snapshot == self.to {
            return;
        }
        if self.to.discontinuous(&snapshot) || self.queue.len() >= clock.max_queue() {
            self.queue.clear();
            self.from = snapshot.clone();
            self.to = snapshot;
            self.elapsed = Seconds(0.0);
        } else {
            self.queue.push_back(snapshot);
        }
    }

    fn advance(&mut self, dt: Seconds, clock: &SnapshotClock) -> T {
        let period = clock.period;
        let lead = (period - self.elapsed) + period * self.queue.len() as f32;
        self.lead += (lead - self.lead) * SMOOTH;
        let deviation = (self.lead - clock.target_lead()).ratio(period);
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

/// Snapshots playback aims to keep buffered ahead of the one playing while the stream runs on
/// schedule. This is the jitter margin: the next snapshot can arrive up to `TARGET` snapshots late
/// without playback running dry and stalling. Playback lags the source by roughly `TARGET + 1`
/// snapshots in exchange.
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

/// Snapshots queued beyond what the target lead needs before the consumer counts as having fallen
/// so far behind (a long stall, a backgrounded tab) that draining smoothly is pointless; playback
/// then resyncs straight to the newest snapshot.
const RESYNC_BACKLOG: usize = 4;

/// The most extra playback delay a late stream may add. Hiding a longer stall would cost more in
/// sluggish response to the player's own input than the stall itself.
const MAX_LAG: Seconds = Seconds(0.5);

/// How quickly the extra delay drains once ticks arrive on time again. Slow enough to still be
/// buffered when a stall recurs a minute later, as they do on a contended host.
const LAG_HALF_LIFE: Seconds = Seconds(30.0);
