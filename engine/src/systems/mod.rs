pub mod account;
pub mod actor;
pub mod area;
pub mod attention;
pub mod belongings;
pub mod chat;
pub mod class;
pub mod combat;
pub mod crafting;
pub mod dialogue;
pub mod equipment;
pub mod frame_rate;
pub mod history;
pub mod hud;
pub mod identity;
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
pub mod prompt;
pub mod quest;
pub mod reach;
pub mod region;
pub mod resource;
pub mod rewards;
pub mod screen;
pub mod script;
pub mod settings;
pub mod shop;
pub mod skill;
pub mod sleep;
pub mod spawner;
pub mod staging;
pub mod stat;
pub mod status;
pub mod terminal;
pub mod text;
pub mod vfx;
pub mod view;
pub mod visibility;
pub mod worlds;

use bevy_app::App;
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::{Entity, ResMut, Resource, World};
use bevy_replication::FromClient;
use schemars::JsonSchema;

use crate::core::assets::SyncedAssetReader;
use crate::core::audio::Hearing;
use crate::core::audio::playback::{SfxDef, SfxPlugin};
use crate::core::audio::soundscape::SoundscapePlugin;
use crate::core::babble::{BabblePlugin, Speech};
use crate::core::content::sync::SyncedContent;
use crate::core::content::{
    Content, ContentFile, ContentFolder, ContentProblem, ContentRegistry, ContentVersion,
    LoadPurpose, LoadReport, Loading, MODULES, RowCheck, SETTINGS_FILE, SettingsFile,
};
use crate::core::gui::GuiPlugin;
use crate::core::interpolate::SnapshotPlugin;
use crate::core::math::Rng;
use crate::core::net::auth::Credentials;
use crate::core::net::transport::ClientTransportPlugin;
use crate::core::os::{Http, Profile, SignIn, StartParams};
use crate::core::render::Look;
use crate::core::render::WorldRenderPlugin;
use crate::core::text::{Lettering, StyledTextPlugin, Typing};
use crate::core::tick::{self, Tick};
use crate::core::time::{DayClock, TimeCourse, WallClock, WeatherClock};
use crate::core::units::{Hertz, Meters, Seconds};
use crate::core::vfx::render::VfxRenderPlugin;
use crate::systems::area::AreaId;
use crate::systems::player::PlayerDefId;

pub const TICK_HZ: Hertz = Hertz(30.0);

/// Replication serialization is the dominant server cost and scales with its rate, so state goes out
/// every Nth tick (clients interpolate between the sparser snapshots) rather than every tick.
pub const REPLICATION_INTERVAL: u64 = 3;

pub const REPLICATION_PERIOD: Seconds = Seconds(REPLICATION_INTERVAL as f32 / TICK_HZ.0);

pub fn protocol(app: &mut App) {
    identity::register(app);
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
    prompt::register(app);
    crafting::register(app);
    resource::register(app);
    quest::register(app);
    player::register(app);
    reach::register(app);
    chat::register(app);
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
    attention::mark_source(world, crafting::marks);
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
    world.init_resource::<player::Connected>();
    world.init_resource::<player::Unwelcomed>();
    world.init_resource::<combat::RegenAt>();
    world.init_resource::<skill::reset::PendingResets>();
    world.init_resource::<dialogue::StepCounter>();
    world.init_resource::<prompt::PromptSteps>();
    world.init_resource::<crate::core::chunk::ChunkIndex>();
    world.init_resource::<npc::Packs>();
    world.init_resource::<spawner::SpawnerEntries>();
    world.insert_resource(sleep::Sleep::of(content));
    world.insert_resource(region::Region::of(content));
    MessageRegistry::register_message::<combat::Died>(world);
    MessageRegistry::register_message::<rewards::KillCredited>(world);
    world.add_observer(player::greet);
    world.add_observer(player::client_left);
    world.add_observer(equipment::reflect);
    world.add_observer(resource::take_out);
    world.add_observer(resource::put_away);
}

