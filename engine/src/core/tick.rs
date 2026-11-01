//! A tick planned on every core at once, then written in order. Every planner works out what
//! its tick comes to from the world as the tick found it, beside every other planner, with the
//! cores shared among them all; then each writes its plan to the world in the order the planners
//! were added. A planner sees the other planners' writes the next tick, never the same one.

use bevy_ecs::prelude::*;
use bevy_ecs::query::{QueryFilter, ReadOnlyQueryData};
use bevy_ecs::system::SystemState;

/// A system split in two: what it works out from the world as the tick found it, on every
/// core beside every other planner, and what it then writes, in its turn.
pub trait Planner: Send + Sync + 'static {
    /// Readies the planner before any plan is made, on its own: queries matched to the world,
    /// and whatever the plan must own taken out of it.
    fn prepare(&mut self, world: &mut World) {
        let _ = world;
    }

    /// Works out what the tick comes to, from the world as the tick found it, and keeps it.
    fn plan(&mut self, world: &World);

    /// Writes what was planned to the world.
    fn apply(&mut self, world: &mut World);
}

/// The planners of a world's tick, in the order their plans are written.
#[derive(Resource, Default)]
pub struct Tick {
    planners: Vec<Box<dyn Planner>>,
}

impl Tick {
    pub fn add(&mut self, planners: impl Planners) -> &mut Tick {
        self.planners.extend(planners.into_planners());
        self
    }
}

/// One or more planners, as a planner or a tuple of them.
pub trait Planners {
    fn into_planners(self) -> Vec<Box<dyn Planner>>;
}

impl<P: Planner> Planners for P {
    fn into_planners(self) -> Vec<Box<dyn Planner>> {
        vec![Box::new(self)]
    }
}

macro_rules! planners_tuple {
    ($($name:ident),+) => {
        impl<$($name: Planner),+> Planners for ($($name,)+) {
            #[allow(non_snake_case)]
            fn into_planners(self) -> Vec<Box<dyn Planner>> {
                let ($($name,)+) = self;
                vec![$(Box::new($name) as Box<dyn Planner>),+]
            }
        }
    };
}

planners_tuple!(A, B);
planners_tuple!(A, B, C);
planners_tuple!(A, B, C, D);
planners_tuple!(A, B, C, D, E);
planners_tuple!(A, B, C, D, E, F);
planners_tuple!(A, B, C, D, E, F, G);
planners_tuple!(A, B, C, D, E, F, G, H);
planners_tuple!(A, B, C, D, E, F, G, H, I);
planners_tuple!(A, B, C, D, E, F, G, H, I, J);
planners_tuple!(A, B, C, D, E, F, G, H, I, J, K);
planners_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
planners_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
planners_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);

/// Runs the world's tick: every planner readied in turn, every plan made at once, every plan
/// written in turn.
pub fn run(world: &mut World) {
    world.resource_scope(|world, mut tick: Mut<Tick>| {
        let planners = &mut tick.planners;
        for planner in planners.iter_mut() {
            planner.prepare(world);
        }
        let shared: &World = world;
        rayon::scope(|scope| {
            for planner in planners.iter_mut() {
                scope.spawn(move |_| planner.plan(shared));
            }
        });
        // The plans' writes land on a tick of their own, after the one the plans read at, so the
        // next tick's change filters see them.
        world.increment_change_tick();
        for planner in planners.iter_mut() {
            planner.apply(world);
        }
    });
}

/// A query a planner keeps between ticks, made when the planner readies and read while it
/// plans. It counts the ticks it ran itself, so a change filter on it sees every write since
/// the planner last planned, the tick's own writes among them.
pub struct Cached<D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static>(
    Option<SystemState<Query<'static, 'static, D, F>>>,
);

impl<D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static> Default for Cached<D, F> {
    fn default() -> Cached<D, F> {
        Cached(None)
    }
}

impl<D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static> Cached<D, F> {
    pub fn ready(&mut self, world: &mut World) {
        self.0.get_or_insert_with(|| SystemState::new(world));
    }

    pub fn query<'w, 's>(&'s mut self, world: &'w World) -> Query<'w, 's, D, F> {
        self.0
            .as_mut()
            .expect("a cached query is readied before it is read")
            .get(world)
            .expect("a query always validates")
    }
}
