pub mod account;
pub mod actor;
pub mod area;
pub mod attention;
pub mod belongings;
pub mod chat;
pub mod class;
pub mod combat;
pub mod dialogue;
pub mod equipment;
pub mod frame_rate;
pub mod history;
pub mod hud;
pub mod input;
pub mod interact;
pub mod interface;
pub mod item;
pub mod launch;
pub mod memory;
pub mod movement;
pub mod notification;
pub mod npc;
pub mod player;
pub mod probe;
pub mod quest;
pub mod reach;
pub mod rewards;
pub mod screen;
pub mod script;
pub mod settings;
pub mod shop;
pub mod skill;
pub mod stat;
pub mod status;
pub mod terminal;
pub mod text;
pub mod vfx;
pub mod view;
pub mod visibility;

use bevy_app::App;
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::{Bundle, Entity, Res, ResMut, Resource, World};
use bevy_replicon::prelude::{FromClient, Replicated};
use schemars::JsonSchema;

use crate::core::assets::SyncedFiles;
use crate::core::content::sync::SyncedContent;
use crate::core::content::{
    Content, ContentFolder, ContentProblem, ContentRegistry, ContentVersion, GAME_FILE, LoadReport,
    Loading, Settings,
};
use crate::core::platform::{ClientPlatform, Platform, SignIn, StartParams};
use crate::core::time::{Hertz, Seconds, WallClock};
use crate::systems::area::AreaId;
use crate::systems::player::PlayerDefId;
use actor::{Actor, Hitbox};
use area::AreaTag;
use movement::Position;
use terminal::TerminalKind;

pub const TICK_HZ: Hertz = Hertz(30.0);

/// Replication serialization is the dominant server cost and scales with its rate, so state goes out
/// every Nth tick (clients interpolate between the sparser snapshots) rather than every tick.
pub const REPLICATION_INTERVAL: u64 = 3;

pub const REPLICATION_PERIOD: Seconds = Seconds(REPLICATION_INTERVAL as f32 / TICK_HZ.0);

pub fn protocol(app: &mut App) {
    actor::register(app);
    crate::core::babble::register(app);
    notification::register(app);
    area::register(app);
    attention::register(app);
    combat::register(app);
    dialogue::register(app);
    stat::register(app);
    status::register(app);
    equipment::register(app);
    history::register(app);
    item::register(app);
    class::register(app);
    skill::register(app);
    movement::register(app);
    interact::register(app);
    npc::register(app);
    shop::register(app);
    quest::register(app);
    player::register(app);
    reach::register(app);
    terminal::register(app);
    vfx::register(app);
}

#[derive(Resource, Clone, Copy)]
pub struct WorldArea(pub AreaId);

#[derive(JsonSchema, Resource, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameSettings {
    pub name: String,
    pub content_version: ContentVersion,
    pub initial_spawn: script::Place,
    pub player: PlayerDefId,
}

#[derive(Bundle)]
pub struct Character {
    pub replicated: Replicated,
    pub position: Position,
    pub actor: Actor,
    pub hitbox: Hitbox,
    pub area: AreaTag,
}