/// The content, its settings and the scripts and parties built from it, as every world playing it
/// reads them.
fn insert_content(world: &mut World, content: &Content) {
    let settings = content.built::<GameSettings>();
    world.insert_resource(settings.look);
    world.insert_resource(settings.typing);
    world.insert_resource(settings.lettering);
    world.insert_resource(settings.speech);
    world.insert_resource(settings.hearing);
    world.insert_resource(settings.clone());
    let bodies = movement::Bodies::of(content).expect("a game that loaded knows its bodies");
    world.insert_resource(bodies);
    world.insert_resource(movement::Spots::new(bodies));
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
    /// How far a player sees: what their client is told of, and so all they may aim at, reach
    /// for or speak to.
    pub sight: Meters,
    /// How many times their pace an actor runs while reaching for a cast.
    pub chase_pace: f32,
    /// Without it, water slows no one who wades through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wading: Option<movement::Wading>,
    /// How long a fallen npc that never comes back lies before it is gone.
    pub corpses_linger: Seconds,
    /// How long a player's claim on a foe holds after their last hit: whoever holds it when the
    /// foe falls is credited with the fall, and what it drops is theirs.
    pub claims_last: Seconds,
    pub slots: SlotCounts,
    pub drops: DroppedItems,
    pub reading: ReadingPace,
    pub notices: NoticeTimes,
    pub words: GameWords,
    pub typing: Typing,
    pub lettering: Lettering,
    pub speech: Speech,
    pub hearing: Hearing,
    pub look: Look,
    pub dialogue_shots: dialogue::camera::DialogueShots,
    pub relations: SideRelations,
    /// How soon after each other like entries of a player's history count as one, a run of loot
    /// or kills read as a tally.
    pub history_merges_within: Seconds,
}

/// How much a player keeps.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotCounts {
    /// Stacks of items in a bag; the most rewards a quest may hand out at once.
    pub bag: u32,
    /// Slots of a hotbar, however a player lays them out.
    pub hotbar: u32,
    /// Quests under way at once.
    pub quest_log: u32,
}

/// How the sides stand to each other: the players' characters, the npcs friendly to them and the
/// npcs hostile to them.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideRelations {
    /// Characters to each other.
    pub characters: combat::Regard,
    pub characters_and_friendly: combat::Regard,
    pub characters_and_hostile: combat::Regard,
    /// Friendly npcs to each other.
    pub friendly: combat::Regard,
    /// Npcs of different packs, unless both are friendly.
    pub npcs_apart: combat::Regard,
}

/// What the game says in its own voice where the engine puts it into words.
#[derive(JsonSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameWords {
    /// The player's choice taking up a quest offered to them.
    pub accept_quest: String,
    /// The player's choice turning a quest offer down.
    pub decline_quest: String,
    /// The player's choice leaving a quest unhanded for now.
    pub not_yet: String,
    /// What a player is told picking a choice not open to them yet.
    pub locked: String,
    /// What a player is told picking a choice no longer open to them.
    pub no_longer: String,
    /// What a player is told offered a quest with every slot of their quest log taken.
    pub quest_log_full: String,
    /// What a trade asking for items a player lacks says: `{count}` more of the item `{item}`.
    pub needs_more: String,
    /// What a trade says that a player's bag has no room for, one slot short.
    pub needs_a_slot: String,
    /// What a trade says that a player's bag has no room for, `{slots}` slots short.
    pub needs_slots: String,
    /// What a player is told working a resource without a tool of the kind `{tool}` in their
    /// tool slot.
    pub needs_tool: String,
    /// What a player is told working a resource as many already work as it has room for.
    pub no_room: String,
    /// What a player is told working a resource their bag has no room for the drops of.
    pub bag_room: String,
    /// What a player is told as the resource they work or set out for has nothing left.
    pub ran_out: String,
    /// The button submitting what a player placed in a prompt.
    pub confirm: String,
    /// The button crafting what a player placed in a crafting prompt.
    pub craft: String,
    /// The button calling off the craft a crafting prompt has under way.
    pub cancel_craft: String,
    /// The button and the title of the window listing what a crafting prompt makes.
    pub recipes: String,
    /// What a crafting prompt says of items placed that are no recipe of its own.
    pub no_recipe: String,
    /// What a crafting prompt asks as the items placed are several recipes.
    pub choose_result: String,
    /// What bursts out of a crafter as a craft of theirs fails.
    pub craft_failed: String,
}

