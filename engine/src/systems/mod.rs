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
pub mod staging;
pub mod stat;
pub mod status;
pub mod terminal;
pub mod text;
pub mod vfx;
pub mod view;
pub mod visibility;

use bevy_app::App;
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::{Entity, Res, ResMut, Resource, World};
use bevy_replicon::prelude::FromClient;
use schemars::JsonSchema;

use crate::core::assets::SyncedFiles;
use crate::core::audio::playback::{SfxDef, SfxPlugin};
use crate::core::audio::soundscape::SoundscapePlugin;
use crate::core::babble::BabblePlugin;
use crate::core::content::sync::SyncedContent;
use crate::core::content::{
    Content, ContentFolder, ContentProblem, ContentRegistry, ContentVersion, LoadReport, Loading,
    MODULES, SETTINGS_FILE, Settings,
};
use crate::core::gui::GuiPlugin;
use crate::core::interpolate::SnapshotPlugin;
use crate::core::math::Rng;
use crate::core::net::auth::Credentials;
use crate::core::net::transport::SocketClientPlugin;
use crate::core::platform::{ClientPlatform, Platform, SignIn, StartParams};
use crate::core::render::WorldRenderPlugin;
use crate::core::text::StyledTextPlugin;
use crate::core::time::{DayClock, Hertz, Seconds, WallClock};
use crate::core::vfx::render::VfxRenderPlugin;
use crate::systems::area::AreaId;
use crate::systems::player::PlayerDefId;

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
    bevy_terminal::register(app, &[]);
    vfx::register(app);
    plug_ins(app.world_mut());
}

/// Wires each system's say into the registries other systems read, so neither names the other: what
/// an actor's effective stats are made of, and what draws a player's attention.
pub fn plug_ins(world: &mut World) {
    world.init_resource::<stat::Sources>();
    world.init_resource::<attention::MarkSources>();
    world.init_resource::<attention::BadgeSources>();
    stat::source(world, status::boost::modifiers);
    stat::source(world, class::level_stats);
    stat::source(world, skill::casting::chase);
    attention::badge_source(world, dialogue::talking);
    attention::mark_source(world, interact::marks);
    attention::mark_source(world, shop::marks);
    attention::mark_source(world, quest::marks);
}

/// What an area's server keeps besides its entities: the game's content and the scripts and parties
/// built from it, who plays, where everyone stands and who sees them, who is on their way out, the
/// clocks of regeneration, resets and conversations, and the messages its systems pass one another.
pub fn area_state(world: &mut World, content: &Content) {
    use bevy_ecs::message::MessageRegistry;

    insert_content(world, content);
    world.init_resource::<script::RiftInstances>();
    world.init_resource::<player::Players>();
    world.init_resource::<movement::Spots>();
    world.init_resource::<area::transition::Departures>();
    world.init_resource::<combat::RegenAt>();
    world.init_resource::<skill::reset::PendingResets>();
    world.init_resource::<dialogue::StepCounter>();
    world.init_resource::<visibility::Sights>();
    MessageRegistry::register_message::<combat::Died>(world);
    MessageRegistry::register_message::<rewards::KillCredited>(world);
    world.add_observer(player::greet);
    world.add_observer(player::client_left);
    world.add_observer(equipment::reflect);
}

/// The content and the scripts and parties built from it, as every world playing it reads them.
fn insert_content(world: &mut World, content: &Content) {
    world.insert_resource(content.built::<quest::Parties>().clone());
    world.insert_resource(content.built::<script::RiftScripts>().clone());
    world.insert_resource(content.clone());
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
    /// Without one the game has no clock: every area's light stands as authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daylight: Option<area::DayCycle>,
}