pub fn server_app(
    area: AreaId,
    ordinal: u64,
    rng_seed: u64,
    clock: WallClock,
    assets: &Content,
) -> App {
    use bevy_app::{First, PostUpdate, Startup, Update};
    use bevy_ecs::schedule::{IntoScheduleConfigs, Schedules, SingleThreadedExecutor};
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin, ServerState};
    use bevy_replicon::server::{ServerSystems, increment_tick};
    use bevy_state::prelude::in_state;

    let mut app = App::new();
    app.insert_resource(WorldArea(area));
    app.insert_resource(assets.built::<GameSettings>().clone());
    app.insert_resource(clock);
    app.insert_resource(assets.clone());
    app.insert_resource(assets.built::<quest::Parties>().clone());
    app.insert_resource(assets.built::<script::RiftScripts>().clone());
    app.insert_resource(input::map::InputMap::new(assets));
    app.init_resource::<script::RiftInstances>();
    app.add_plugins((bevy_time::TimePlugin, bevy_state::app::StatesPlugin));
    // Every tick steps the same span of game time however long it took, so a seeded world plays out
    // alike tick for tick.
    app.insert_resource(bevy_time::TimeUpdateStrategy::ManualDuration(
        TICK_HZ.period(),
    ));
    app.insert_resource(crate::core::math::Rng::for_world(rng_seed, ordinal));
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
        .init_resource::<area::transition::Departures>()
        .init_resource::<combat::RegenAt>()
        .init_resource::<skill::reset::PendingResets>()
        // Seed the phase by the world's ordinal so worlds replicate on different ticks, spreading the
        // serialization load across ticks instead of spiking every Nth tick in lockstep.
        .insert_resource(ReplicationClock(ordinal))
        .add_message::<combat::Died>()
        .add_message::<rewards::KillCredited>()
        .add_observer(player::greet)
        .add_observer(player::client_left)
        .add_systems(
            Startup,
            (
                script::spawn_stateless,
                area::object::spawn_all,
                npc::spawn_all,
            )
                .chain(),
        )
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
                    (status::expire, status::aspect::settle, status::aspect::tick).chain(),
                    vfx::expire,
                    player::arrival_holds,
                    combat::regen,
                    movement::move_request,
                    combat::request,
                    skill::requests,
                    item::use_item,
                    item::drop_item,
                    item::pickup_request,
                    interact::interact_request,
                    dialogue::requests,
                    shop::requests,
                    quest::requests,
                    equipment::unequip,
                    skill::reset::expire,
                    actor::playing::progress,
                    skill::using::casts,
                    reach::mirror_engaged,
                )
                    .chain(),
                (
                    (rewards::grant, quest::credit, npc::defeated).chain(),
                    (
                        script::query::take_tick_roster,
                        (area::object::track, npc::approach)
                            .chain()
                            .run_if(on_replication_tick),
                    )
                        .chain(),
                    script::run_tick,
                    npc::body::drive,
                    movement::advance,
                    item::pickups,
                    interact::interactions,
                    (
                        dialogue::hold,
                        dialogue::refresh.run_if(on_replication_tick),
                    )
                        .chain(),
                    (shop::hold, shop::refresh.run_if(on_replication_tick)).chain(),
                    quest::refresh.run_if(on_replication_tick),
                    item::expire_drops,
                    player::join,
                    player::respawn,
                    (npc::run_respawn, npc::dismiss).chain(),
                )
                    .chain(),
                (
                    stat::settle_health,
                    combat::defeats,
                    skill::hotbar::settle_basic_attack,
                    actor::settle,
                    bevy_terminal::ingest::<TerminalKind>,
                    bevy_terminal::dispatch::<TerminalKind>,
                    chat::rebroadcast,
                    belongings::notify_changes,
                    history::backfill,
                    attention::update.run_if(on_replication_tick),
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

pub fn start_server(app: &mut App) {
    use bevy_replicon::prelude::ServerState;
    use bevy_state::prelude::NextState;

    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update();
}

pub fn from_client<M: bevy_ecs::message::Message>(world: &mut World, conn: Entity, message: M) {
    use bevy_replicon::prelude::{ClientId, FromClient};

    world.write_message(FromClient {
        client_id: ClientId::Client(conn),
        message,
    });
}

pub fn client_app(
    platform: std::sync::Arc<dyn Platform>,
    params: StartParams,
    window: bevy::window::Window,
) -> App {
    use bevy::prelude::*;

    let credentials = match &params.sign_in {
        SignIn::Token(token) => Some(crate::core::net::auth::Credentials::from_access_token(
            token,
        )),
        SignIn::AnyAccount | SignIn::Provider(_) => None,
    };

    let mut app = App::new();
    let files = SyncedFiles::default();
    app.insert_resource(files.clone());
    app.insert_resource(crate::core::math::Rng::from_entropy());
    bevy::asset::AssetApp::register_asset_source(
        &mut app,
        bevy::asset::io::AssetSourceId::Default,
        files.source(),
    )
    .add_plugins(
        platform.engine(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(window),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        ),
    );
    // The client shows a world the server plays out in real time, so a frame however long moves it
    // on by all the time the frame took, as the server's world moved on, rather than by the most the
    // engine otherwise lets one frame take.
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(std::time::Duration::MAX);
    app.insert_resource(params)
        .insert_resource(ClientPlatform(platform))
        .add_plugins((
            crate::core::gui::GuiPlugin,
            crate::core::text::TextPlugin,
            crate::core::net::transport::SocketClientPlugin,
            crate::core::interpolate::SnapshotPlugin {
                period: REPLICATION_PERIOD,
            },
            crate::core::render::RenderPlugin,
            crate::core::vfx::render::VfxRenderPlugin,
            crate::core::audio::playback::SfxPlugin,
            crate::core::audio::soundscape::SoundscapePlugin,
            crate::core::babble::BabblePlugin,
        ))
        .add_plugins((
            screen::ClientScreenPlugin { credentials },
            actor::render::ActorPlugin,
            actor::plate::PlatePlugin,
            movement::MovementPlugin,
            combat::render::CombatPlugin,
            item::render::ItemsPlugin,
            area::object::render::ObjectPlugin,
            view::ViewPlugin,
            player::session::ClientSessionPlugin,
            input::InputPlugin,
        ))
        .add_plugins((
            hud::HudPlugin,
            dialogue::stage::StagePlugin,
            notification::NotificationPlugin,
            history::widget::HistoryPlugin,
            shop::counter::ShopCounterPlugin,
            item::card::ItemCardPlugin,
            item::widget::CarryPlugin,
            skill::client::SkillClientPlugin,
            quest::card::QuestCardPlugin,
            quest::log::QuestLogPlugin,
            quest::tracker::QuestTrackerPlugin,
            terminal::TerminalPlugin,
            frame_rate::FrameRatePlugin,
        ));
    app
}

/// The worlds, not the owning `App`s, are stepped because `App` is `!Send` while `World` is `Send`;
/// for these single-schedule headless apps `run_schedule(Main)` + `clear_trackers` is what
/// `App::update` does.
pub fn step_areas(apps: &mut [App], clock: WallClock) {
    use bevy_app::Main;
    use rayon::prelude::*;

    apps.iter_mut()
        .map(App::world_mut)
        .collect::<Vec<_>>()
        .into_par_iter()
        .for_each(|world| {
            world.insert_resource(clock);
            world.run_schedule(Main);
            world.clear_trackers();
        });
}

pub fn load_content(
    folder: std::sync::Arc<dyn ContentFolder>,
    outcome: &str,
) -> Result<Content, LoadReport> {
    crate::core::content::load(
        &registry(),
        folder,
        crate::core::content::version::build(),
        outcome,
    )
}

pub fn install_content(world: &mut World, synced: SyncedContent) -> Result<(), LoadReport> {
    let before = world.get_resource::<SyncedContent>();
    if before.is_some_and(|before| before.same_as(&synced)) {
        return Ok(());
    }
    let content = load_content(
        std::sync::Arc::new(synced.folder()),
        "the content did not load",
    )?;
    let changed = before.map(|before| synced.changed_assets(before));
    world
        .resource::<SyncedFiles>()
        .replace(synced.assets.clone());
    if let Some(changed) = changed {
        let assets = world.resource::<bevy::asset::AssetServer>();
        for path in changed {
            let path = bevy::asset::AssetPath::from(path);
            if assets.get_path_id(path.clone()).is_some() {
                assets.reload(path);
            }
        }
    }
    world.insert_resource(content.built::<quest::Parties>().clone());
    world.insert_resource(content.built::<script::RiftScripts>().clone());
    world.insert_resource(content.built::<GameSettings>().clone());
    if let Some(fonts) = interface::theme_fonts(&content, &synced.assets) {
        world.insert_resource(crate::core::gui::GuiFonts(fonts));
    }
    world.insert_resource(input::map::InputMap::new(&content));
    world.insert_resource(content);
    world.insert_resource(synced);
    Ok(())
}

pub fn registry() -> ContentRegistry {
    use crate::core::content::LoadStep;
    let mut registry = ContentRegistry::default();
    registry
        .row::<crate::core::audio::playback::SfxDef>()
        .row::<crate::core::audio::playback::SfxRoleDef>()
        .row::<attention::AttentionDef>()
        .row::<input::map::InputDef>()
        .row::<interface::CursorDef>()
        .row::<interface::FontDef>()
        .row::<interface::InterfaceIconDef>()
        .row::<item::ItemDef>()
        .modules::<item::ItemDef>()
        .row::<class::ClassDef>()
        .row::<memory::MemoryDef>()
        .row::<notification::NotificationDef>()
        .row::<npc::NpcDef>()
        .modules::<npc::NpcDef>()
        .row::<player::PlayerDef>()
        .modules::<player::PlayerDef>()
        .row::<player::snapshot::CharacterSnapshot>()
        .row::<quest::QuestDef>()
        .modules::<quest::QuestDef>()
        .row::<script::ScriptDef>()
        .row::<shop::ShopDef>()
        .modules::<shop::ShopDef>()
        .row::<skill::SkillDef>()
        .modules::<skill::SkillDef>()
        .row::<status::StatusEffectDef>()
        .modules::<status::StatusEffectDef>()
        .list_kind("modules", "unknown module type", "a module type")
        .fixture::<input::map::InputAction>()
        .fixture::<attention::AttentionMark>()
        .fixture::<interface::InterfaceSound>()
        .fixture::<interface::CursorShape>()
        .fixture::<interface::InterfaceIcon>()
        .fixture::<interface::FontFace>()
        .fixture::<vfx::VfxFixture>()
        .fixture::<vfx::VfxShaderFixture>()
        .settings::<GameSettings>()
        .hook(LoadStep::Files, crate::core::content::media::check_media);
    crate::core::babble::content(&mut registry);
    actor::content(&mut registry);
    area::content(&mut registry);
    crate::core::vfx::content(&mut registry);
    registry
        .hook(LoadStep::Expand, script::build_all)
        .hook(LoadStep::Across, shop::check_all)
        .hook(LoadStep::Across, input::map::check_all)
        .hook(LoadStep::Across, quest::check_all)
        .hook(LoadStep::Across, notification::check_all)
        .hook(LoadStep::Across, script::check_all)
        .hook(LoadStep::Across, class::check_all)
        .hook(LoadStep::Across, skill::check_all)
        .hook(LoadStep::Across, player::snapshot::check_all)
        .hook(LoadStep::Settings, check_settings);
    registry
}

impl Settings for GameSettings {
    fn content_version(&self) -> &ContentVersion {
        &self.content_version
    }
}

fn check_settings(loading: &mut Loading) {
    let content = loading.content();
    let settings = content.built::<GameSettings>();
    if let Err(problem) = area::check_place(&content, settings.initial_spawn) {
        loading.json_error(
            GAME_FILE,
            "/initial_spawn",
            ContentProblem::new(problem.rule, problem.found, problem.expected)
                .near(problem.nearest),
        );
    }
}

pub(crate) fn drain_requests<M: Message>(world: &mut World) -> Vec<FromClient<M>> {
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