/// How long notices take to read: each stays a glance and a little longer for every word, and
/// stays at least a glance before a newer one in its place replaces it.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingPace {
    pub glance: Seconds,
    pub per_word: Seconds,
}

impl ReadingPace {
    /// How long a notice of `words` stays.
    pub fn of(self, words: usize) -> Seconds {
        Seconds(self.glance.0 + self.per_word.0 * words as f32)
    }
}

/// How the notices that aren't read move.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeTimes {
    /// How long a number bursting out of an actor flies before it is gone.
    pub pops_last: Seconds,
    /// How long an area stands settled after it comes into view before its intro lands, so the
    /// title and its sound don't arrive on the frame the transition ends.
    pub intro_after: Seconds,
}

/// What becomes of items on the ground.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DroppedItems {
    /// How long an item lies on the ground, unless what dropped it says otherwise.
    pub last: Seconds,
    /// How far from where they fell several items lie, around it.
    pub scatter: Meters,
    /// How close a player comes to an item to pick it up.
    pub pickup_reach: Meters,
    /// How far from an item on the ground a click still picks it.
    pub click_reach: Meters,
}

/// How the server runs its world's day and weather as it starts them: what staging returns the
/// world to once a bench is done holding either.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ServerClocks {
    pub day: Option<TimeCourse<f32>>,
    pub weather: TimeCourse<usize>,
}

/// Runs the world's day and weather on `clocks`, as the server does from its start.
pub fn start_clocks(world: &mut World, clocks: ServerClocks) {
    world.insert_resource(DayClock(clocks.day));
    world.insert_resource(WeatherClock(Some(clocks.weather)));
    world.insert_resource(clocks);
}

pub fn server_app(
    area: AreaId,
    ordinal: u64,
    rng_seed: u64,
    clock: WallClock,
    day: launch::ServerDay,
    weather: TimeCourse<usize>,
    content: &Content,
) -> App {
    use bevy_app::{First, PostUpdate, Startup, Update};
    use bevy_ecs::schedule::{IntoScheduleConfigs, Schedules, SingleThreadedExecutor};
    use bevy_replication::{Sending, ServerReplicationPlugin};

    let mut app = App::new();
    let settings = content.built::<GameSettings>();
    app.insert_resource(WorldArea(area));
    app.insert_resource(clock);
    start_clocks(
        app.world_mut(),
        ServerClocks {
            day: settings.daylight.map(|cycle| cycle.course(day, clock.now)),
            weather,
        },
    );
    app.insert_resource(input::map::InputMap::new(content));
    app.add_plugins(bevy_time::TimePlugin);
    // Every tick steps the same span of game time however long it took, so a seeded world plays out
    // alike tick for tick.
    app.insert_resource(bevy_time::TimeUpdateStrategy::ManualDuration(
        TICK_HZ.period(),
    ));
    app.insert_resource(Rng::for_world(rng_seed, ordinal));
    app.insert_resource(identity::Names::for_world(ordinal));
    app.add_plugins(ServerReplicationPlugin {
        sending: Sending::ByHost,
    });
    protocol(&mut app);
    visibility::register(&mut app);
    area_state(app.world_mut(), content);
    // Seed the phase by the world's ordinal so worlds replicate on different ticks, spreading the
    // serialization load across ticks instead of spiking every Nth tick in lockstep.
    app.insert_resource(ReplicationClock(ordinal))
        .add_systems(Startup, script::spawn_stateless)
        .add_systems(
            First,
            (
                player::welcome,
                advance_replication_clock,
                (sleep::settle, region::settle).chain(),
            ),
        )
        .add_systems(PostUpdate, movement::file_by_chunk)
        .add_systems(
            Update,
            (
                (
                    status::expire,
                    vfx::expire,
                    player::arrival_holds,
                    combat::regen,
                    movement::move_request,
                    combat::request,
                    skill::requests,
                    item::use_item,
                    item::drop_item,
                    (item::pickup_request, resource::gather_request).chain(),
                    interact::interact_request,
                    dialogue::requests,
                    (shop::requests, prompt::requests, crafting::requests).chain(),
                    quest::requests,
                    equipment::hold,
                    equipment::unequip,
                    skill::reset::expire,
                    (actor::playing::progress, resource::swings, crafting::moves).chain(),
                    reach::mirror_engaged,
                    (rewards::grant, quest::credit, npc::defeated).chain(),
                )
                    .chain(),
                tick::run,
                (
                    item::pickups,
                    (resource::gathering, resource::work).chain(),
                    interact::interactions,
                    (dialogue::hold, dialogue::refresh.run_if(replication_tick)).chain(),
                    (shop::hold, shop::refresh.run_if(replication_tick)).chain(),
                    (prompt::hold, crafting::hold, crafting::work).chain(),
                    item::expire_drops,
                    player::join,
                    player::respawn,
                    (npc::clear_fallen, spawner::refill, npc::dismiss).chain(),
                    combat::defeats,
                    skill::hotbar::settle_basic_attack,
                    chat::hear,
                    chat::overhear,
                    chat::rebroadcast,
                    belongings::notify_changes,
                    history::backfill,
                    identity::name_replicated.run_if(replication_tick),
                )
                    .chain(),
            )
                .chain(),
        );
    app.insert_resource(tick_planners());

    // The schedules run single-threaded: the parallel work, an area's planners and the areas'
    // ticks side by side, all runs on one pool, and bevy's executor would fan systems out onto a
    // pool of its own beside it and oversubscribe the cores.
    for (_, schedule) in app.world_mut().resource_mut::<Schedules>().iter_mut() {
        schedule.set_executor(SingleThreadedExecutor::new());
    }
    app
}

