use bevy_app::Main;
use bevy_ecs::message::{Message, MessageCursor, MessageRegistry, Messages, message_update_system};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoScheduleConfigs, SingleThreadedExecutor, SystemSet};
use bevy_ecs::system::ScheduleSystem;
use bevy_replicon::prelude::ToClients;
use bevy_time::Time;
use engine::core::content::{Content, ContentFile, ContentId};
use engine::core::math::Rng;
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::systems::notification::NotificationSent;
use engine::systems::{TICK_HZ, area_state, plug_ins};

use crate::notices::{self, Notified};

/// A bare world holding a game's content, its clocks, its dice, and what an area's server keeps
/// besides, stepped tick by tick through only the systems a test adds, in the order it adds them,
/// as the server's main schedule, so the server's own stepping of worlds steps it too. Messages
/// last two ticks, as the server's do.
pub struct Sim {
    pub world: World,
    steps: usize,
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Step(usize);

impl Sim {
    pub fn new(content: Content) -> Sim {
        let mut world = World::new();
        world.insert_resource(Rng::new(drawn_seed()));
        world.insert_resource(Time::<()>::default());
        world.insert_resource(WallClock {
            now: UnixMillis(0),
            reset: UtcHour::MIDNIGHT,
        });
        plug_ins(&mut world);
        area_state(&mut world, &content);
        MessageRegistry::register_message::<ToClients<NotificationSent>>(&mut world);
        world.init_resource::<Notified>();
        let mut schedule = Schedule::new(Main);
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.add_systems(
            (advance_time, notices::collect, message_update_system)
                .chain()
                .in_set(Step(0)),
        );
        world.add_schedule(schedule);
        Sim { world, steps: 0 }
    }

    /// The file of `Row`'s type the test declared as `name`.
    pub fn id<Row: ContentFile>(&self, name: &str) -> ContentId<Row> {
        let path = crate::path(Row::TABLE, name);
        self.world
            .resource::<Content>()
            .by_path::<Row>(&path)
            .unwrap_or_else(|| panic!("the test game has no {} `{path}`", Row::TABLE))
    }

    /// Runs `systems` every tick, one after another in the order given, after those added before.
    pub fn add<M>(&mut self, systems: impl IntoScheduleConfigs<ScheduleSystem, M>) -> &mut Sim {
        let previous = Step(self.steps);
        self.steps += 1;
        let step = Step(self.steps);
        self.world.schedule_scope(Main, |_, schedule| {
            schedule.configure_sets(step.after(previous));
            schedule.add_systems(systems.chain().in_set(step));
        });
        self
    }

    /// Lets the world carry messages of `M`, as an app that adds them does.
    pub fn message<M: Message>(&mut self) -> &mut Sim {
        if !self.world.contains_resource::<Messages<M>>() {
            MessageRegistry::register_message::<M>(&mut self.world);
        }
        self
    }

    /// Every message of `M` the cursor hasn't seen, of those still held.
    pub fn read<M: Message + Clone>(&mut self, cursor: &mut MessageCursor<M>) -> Vec<M> {
        self.message::<M>();
        let messages = self.world.resource::<Messages<M>>();
        cursor.read(messages).cloned().collect()
    }

    /// One tick: the wall clock moves on by the server's tick, then every added system runs
    /// once, the world's time moving on with them.
    pub fn tick(&mut self) {
        let mut clock = self.world.resource_mut::<WallClock>();
        clock.now = clock.now.after(Seconds(TICK_HZ.period().as_secs_f32()));
        self.world.run_schedule(Main);
    }

    /// Ticks for up to `seconds` until `done` holds, and tells whether it did.
    pub fn run_until(&mut self, seconds: f32, mut done: impl FnMut(&mut World) -> bool) -> bool {
        let ticks = (seconds * TICK_HZ.0).ceil() as usize;
        for _ in 0..ticks {
            if done(&mut self.world) {
                return true;
            }
            self.tick();
        }
        done(&mut self.world)
    }

    pub fn run(&mut self, seconds: f32) {
        self.run_until(seconds, |_| false);
    }

    /// The wall clock jumps `seconds` ahead, as it does over a stretch nobody plays through, and
    /// the world catches up for half a second.
    pub fn later(&mut self, seconds: f32) {
        let mut clock = self.world.resource_mut::<WallClock>();
        clock.now = clock.now.after(Seconds(seconds));
        self.run(0.5);
    }
}

fn advance_time(mut time: ResMut<Time>) {
    time.advance_by(TICK_HZ.period());
}

// Tests hold on any seed: the one drawn shows in a failing test's output, to play it out again.
pub fn drawn_seed() -> u64 {
    let seed = Rng::random_seed();
    eprintln!("rng seed {seed}");
    seed
}