pub fn server_app(
    area: AreaId,
    ordinal: u64,
    rng_seed: u64,
    clock: WallClock,
    daylight: launch::ServerDaylight,
    content: &Content,
) -> App {
    use bevy_app::{First, PostUpdate, Startup, Update};
    use bevy_ecs::schedule::{IntoScheduleConfigs, Schedules, SingleThreadedExecutor};
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin, ServerState};
    use bevy_replicon::server::{ServerSystems, increment_tick};
    use bevy_state::prelude::in_state;

    let mut app = App::new();
    let settings = content.built::<GameSettings>();
    app.insert_resource(WorldArea(area));
    app.insert_resource(settings.clone());
    app.insert_resource(clock);
    app.insert_resource(DayClock(
        settings
            .daylight
            .map(|cycle| cycle.course(daylight, clock.now)),
    ));
    app.insert_resource(input::map::InputMap::new(content));
    app.add_plugins((bevy_time::TimePlugin, bevy_state::app::StatesPlugin));
    // Every tick steps the same span of game time however long it took, so a seeded world plays out
    // alike tick for tick.
    app.insert_resource(bevy_time::TimeUpdateStrategy::ManualDuration(
        TICK_HZ.period(),
    ));
    app.insert_resource(Rng::for_world(rng_seed, ordinal));
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
    area_state(app.world_mut(), content);
    // Seed the phase by the world's ordinal so worlds replicate on different ticks, spreading the
    // serialization load across ticks instead of spiking every Nth tick in lockstep.
    app.insert_resource(ReplicationClock(ordinal))
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
                    equipment::hold,
                    equipment::unequip,
                    skill::reset::expire,
                    actor::playing::progress,
                    skill::casting::casts,
                    reach::mirror_engaged,
                )
                    .chain(),
                (
                    (rewards::grant, quest::credit, npc::defeated).chain(),
                    (
                        script::query::take_tick_roster,
                        area::object::track.run_if(on_replication_tick),
                        // Every tick, so whoever walks up is near before they can talk: a check
                        // between replication ticks would let the talk run first.
                        npc::approach,
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
                    bevy_terminal::ingest,
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

pub fn from_client<M: Message>(world: &mut World, conn: Entity, message: M) {
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
        SignIn::Token(token) => Some(Credentials::from_access_token(token)),
        SignIn::AnyAccount | SignIn::Provider(_) => None,
    };

    let mut app = App::new();
    let files = SyncedFiles::default();
    app.insert_resource(files.clone());
    app.insert_resource(Rng::from_entropy());
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
            GuiPlugin,
            StyledTextPlugin,
            SocketClientPlugin,
            SnapshotPlugin {
                period: REPLICATION_PERIOD,
            },
            WorldRenderPlugin,
            VfxRenderPlugin,
            SfxPlugin,
            SoundscapePlugin,
            BabblePlugin,
        ))
        .add_plugins((
            screen::ClientScreenPlugin { credentials },
            area::render::AreaRenderPlugin,
            actor::render::ActorRenderPlugin,
            actor::plate::PlatePlugin,
            movement::MovementRenderPlugin,
            combat::render::CombatRenderPlugin,
            item::render::ItemRenderPlugin,
            view::CameraFollowPlugin,
            player::session::ClientSessionPlugin,
            input::PlayerInputPlugin,
        ))
        .add_plugins((
            settings::plugin,
            hud::HudPlugin,
            dialogue::stage::DialogueStagePlugin,
            notification::NotificationPlugin,
            history::widget::HistoryPlugin,
            shop::counter::ShopCounterPlugin,
            item::card::ItemCardPlugin,
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
        registry(),
        folder,
        crate::core::content::version::build(),
        outcome,
    )
}

/// Loads a game still being made, through `making_registry`.
pub fn load_making(
    folder: std::sync::Arc<dyn ContentFolder>,
    outcome: &str,
) -> Result<Content, LoadReport> {
    crate::core::content::load(
        making_registry(),
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
    insert_content(world, &content);
    world.insert_resource(content.built::<GameSettings>().clone());
    world.insert_resource(input::map::InputMap::new(&content));
    world.insert_resource(synced);
    Ok(())
}

/// A whole game: the tables it's made of, its settings, every row the engine itself requires, and
/// every quest and shop within a player's reach.
pub fn registry() -> &'static ContentRegistry {
    use crate::core::content::LoadStep;
    // Built once a process, as its content version hashes every shape it reads.
    static REGISTRY: std::sync::LazyLock<ContentRegistry> = std::sync::LazyLock::new(|| {
        let mut registry = settled_tables();
        registry
            .require_fixtures()
            .hook(LoadStep::Across, script::check_reached);
        registry
    });
    &REGISTRY
}

/// A game being made: all a whole game is, but that its quests and shops may still be out of a
/// player's reach, as one is made before the scripts that offer or open it can name it.
pub fn making_registry() -> &'static ContentRegistry {
    static REGISTRY: std::sync::LazyLock<ContentRegistry> = std::sync::LazyLock::new(|| {
        let mut registry = settled_tables();
        registry.require_fixtures();
        registry
    });
    &REGISTRY
}