/// The planners of an area's tick, in the order their plans are written: what every script,
/// body, walk, cast, status and quest comes to, and what each client is told of it, worked out
/// on every core at once from the world as the tick found it.
fn tick_planners() -> Tick {
    let mut tick = Tick::default();
    tick.add((
        status::aspect::SettleAspects::default(),
        status::aspect::TickAspects::default(),
        skill::casting::StepCasts::default(),
        script::query::ListRoster::default(),
        area::object::TrackUnderfoot::default(),
        npc::ApproachWatchers::default(),
        script::RunFlows::default(),
        npc::body::DriveBodies::default(),
        movement::AdvanceWalks::default(),
        stat::SettleHealth::default(),
        actor::SettleActions::default(),
        attention::MarkAttention::default(),
        quest::AnnounceQuests::default(),
        visibility::SendSights::default(),
    ));
    tick
}

pub fn start_server(app: &mut App) {
    app.finish();
    app.cleanup();
    app.update();
}

pub fn from_client<M: Message>(world: &mut World, conn: Entity, message: M) {
    world.write_message(FromClient {
        client: conn,
        message,
    });
}

pub fn client_app(
    profile: Profile,
    http: Http,
    params: StartParams,
    window: bevy::window::Window,
) -> App {
    use bevy::prelude::*;

    let credentials = match &params.sign_in {
        SignIn::Token(token) => Some(Credentials::from_access_token(token)),
        SignIn::AnyAccount | SignIn::Browser(_) => None,
    };

    let mut app = App::new();
    let files = SyncedAssetReader::default();
    app.insert_resource(files.clone());
    app.insert_resource(Rng::from_entropy());
    bevy::asset::AssetApp::register_asset_source(
        &mut app,
        bevy::asset::io::AssetSourceId::Default,
        files.source(),
    )
    .add_plugins(
        DefaultPlugins
            .set(bevy::log::LogPlugin {
                filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                custom_layer: probe::timing_layer,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(window),
                ..default()
            })
            .set(ImagePlugin::default_nearest()),
    );
    // The client shows a world the server plays out in real time, so a frame however long moves it
    // on by all the time the frame took, as the server's world moved on, rather than by the most the
    // engine otherwise lets one frame take.
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(std::time::Duration::MAX);
    app.insert_resource(params)
        .insert_resource(profile)
        .insert_resource(http)
        .add_plugins((
            GuiPlugin,
            StyledTextPlugin,
            ClientTransportPlugin,
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
            resource::render::ResourceRenderPlugin,
            view::CameraFollowPlugin,
            dialogue::camera::DialogueCameraPlugin,
            player::session::ClientSessionPlugin,
            input::PlayerInputPlugin,
            region::ClientRegionPlugin,
        ))
        .add_plugins((
            settings::plugin,
            hud::HudPlugin,
            dialogue::stage::DialogueStagePlugin,
            notification::NotificationPlugin,
            history::widget::HistoryPlugin,
            shop::counter::ShopCounterPlugin,
            prompt::tray::PromptTrayPlugin,
            crafting::panel::CraftingPanelPlugin,
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

/// One tick of one area's world. The world is stepped rather than its `App`, which can't cross
/// threads; for a headless app this is what `App::update` does.
pub fn step_area(world: &mut World, clock: WallClock) {
    world.insert_resource(clock);
    world.run_schedule(bevy_app::Main);
    world.clear_trackers();
}

pub fn load_content(
    folder: std::sync::Arc<dyn ContentFolder>,
    outcome: &str,
    purpose: LoadPurpose,
) -> Result<Content, LoadReport> {
    load_through(registry(), folder, outcome, purpose)
}

/// Loads a game still being made, through `making_registry`, for `purpose`.
pub fn load_making(
    folder: std::sync::Arc<dyn ContentFolder>,
    outcome: &str,
    purpose: LoadPurpose,
) -> Result<Content, LoadReport> {
    load_through(making_registry(), folder, outcome, purpose)
}

fn load_through(
    registry: &ContentRegistry,
    folder: std::sync::Arc<dyn ContentFolder>,
    outcome: &str,
    purpose: LoadPurpose,
) -> Result<Content, LoadReport> {
    crate::core::content::load(
        registry,
        folder,
        crate::core::content::version::build(),
        outcome,
        purpose,
    )
}

/// Puts `content`, loaded from `synced`, into a client's world, and has the assets the sync changed
/// read again.
pub fn install_content(world: &mut World, synced: SyncedContent, content: Content) {
    let changed = world
        .get_resource::<SyncedContent>()
        .map(|before| synced.changed_assets(before));
    world
        .resource::<SyncedAssetReader>()
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
    world.insert_resource(input::map::InputMap::new(&content));
    world.insert_resource(synced);
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
            .hook(LoadStep::Whole, script::check_reached);
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
        .file::<item::ToolTypeDef>()
        .named::<item::ToolTypeDef>()
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
        .file::<crafting::RecipeBookDef>()
        .modules::<crafting::RecipeBookDef>()
        .file::<quest::QuestDef>()
        .named::<quest::QuestDef>()
        .modules::<quest::QuestDef>()
        .file::<resource::ResourceDef>()
        .named::<resource::ResourceDef>()
        .modules::<resource::ResourceDef>()
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
    area::content(&mut registry);
    crate::core::vfx::content(&mut registry);
    registry
        .hook(LoadStep::Expand, script::build_all)
        .hook(LoadStep::Across, shop::check_all)
        .hook(LoadStep::Across, resource::check_all)
        .hook(LoadStep::Across, crafting::check_all)
        .hook(LoadStep::Across, quest::check_all)
        .hook(LoadStep::Settings, quest::check_rewards)
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

impl SettingsFile for GameSettings {
    fn content_version(&self) -> &ContentVersion {
        &self.content_version
    }
}

fn check_settings(loading: &mut Loading) {
    let content = loading.content();
    let settings = content.built::<GameSettings>();
    let mut check = RowCheck::default();
    if let Some(cycle) = &settings.daylight {
        cycle.check("/daylight", &mut check);
    }
    check.at_least("/sight", settings.sight.0, 0.0, "meters");
    check.at_least("/chase_pace", settings.chase_pace, 1.0, "times");
    if let Some(wading) = &settings.wading {
        wading.check("/wading", &mut check);
    }
    check.at_least("/corpses_linger", settings.corpses_linger.0, 0.0, "seconds");
    check.at_least("/claims_last", settings.claims_last.0, 0.0, "seconds");
    for (field, slots) in [
        ("bag", settings.slots.bag),
        ("hotbar", settings.slots.hotbar),
        ("quest_log", settings.slots.quest_log),
    ] {
        check.at_least(format!("/slots/{field}"), slots, 1, "slots");
    }
    let drops = &settings.drops;
    check.at_least("/drops/last", drops.last.0, 0.0, "seconds");
    check.at_least("/drops/scatter", drops.scatter.0, 0.0, "meters");
    check.at_least("/drops/pickup_reach", drops.pickup_reach.0, 0.0, "meters");
    check.at_least("/drops/click_reach", drops.click_reach.0, 0.0, "meters");
    for (field, said) in [
        ("accept_quest", &settings.words.accept_quest),
        ("decline_quest", &settings.words.decline_quest),
        ("not_yet", &settings.words.not_yet),
        ("locked", &settings.words.locked),
        ("no_longer", &settings.words.no_longer),
        ("quest_log_full", &settings.words.quest_log_full),
        ("needs_more", &settings.words.needs_more),
        ("needs_a_slot", &settings.words.needs_a_slot),
        ("needs_slots", &settings.words.needs_slots),
        ("needs_tool", &settings.words.needs_tool),
        ("no_room", &settings.words.no_room),
        ("bag_room", &settings.words.bag_room),
        ("ran_out", &settings.words.ran_out),
        ("confirm", &settings.words.confirm),
        ("craft", &settings.words.craft),
        ("cancel_craft", &settings.words.cancel_craft),
        ("recipes", &settings.words.recipes),
        ("no_recipe", &settings.words.no_recipe),
        ("choose_result", &settings.words.choose_result),
    ] {
        check.text(format!("/words/{field}"), said, 1, 80);
    }
    check_typing(&mut check, &settings.typing);
    check_lettering(&mut check, &settings.lettering);
    check_speech(&mut check, &settings.speech);
    check_hearing(&mut check, &settings.hearing);
    check.at_least("/reading/glance", settings.reading.glance.0, 0.0, "seconds");
    check.at_least(
        "/reading/per_word",
        settings.reading.per_word.0,
        0.0,
        "seconds",
    );
    check.at_least(
        "/notices/pops_last",
        settings.notices.pops_last.0,
        0.0,
        "seconds",
    );
    check.at_least(
        "/notices/intro_after",
        settings.notices.intro_after.0,
        0.0,
        "seconds",
    );
    check.at_least(
        "/history_merges_within",
        settings.history_merges_within.0,
        0.0,
        "seconds",
    );
    check_look(&mut check, &settings.look);
    dialogue::camera::check(&mut check, &settings.dialogue_shots);
    for error in check.into_errors() {
        loading.json_error(SETTINGS_FILE, &error.pointer, error.problem);
    }
    if loading.thorough() {
        check_reach(loading, &content, settings.initial_spawn);
    }
}

fn check_typing(check: &mut RowCheck, typing: &Typing) {
    check.at_least("/typing/slowest", typing.slowest.0, 1.0, "letters a second");
    check.range(
        "/typing/fastest",
        typing.fastest.0,
        typing.slowest.0,
        f32::MAX,
        "letters a second, no slower than the slowest",
    );
    check.range(
        "/typing/pace",
        typing.pace.0,
        typing.slowest.0,
        typing.fastest.0,
        "letters a second, between the slowest and the fastest",
    );
    check.at_least("/typing/slow_by", typing.slow_by, 1.0, "times");
}

fn check_lettering(check: &mut RowCheck, lettering: &Lettering) {
    for (field, size) in [
        ("whisper/size", lettering.whisper.size),
        ("shout_size", lettering.shout_size),
        ("caps/size", lettering.caps.size),
    ] {
        check.range(
            format!("/lettering/{field}"),
            size,
            0.25,
            4.0,
            "times the text's size",
        );
    }
    check.range(
        "/lettering/whisper/opacity",
        lettering.whisper.opacity,
        0.0,
        1.0,
        "a share",
    );
    check.at_least("/lettering/caps/room", lettering.caps.room, 0.0, "ems");
    check.at_least("/lettering/halo", lettering.halo, 0.0, "points");
    for (field, swing) in [
        ("wave", lettering.wave),
        ("shake/across", lettering.shake.across),
        ("shake/down", lettering.shake.down),
        ("pulse", lettering.pulse),
    ] {
        check.at_least(
            format!("/lettering/{field}/reach"),
            swing.reach,
            0.0,
            "points or a share",
        );
    }
}

fn check_speech(check: &mut RowCheck, speech: &Speech) {
    for (field, gap) in [
        ("word", speech.gaps.word),
        ("clause", speech.gaps.clause),
        ("sentence", speech.gaps.sentence),
    ] {
        check.at_least(format!("/speech/gaps/{field}"), gap.0, 0.0, "seconds");
    }
    for (field, shift) in [
        ("exclaimed", speech.exclaimed),
        ("shout", speech.shout),
        ("whisper", speech.whisper),
    ] {
        check.at_least(
            format!("/speech/{field}/loudness"),
            shift.loudness,
            0.0,
            "times",
        );
    }
    check.at_least("/speech/pitches", speech.pitches, 2, "pitches");
}

fn check_hearing(check: &mut RowCheck, hearing: &Hearing) {
    check.at_least("/hearing/wide", hearing.wide.0, 0.5, "meters");
    check.at_least("/hearing/deep", hearing.deep.0, 0.5, "meters");
    check.at_least(
        "/hearing/repeats_after",
        hearing.repeats_after.0,
        0.0,
        "seconds",
    );
    check.at_least("/hearing/glide", hearing.glide.0, 0.01, "seconds");
}

/// The rules of a look a schema can't state: a camera that can see, between its own limits, with
/// its zooms in order.
fn check_look(check: &mut RowCheck, look: &Look) {
    let camera = &look.camera;
    check.range("/look/camera/fov", camera.fov.0, 1.0, 170.0, "degrees");
    check.range(
        "/look/camera/highest_pitch",
        camera.highest_pitch.0,
        camera.lowest_pitch.0,
        90.0,
        "degrees",
    );
    check.range(
        "/look/camera/pitch",
        camera.pitch.0,
        camera.lowest_pitch.0,
        camera.highest_pitch.0,
        "degrees",
    );
    let zooms = &camera.zooms;
    check.range(
        "/look/camera/zooms/middle",
        zooms.middle.0,
        zooms.close.0,
        zooms.wide.0,
        "meters",
    );
    check.at_least("/look/camera/zooms/close", zooms.close.0, 0.1, "meters");
    check.at_least("/look/camera/turn_ease", camera.turn_ease.0, 0.0, "seconds");
    check.at_least("/look/camera/zoom_ease", camera.zoom_ease.0, 0.0, "seconds");
    check.at_least("/look/camera/shot_ease", camera.shot_ease.0, 0.0, "seconds");
    check.at_least(
        "/look/clearing/fades",
        look.clearing.fades.0,
        0.0,
        "seconds",
    );
    check.at_least(
        "/look/clearing/radius",
        look.clearing.radius.0,
        0.0,
        "meters",
    );
}

/// Whether the initial spawn stands on ground, and everything placed in an area stands where the
/// spawn or a warp into it reaches.
fn check_reach(loading: &mut Loading, content: &Content, spawn: script::Place) {
    if let Err(problem) = area::check_place(content, spawn) {
        loading.json_error(
            SETTINGS_FILE,
            "/initial_spawn",
            ContentProblem::new(problem.rule, problem.found, problem.expected)
                .near(problem.nearest),
        );
    }
    let rechecks = |id: area::AreaId| loading.rechecks(area::AreaDef::TABLE, id.path(content));
    for (site, problem) in area::check_islands(content, spawn, rechecks) {
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

/// Whether this tick is one the server replicates on; a world without the clock replicates every
/// tick.
pub fn replication_tick(world: &World) -> bool {
    world
        .get_resource::<ReplicationClock>()
        .is_none_or(|clock| clock.0.is_multiple_of(REPLICATION_INTERVAL))
}
