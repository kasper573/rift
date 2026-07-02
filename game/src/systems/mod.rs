pub mod account;
pub mod actor;
pub mod area;
pub mod chat;
pub mod combat;
pub mod debug;
pub mod effect;
pub mod equipment;
pub mod fps;
pub mod hud;
pub mod input;
pub mod item;
pub mod job;
pub mod movement;
pub mod npc;
pub mod player;
pub mod rewards;
pub mod scene;
pub mod settings;
pub mod spectate;
pub mod stat;
pub mod terminal;
pub mod view;
pub mod visibility;

use std::collections::HashMap;
use std::sync::LazyLock;

use bevy_app::App;
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::{Bundle, Res, ResMut, Resource, World};
use bevy_replicon::prelude::{FromClient, Replicated};
use bevy_terminal::Terminal;
use strum::VariantArray;

use actor::{Actor, Hitbox};
use area::AreaTag;
use movement::Position;

pub const TICK_HZ: crate::core::time::Hertz = crate::core::time::Hertz(30.0);

/// Simulation ticks per replication. The world steps every tick, but state is replicated to clients
/// (and visibility recomputed) only every Nth tick; clients interpolate between the sparser
/// snapshots. Replication serialization is the dominant server cost and scales with this rate, so
/// replicating at `TICK_HZ / REPLICATION_INTERVAL` (10 Hz) rather than every tick cuts it ~3x.
pub const REPLICATION_INTERVAL: u64 = 3;

/// Seconds of server time covered by one replication snapshot.
pub const REPLICATION_PERIOD: crate::core::time::Seconds =
    crate::core::time::Seconds(REPLICATION_INTERVAL as f32 / TICK_HZ.0);

static TERMINALS: LazyLock<HashMap<crate::data::terminal::Id, &'static Terminal>> =
    LazyLock::new(|| {
        crate::data::terminal::Id::VARIANTS
            .iter()
            .copied()
            .zip(crate::data::terminal::TABLE)
            .collect()
    });

pub fn protocol(app: &mut App) {
    actor::register(app);
    area::register(app);
    combat::register(app);
    stat::register(app);
    effect::register(app);
    equipment::register(app);
    item::register(app);
    job::register(app);
    movement::register(app);
    npc::register(app);
    player::register(app);
    spectate::register(app);
    bevy_terminal::register(app, &TERMINALS);
}

#[derive(Resource, Clone, Copy)]
pub struct WorldArea(pub area::Id);

#[derive(Bundle)]
pub struct Character {
    pub replicated: Replicated,
    pub position: Position,
    pub actor: Actor,
    pub hitbox: Hitbox,
    pub area: AreaTag,
}

/// Builds one area world. `ordinal` is the world's slot among the worlds a driver steps concurrently;
/// it only staggers the replication phase, so identical area instances still replicate on different
/// ticks. The driver passes each world's index.
pub fn server_app(area: area::Id, ordinal: u64) -> App {
    use bevy_app::{First, PostUpdate, Startup, Update};
    use bevy_ecs::schedule::{IntoScheduleConfigs, Schedules, SingleThreadedExecutor};
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin, ServerState};
    use bevy_replicon::server::{ServerSystems, increment_tick};
    use bevy_state::prelude::in_state;

    let mut app = App::new();
    app.insert_resource(WorldArea(area));
    app.add_plugins((bevy_time::TimePlugin, bevy_state::app::StatesPlugin));
    app.add_plugins(
        bevy_app::PluginGroup::build(bevy_replicon::prelude::RepliconPlugins)
            .set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            })
            // No tick schedule: we drive replication ourselves at REPLICATION_INTERVAL via
            // `on_replication_tick`, rather than letting replicon tick every frame.
            .set(bevy_replicon::server::ServerPlugin {
                tick_schedule: None,
                ..Default::default()
            }),
    );
    protocol(&mut app);
    visibility::register(&mut app);
    app.init_resource::<player::Players>()
        .init_resource::<spectate::Spectators>()
        .init_resource::<combat::RegenAt>()
        // Seed the phase by the world's ordinal so worlds replicate on different ticks, spreading the
        // serialization load across ticks instead of spiking every Nth tick in lockstep.
        .insert_resource(ReplicationClock(ordinal))
        .add_message::<combat::Died>()
        .add_observer(player::greet)
        .add_observer(player::client_left)
        .add_observer(spectate::client_left)
        .add_systems(Startup, npc::spawn_all)
        .add_systems(First, advance_replication_clock)
        .add_systems(
            PostUpdate,
            increment_tick
                .in_set(ServerSystems::IncrementTick)
                .run_if(in_state(ServerState::Running))
                .run_if(on_replication_tick),
        )
        .add_systems(
            Update,
            (
                (
                    actor::reset,
                    combat::regen,
                    npc::run_ai,
                    movement::move_request,
                    movement::move_to_portal,
                    combat::request,
                    item::use_item,
                    item::drop_item,
                    item::pickup_request,
                    equipment::unequip,
                    effect::expire,
                    combat::combat,
                )
                    .chain(),
                (
                    rewards::grant,
                    movement::advance,
                    item::pickups,
                    item::expire_drops,
                    player::join,
                    player::respawn,
                    spectate::requests,
                    spectate::follow,
                    npc::run_respawn,
                    bevy_terminal::ingest::<crate::data::terminal::Id>,
                    bevy_terminal::dispatch::<crate::data::terminal::Id>,
                    chat::rebroadcast,
                    visibility::update.run_if(on_replication_tick),
                )
                    .chain(),
            )
                .chain(),
        );

    // An area world is stepped single-threaded so the driver (server/bench) owns all parallelism by
    // running whole areas concurrently. Letting each area's schedules fan out onto the global task
    // pool too would nest a work-stealing pool inside the per-area pool and oversubscribe the cores.
    for (_, schedule) in app.world_mut().resource_mut::<Schedules>().iter_mut() {
        schedule.set_executor(SingleThreadedExecutor::new());
    }
    app
}

/// Steps every area world one tick, fanning the isolated worlds across all cores. Areas share only the
/// read-only asset service, so this is embarrassingly parallel. The worlds (not the owning `App`s) are
/// stepped directly because `App` is `!Send` (its runner box carries no `Send` bound) while `World` is
/// `Send`; for these single-schedule headless apps `world.run_schedule(Main)` + `clear_trackers` is
/// exactly what `App::update` does.
pub fn step_areas(apps: &mut [App]) {
    use bevy_app::Main;
    use rayon::prelude::*;

    apps.iter_mut()
        .map(App::world_mut)
        .collect::<Vec<_>>()
        .into_par_iter()
        .for_each(|world| {
            world.run_schedule(Main);
            world.clear_trackers();
        });
}

pub(crate) fn requests<M: Message>(world: &mut World) -> Vec<FromClient<M>> {
    world
        .resource_mut::<Messages<FromClient<M>>>()
        .drain()
        .collect()
}

#[derive(Resource)]
struct ReplicationClock(u64);

fn advance_replication_clock(mut clock: ResMut<ReplicationClock>) {
    clock.0 = clock.0.wrapping_add(1);
}

fn on_replication_tick(clock: Res<ReplicationClock>) -> bool {
    clock.0.is_multiple_of(REPLICATION_INTERVAL)
}