/// Every table a game is made of, and the settings its `game.config.json` holds: a folder of only some of
/// a game's rows, set up to be played, loads through this.
pub fn settled_tables() -> ContentRegistry {
    use crate::core::content::LoadStep;
    let mut registry = tables();
    registry
        .settings::<GameSettings>()
        .hook(LoadStep::Settings, check_settings);
    registry
}

/// Every table a game is made of and what the engine builds from them, without what a whole game
/// must also provide: a folder of only some of a game's rows loads through this.
pub fn tables() -> ContentRegistry {
    use crate::core::content::LoadStep;
    let mut registry = ContentRegistry::default();
    registry
        .fixture::<input::map::InputFixture>()
        .fixture::<attention::AttentionMark>()
        .fixture::<interface::InterfaceSound>()
        .fixture::<interface::CursorShape>()
        .fixture::<interface::InterfaceIcon>()
        .fixture::<vfx::VfxFixture>()
        .fixture::<vfx::VfxShaderFixture>()
        .fixture::<area::world_map::WorldMap>()
        .bundle::<SfxDef>()
        .file::<attention::AttentionDef>()
        .file::<input::map::InputDef>()
        .file::<interface::CursorDef>()
        .file::<interface::InterfaceIconDef>()
        .file::<item::ItemDef>()
        .named::<item::ItemDef>()
        .modules::<item::ItemDef>()
        .file::<class::ClassDef>()
        .named::<class::ClassDef>()
        .file::<memory::MemoryDef>()
        .named::<memory::MemoryDef>()
        .file::<notification::NotificationDef>()
        .file::<npc::NpcDef>()
        .named::<npc::NpcDef>()
        .modules::<npc::NpcDef>()
        .file::<player::PlayerDef>()
        .modules::<player::PlayerDef>()
        .file::<player::snapshot::CharacterSnapshot>()
        .named::<player::snapshot::CharacterSnapshot>()
        .file::<quest::QuestDef>()
        .named::<quest::QuestDef>()
        .modules::<quest::QuestDef>()
        .file::<script::ScriptDef>()
        .file::<shop::ShopDef>()
        .named::<shop::ShopDef>()
        .modules::<shop::ShopDef>()
        .file::<skill::SkillDef>()
        .named::<skill::SkillDef>()
        .modules::<skill::SkillDef>()
        .file::<status::StatusEffectDef>()
        .named::<status::StatusEffectDef>()
        .modules::<status::StatusEffectDef>()
        .list_kind(MODULES, "unknown module type", "a module type")
        .hook(LoadStep::Files, crate::core::content::media::check_media);
    crate::core::babble::content(&mut registry);
    actor::content(&mut registry);
    dialogue::bust::content(&mut registry);
    area::content(&mut registry);
    crate::core::vfx::content(&mut registry);
    registry
        .hook(LoadStep::Expand, script::build_all)
        .hook(LoadStep::Across, shop::check_all)
        .hook(LoadStep::Across, quest::check_all)
        .hook(LoadStep::Across, notification::check_all)
        .hook(LoadStep::Across, script::check_all)
        .hook(LoadStep::Across, class::check_all)
        .hook(LoadStep::Across, npc::check_all)
        .hook(LoadStep::Across, skill::check_all)
        .hook(LoadStep::Across, item::check_pictures)
        .hook(LoadStep::Across, item::check_looks)
        .hook(LoadStep::Across, vfx::check_sites)
        .hook(LoadStep::Across, player::snapshot::check_all);
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
    if let Some(cycle) = &settings.daylight {
        let mut check = crate::core::content::RowCheck::default();
        cycle.check("/daylight", &mut check);
        for error in check.into_errors() {
            loading.json_error(SETTINGS_FILE, &error.pointer, error.problem);
        }
    }
    if let Err(problem) = area::check_place(&content, settings.initial_spawn) {
        loading.json_error(
            SETTINGS_FILE,
            "/initial_spawn",
            ContentProblem::new(problem.rule, problem.found, problem.expected)
                .near(problem.nearest),
        );
    }
    for (site, problem) in area::check_islands(&content, settings.initial_spawn) {
        match site {
            area::IslandSite::Scene(file, pointer) => loading.json_error(&file, &pointer, problem),
            area::IslandSite::Spawn => loading.json_error(SETTINGS_FILE, "/initial_spawn", problem),
        }
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
