pub mod account;
pub mod actor;
pub mod aim;
pub mod area;
pub mod area_instance;
pub mod area_map;
pub mod attention;
pub mod belongings;
pub mod chat;
pub mod class;
pub mod combat;
pub mod command;
pub mod crafting;
pub mod dialogue;
pub mod duration;
pub mod emote;
pub mod equipment;
pub mod frame_rate;
pub mod friend;
pub mod history;
pub mod hud;
pub mod identity;
pub mod input;
pub mod interact;
pub mod interface;
pub mod item;
pub mod launch;
pub mod mail;
pub mod market;
pub mod memory;
pub mod mention;
pub mod messages;
pub mod mount;
pub mod movement;
pub mod notification;
pub mod npc;
pub mod party;
pub mod player;
pub mod probe;
pub mod prompt;
pub mod quest;
pub mod reach;
pub mod region;
pub mod requirement;
pub mod resource;
pub mod reward;
pub mod roster;
pub mod save;
pub mod screen;
pub mod settings;
pub mod shop;
pub mod skill;
pub mod sleep;
pub mod social_request;
pub mod society;
pub mod spawner;
pub mod staging;
pub mod stat;
pub mod status;
pub mod terminal;
pub mod trade;
pub mod view;
pub mod visibility;
pub mod window;
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
use crate::core::card::{CardHandling, CardStyleDef, CardStyleFixture};
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
use crate::core::script::{Instances, Scripts, Vocabulary};
use crate::core::stage::StagePlugin;
use crate::core::text::{Lettering, SpanKinds, StyledTextPlugin, Typing};
use crate::core::tick::{self, Tick};
use crate::core::time::{DayClock, TimeCourse, WallClock, WeatherClock};
use crate::core::units::{Hertz, HourOfDay, Meters, Seconds, WindowPx};
use crate::core::vfx::Stages;
use crate::core::vfx::render::VfxRenderPlugin;
use crate::systems::area::AreaId;
use crate::systems::player::CharacterDefId;

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
    save::notice::register(app);
    social_request::register(app);
    society::view::register(app);
    friend::register(app);
    mail::register(app);
    party::register(app);
    mount::register(app);
    area_instance::register(app);
    trade::register(app);
    market::register(app);
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
    emote::register(app);
    plug_ins(app.world_mut());
}

/// The kinds of request between players the engine's systems bring.
pub fn request_sources() -> Vec<social_request::SocialRequestSource> {
    vec![
        friend::request_source(),
        party::request_source(),
        trade::request_source(),
    ]
}

/// Wires each system's say into the registries other systems read, so neither names the other: what
/// an actor's effective stats are made of, and what draws a player's attention.
pub fn plug_ins(world: &mut World) {
    world.init_resource::<stat::Sources>();
    world.init_resource::<attention::MarkSources>();
    world.init_resource::<attention::BadgeSources>();
    world.init_resource::<chat::Sources>();
    chat::source(world, chat::area_channel());
    chat::source(world, party::chat_channel());
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
    world.init_resource::<Instances>();
    world.init_resource::<player::Players>();
    world.init_resource::<player::Connected>();
    world.init_resource::<player::Unwelcomed>();
    world.init_resource::<player::Departures>();
    world.init_resource::<player::roster::ServerRoster>();
    world.init_resource::<player::roster::Enrollment>();
    world.init_resource::<player::roster::EnrollmentQuery>();
    world.init_resource::<player::errand::Errands>();
    world.init_resource::<society::ServerSociety>();
    world.init_resource::<society::SocietyAsks>();
    world.init_resource::<area_instance::AreaInstanceTally>();
    world.init_resource::<trade::Trades>();
    world.init_resource::<combat::RegenAt>();
    world.init_resource::<skill::reset::PendingResets>();
    world.init_resource::<window::WindowSteps>();
    world.init_resource::<crate::core::chunk::ChunkIndex>();
    world.init_resource::<npc::Packs>();
    world.init_resource::<spawner::SpawnerEntries>();
    world.init_resource::<area::weather::ServerSky>();
    world.init_resource::<area::sky::Sky>();
    world.init_resource::<area::sky::DayWatch>();
    world.init_resource::<area::rain::ServerShelters>();
    world.insert_resource(sleep::Sleep::of(content));
    world.insert_resource(region::Region::of(content));
    MessageRegistry::register_message::<combat::Died>(world);
    MessageRegistry::register_message::<combat::credit::KillCredited>(world);
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
    world.insert_resource(settings.cards);
    world.insert_resource(settings.typing);
    world.insert_resource(settings.lettering);
    world.insert_resource(settings.speech);
    world.insert_resource(settings.hearing);
    world.insert_resource(settings.clone());
    let bodies = movement::Bodies::of(content).expect("a game that loaded knows its bodies");
    world.insert_resource(bodies);
    world.insert_resource(movement::Spots::new(bodies));
    world.insert_resource(content.built::<quest::Parties>().clone());
    world.insert_resource(content.built::<Scripts>().clone());
    world.insert_resource(content.clone());
}

#[derive(Resource, Clone, Copy)]
pub struct WorldArea(pub AreaId);

#[derive(JsonSchema, Resource, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameSettings {
    pub name: String,
    pub content_version: ContentVersion,
    pub initial_spawn: area::Place,
    /// What every new character starts as.
    pub character: CharacterDefId,
    /// Without one the game has no clock: every area's light stands as authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daylight: Option<area::DayCycle>,
    /// What stands where an npc lives while its absence leaves a notice that names no sign of
    /// its own, and from how far a sign is read. Without it, every notice names its sign, and is
    /// read from as far as its npc is talked to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absence_sign: Option<npc::AbsenceSign>,
    /// How far a player sees: what their client is told of, and so all they may aim at, reach
    /// for or speak to.
    pub sight: Meters,
    /// How far a player's map of the ground around them reaches, at each step they may choose.
    pub map_reaches: area_map::MapReaches,
    /// How many times their pace an actor runs while reaching for a cast.
    pub chase_pace: f32,
    /// Without it, water slows no one who wades through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wading: Option<movement::Wading>,
    /// How long a fallen npc that never comes back lies before it is gone.
    pub corpses_linger: Seconds,
    /// How long a character's claim on a foe holds after their last hit: whoever holds it when the
    /// foe falls is credited with the fall, and what it drops is theirs.
    pub claims_last: Seconds,
    pub slots: SlotCounts,
    pub drops: DroppedItems,
    /// The colour behind the picture on an item's card, by the item's type.
    pub item_type_art_colors: item::card::ItemTypeArtColors,
    pub cards: CardHandling,
    pub reading: ReadingPace,
    pub notices: NoticeSettings,
    pub typing: Typing,
    pub lettering: Lettering,
    pub speech: Speech,
    pub hearing: Hearing,
    pub look: Look,
    pub dialogue_shots: dialogue::camera::DialogueShots,
    pub relations: SideRelations,
    pub history: history::HistorySettings,
    /// How many ways to go on the terminal offers at the most for a command being typed, and how
    /// many players a search by name lists.
    pub suggestions: u32,
    /// The fewest letters of a name a search for a player goes by, as friends and party invites
    /// search.
    pub search_letters: u32,
    /// How many checkpoints fail in a row before players are told the game is not being saved.
    pub unsaved_told_after: u32,
    /// How long a character stays in the world once its player is gone, however they left. At
    /// zero it is gone at once.
    pub lingers_for: Seconds,
    pub requests: social_request::RequestSettings,
    pub chat: chat::ChatSettings,
    /// How friends work. A game without it has no friends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friends: Option<friend::FriendSettings>,
    /// How mail works. A game without it has no mail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<mail::MailSettings>,
    /// How parties work. A game without it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<party::PartySettings>,
    /// How players trade. A game without it has no trading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade: Option<trade::TradeSettings>,
    /// How the market works. A game without it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<market::MarketSettings>,
}

/// How much a character keeps.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotCounts {
    /// Stacks of items in a bag.
    pub bag: u32,
    /// Slots of the player bar, however a player lays them out.
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

/// How the notices that aren't read move, and how many reach a player.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeSettings {
    /// How long a number bursting out of an actor flies before it is gone.
    pub pops_last: Seconds,
    /// How long an area stands settled after it comes into view before its intro lands, so the
    /// title and its sound don't arrive on the frame the transition ends.
    pub intro_after: Seconds,
    /// How many lines the notification feed shows at once. The oldest make way for new ones.
    pub feed_rows: u32,
    /// The rate limit on the notifications a player is told live.
    pub budget: notification::budget::NotificationBudget,
}

/// What becomes of items on the ground.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DroppedItems {
    /// How long an item lies on the ground, unless what dropped it says otherwise.
    pub last: Seconds,
    /// How far from where they fell several items lie, around it.
    pub scatter: Meters,
    /// How close a character comes to an item to pick it up.
    pub pickup_reach: Meters,
    /// How far from an item on the ground a click still picks it.
    pub click_reach: Meters,
    /// How wide the line round an item is while the player asks for the names of items.
    pub outline: WindowPx,
    /// How wide the line round an item is while the pointer rests on it.
    pub outline_hover: WindowPx,
    /// The smallest an item is drawn across as it lies: a smaller model is scaled up to it.
    pub least_size: Meters,
    /// How long before it is gone an item pulses and its mark on the ground fades.
    pub fades_for: Seconds,
    /// How far from a player's character their items are named when they ask for names.
    pub names_reach: Meters,
    /// How many names of items show at once at the most.
    pub names_shown: u32,
    /// How many items play the vfx of their rarity at once at the most.
    pub effects_shown: u32,
}

/// How the server runs its world's day and weather as it starts them: what staging returns the
/// world to once a bench is done holding either.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ServerClocks {
    pub day: Option<TimeCourse<HourOfDay>>,
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
            day: settings
                .daylight
                .as_ref()
                .map(|cycle| cycle.course(day, clock.now)),
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
        .add_systems(Startup, spawn_scripts)
        .add_systems(
            First,
            (
                (area::weather::turn_sky, player::welcome).chain(),
                advance_replication_clock,
                (sleep::settle, region::settle).chain(),
            ),
        )
        .add_systems(PostUpdate, movement::file_by_chunk)
        .add_systems(
            Update,
            (
                (
                    (
                        status::expire,
                        area::vfx::expire,
                        area::vfx::follow,
                        area::spawnable::expire,
                    )
                        .chain(),
                    player::arrival_holds,
                    combat::regen,
                    movement::move_request,
                    combat::request,
                    skill::requests,
                    item::use_item,
                    (item::drop_item, item::arrange_stacks).chain(),
                    (item::pickup_request, resource::gather_request).chain(),
                    (
                        interact::interact_request,
                        trade::requests,
                        market::requests,
                    ),
                    dialogue::requests,
                    (shop::requests, prompt::requests, crafting::requests).chain(),
                    quest::requests,
                    (emote::requests, emote::rise).chain(),
                    equipment::equip_requested,
                    equipment::unequip,
                    skill::reset::expire,
                    (actor::playing::progress, resource::swings, crafting::moves).chain(),
                    reach::mirror_engaged,
                    (
                        (combat::credit::credit, reward::kills).chain(),
                        quest::credit,
                        npc::defeated,
                        area_instance::count_defeats,
                    )
                        .chain(),
                )
                    .chain(),
                tick::run,
                (
                    item::pickups,
                    (resource::gathering, resource::work).chain(),
                    (interact::interactions, trade::approach).chain(),
                    (dialogue::hold, dialogue::refresh.run_if(replication_tick)).chain(),
                    (
                        window::hold::<shop::ShopVisit>,
                        shop::refresh.run_if(replication_tick),
                    )
                        .chain(),
                    (
                        window::hold::<prompt::PromptSession>,
                        window::hold::<crafting::CraftingSession>,
                        crafting::work,
                    )
                        .chain(),
                    (
                        window::hold::<area_instance::entrance::EntranceSession>,
                        trade::hold,
                        window::hold::<market::MarketVisit>,
                    ),
                    item::expire_drops,
                    player::join,
                    (
                        save::notice::tell_returned,
                        save::notice::tell_saving,
                        society::view::greet,
                        social_request::requests,
                        friend::requests,
                        mail::requests,
                        mail::overflow,
                        party::requests,
                        mount::seat::requests,
                        (
                            area_instance::requests,
                            area_instance::entrance::requests,
                            area_instance::roster::requests,
                        ),
                        chat::channel::list,
                    ),
                    player::linger,
                    player::respawn,
                    (npc::clear_fallen, spawner::tend, npc::dismiss).chain(),
                    combat::defeats,
                    skill::hotbar::settle_basic_attack,
                    chat::hear,
                    (command::run, command::retire, command::complete::answer).chain(),
                    chat::speak,
                    belongings::notify_changes,
                    history::backfill,
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
        roster::ListRoster::default(),
        area::object::TrackUnderfoot::default(),
        npc::ApproachWatchers::default(),
        crate::core::script::run::FlowRunner::default(),
        npc::body::DriveBodies::default(),
        movement::AdvanceWalks::default(),
    ));
    tick.add((
        mount::UnseatFallen::default(),
        stat::SettleVitals::default(),
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

pub fn client_app(profile: Profile, http: Http, params: StartParams) -> App {
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
            StagePlugin,
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
            (
                actor::render::ActorRenderPlugin,
                actor::portrait::PortraitPlugin,
                actor::plate::PlatePlugin,
            ),
            movement::MovementRenderPlugin,
            combat::render::CombatRenderPlugin,
            (
                item::render::ItemRenderPlugin,
                item::plates::ItemPlatePlugin,
            ),
            resource::render::ResourceRenderPlugin,
            view::CameraFollowPlugin,
            visibility::SightEdgePlugin,
            dialogue::camera::DialogueCameraPlugin,
            player::session::ClientSessionPlugin,
            input::PlayerInputPlugin,
            region::ClientRegionPlugin,
            emote::wheel::EmoteWheelPlugin,
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
        ))
        .add_plugins((
            area_map::AreaMapPlugin,
            party::frames::PartyFramesPlugin,
            save::notice::SavingNoticePlugin,
            social_request::card::plugin,
            society::tab::plugin,
            friend::page::plugin,
            mail::page::plugin,
            party::page::plugin,
            mount::render::MountRenderPlugin,
            mount::plate::plugin,
            area_instance::plate::plugin,
            area_instance::entrance::plugin,
            area_instance::gather::plugin,
        ))
        .add_plugins((
            trade::window::plugin,
            market::window::plugin,
            actor::options::plugin,
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
/// every quest and shop within a character's reach.
pub fn registry() -> &'static ContentRegistry {
    use crate::core::content::LoadStep;
    // Built once a process, as its content version hashes every shape it reads.
    static REGISTRY: std::sync::LazyLock<ContentRegistry> = std::sync::LazyLock::new(|| {
        let mut registry = settled_tables();
        registry
            .require_fixtures()
            .hook(LoadStep::Whole, quest::check_reached)
            .hook(LoadStep::Whole, shop::check_reached)
            .hook(LoadStep::Whole, crafting::check_reached);
        registry
    });
    &REGISTRY
}

/// A game being made: all a whole game is, but that its quests and shops may still be out of a
/// character's reach, as one is made before the scripts that offer or open it can name it.
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
    install_vocabulary();
    install_stages();
    install_spans();
    let mut registry = ContentRegistry::default();
    registry
        .fixture::<input::map::InputFixture>()
        .fixture::<attention::AttentionMark>()
        .fixture::<interface::InterfaceSound>()
        .fixture::<interface::CursorShape>()
        .fixture::<interface::InterfaceIcon>()
        .fixture::<chat::ChatChannel>()
        .fixture::<stat::VitalVfx>()
        .fixture::<combat::DefeatVfx>()
        .fixture::<class::LevelVfx>()
        .fixture::<notification::pop::PopShader>()
        .fixture::<skill::aim::AreaShader>()
        .fixture::<area::world_map::WorldMap>()
        .fixture::<CardStyleFixture>()
        .bundle::<SfxDef>()
        .file::<CardStyleDef>()
        .file::<attention::AttentionDef>()
        .file::<input::map::InputDef>()
        .file::<interface::CursorDef>()
        .file::<interface::InterfaceIconDef>()
        .file::<chat::ChatChannelDef>()
        .file::<item::ItemDef>()
        .named::<item::ItemDef>()
        .modules::<item::ItemDef>()
        .file::<item::rarity::ItemRarityDef>()
        .named::<item::rarity::ItemRarityDef>()
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
        .file::<player::CharacterDef>()
        .modules::<player::CharacterDef>()
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
        .file::<crate::core::script::ScriptDef>()
        .file::<account::RoleDef>()
        .named::<account::RoleDef>()
        .file::<command::TerminalCommandDef>()
        .file::<shop::ShopDef>()
        .named::<shop::ShopDef>()
        .modules::<shop::ShopDef>()
        .file::<skill::SkillDef>()
        .named::<skill::SkillDef>()
        .modules::<skill::SkillDef>()
        .file::<emote::EmoteDef>()
        .named::<emote::EmoteDef>()
        .file::<status::StatusEffectDef>()
        .named::<status::StatusEffectDef>()
        .modules::<status::StatusEffectDef>()
        .file::<mount::MountDef>()
        .named::<mount::MountDef>()
        .modules::<mount::MountDef>()
        .list_kind(MODULES, "unknown module type", "a module type")
        .hook(LoadStep::Files, crate::core::content::media::check_media);
    crate::core::babble::content(&mut registry);
    messages::content(&mut registry);
    actor::content(&mut registry);
    area::content(&mut registry);
    crate::core::vfx::content(&mut registry);
    crate::core::script::bind::content(&mut registry);
    registry
        .hook(LoadStep::Across, shop::check_all)
        .hook(LoadStep::Across, resource::check_all)
        .hook(LoadStep::Across, crafting::check_all)
        .hook(LoadStep::Across, quest::check_all)
        .hook(LoadStep::Settings, command::check_all)
        .hook(LoadStep::Across, notification::check_all)
        .hook(LoadStep::Across, area::script::check_places)
        .hook(LoadStep::Across, area::object::check_watched)
        .hook(LoadStep::Across, area::sky::check_times)
        .hook(LoadStep::Across, spawner::check_heard)
        .hook(LoadStep::Across, interact::check_titled)
        .hook(LoadStep::Across, item::check_used)
        .hook(LoadStep::Across, quest::build_parties)
        .hook(LoadStep::Across, class::check_all)
        .hook(LoadStep::Across, npc::check_all)
        .hook(LoadStep::Across, skill::check_all)
        .hook(LoadStep::Across, mount::check_all)
        .hook(LoadStep::Across, emote::check_all)
        .hook(LoadStep::Across, item::check_pictures)
        .hook(LoadStep::Across, item::check_looks)
        .hook(LoadStep::Across, item::rarity::check_all)
        .hook(LoadStep::Across, stat::check_vfx)
        .hook(LoadStep::Across, combat::check_vfx)
        .hook(LoadStep::Across, status::check_vfx)
        .hook(LoadStep::Across, actor::model::check_vfx)
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
    settings.map_reaches.check("/map_reaches", &mut check);
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
    check.at_least("/drops/outline", drops.outline.0, 0.0, "pixels");
    check.at_least("/drops/outline_hover", drops.outline_hover.0, 0.0, "pixels");
    check.at_least("/drops/least_size", drops.least_size.0, 0.0, "meters");
    check.at_least("/drops/fades_for", drops.fades_for.0, 0.0, "seconds");
    check.at_least("/drops/names_reach", drops.names_reach.0, 0.0, "meters");
    check.at_least(
        "/requests/card_shows",
        settings.requests.card_shows.0,
        0.0,
        "seconds",
    );
    check.at_least("/chat/lines", settings.chat.lines, 1, "lines");
    check.at_least("/chat/within", settings.chat.within.0, 0.0, "seconds");
    check.at_least("/chat/kept_lines", settings.chat.kept_lines, 1, "lines");
    check.at_least("/search_letters", settings.search_letters, 1, "letters");
    check.at_least(
        "/chat/name_colors",
        settings.chat.name_colors.len(),
        1,
        "colors",
    );
    if let Some(friends) = &settings.friends {
        check.at_least("/friends/most", friends.most, 1, "friends");
        check.at_least("/friends/requests", friends.requests, 1, "requests");
        check.at_least(
            "/friends/request_lasts",
            friends.request_lasts.0,
            0.0,
            "seconds",
        );
    }
    if let Some(mail) = &settings.mail {
        check.at_least("/mail/inbox", mail.inbox, 1, "letters");
        check.at_least("/mail/title_length", mail.title_length, 1, "characters");
        check.at_least("/mail/text_length", mail.text_length, 1, "characters");
        check.at_least("/mail/a_minute", mail.a_minute, 1, "letters");
    }
    if let Some(party) = &settings.party {
        check.at_least("/party/size", party.size, 2, "members");
        check.at_least("/party/invite_lasts", party.invite_lasts.0, 0.0, "seconds");
        check.at_least("/party/call_lasts", party.call_lasts.0, 1.0, "seconds");
        check.at_least("/party/absent_for", party.absent_for.0, 0.0, "seconds");
        check.at_least(
            "/party/experience/range",
            party.experience.range.0,
            0.0,
            "meters",
        );
        check.at_least(
            "/party/experience/bonus",
            party.experience.bonus,
            0.0,
            "percent",
        );
    }
    if let Some(trade) = &settings.trade {
        check.at_least("/trade/reach", trade.reach.0, 0.0, "meters");
        check.at_least("/trade/slots", trade.slots, 1, "items");
        check.at_least("/trade/accept_waits", trade.accept_waits.0, 0.0, "seconds");
    }
    if let Some(market) = &settings.market {
        check.at_least("/market/durations", market.durations.len(), 1, "lengths");
        for (index, duration) in market.durations.iter().enumerate() {
            check.at_least(
                format!("/market/durations/{index}"),
                duration.0,
                1.0,
                "seconds",
            );
        }
        check.at_least(
            "/market/max_goods_items",
            market.max_goods_items,
            1,
            "items",
        );
        check.at_least(
            "/market/max_price_items",
            market.max_price_items,
            1,
            "items",
        );
        check.at_least("/market/max_listings", market.max_listings, 1, "listings");
        check.at_least("/market/page_rows", market.page_rows, 1, "rows");
    }
    settings.cards.check("/cards", &mut check);
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
    check.at_least("/notices/feed_rows", settings.notices.feed_rows, 1, "rows");
    check.at_least(
        "/notices/budget/burst",
        settings.notices.budget.burst,
        1.0,
        "notifications",
    );
    check.at_least(
        "/notices/budget/refill_per_second",
        settings.notices.budget.refill_per_second,
        f32::MIN_POSITIVE,
        "notifications a second",
    );
    check.at_least(
        "/history/merges_within",
        settings.history.merges_within.0,
        0.0,
        "seconds",
    );
    check.at_least(
        "/unsaved_told_after",
        settings.unsaved_told_after,
        1,
        "failed checkpoints",
    );
    check.at_least("/history/kept", settings.history.kept, 1, "records");
    check_look(&mut check, &settings.look);
    dialogue::camera::check(&mut check, &settings.dialogue_shots);
    for error in check.into_errors() {
        loading.json_error(SETTINGS_FILE, &error.pointer, error.problem);
    }
    area_instance::check_all(loading, &content, settings);
    item::deliver::check(loading, &content, settings);
    market::check(loading, &content, settings);
    if loading.thorough() {
        check_reach(loading, &content, settings.initial_spawn);
    }
}

fn check_typing(check: &mut RowCheck, typing: &Typing) {
    check.at_least(
        "/typing/slowest",
        typing.slowest.0,
        1.0,
        "characters a second",
    );
    check.range(
        "/typing/fastest",
        typing.fastest.0,
        typing.slowest.0,
        f32::MAX,
        "characters a second, no slower than the slowest",
    );
    check.range(
        "/typing/pace",
        typing.pace.0,
        typing.slowest.0,
        typing.fastest.0,
        "characters a second, between the slowest and the fastest",
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
fn check_reach(loading: &mut Loading, content: &Content, spawn: area::Place) {
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

fn install_spans() {
    SpanKinds::install(|kinds| {
        input::map::spans(kinds);
        item::inline::spans(kinds);
        mention::spans(kinds);
        dialogue::span::spans(kinds);
    });
}

fn install_stages() {
    Stages::install(area::stages::register);
}

fn install_vocabulary() {
    Vocabulary::install(|vocabulary| {
        area::script::scripting(vocabulary);
        spawner::scripting(vocabulary);
        npc::scripting(vocabulary);
        item::scripting(vocabulary);
        quest::scripting(vocabulary);
        shop::scripting(vocabulary);
        resource::scripting(vocabulary);
        crafting::scripting(vocabulary);
        skill::scripting(vocabulary);
        status::scripting(vocabulary);
        mount::scripting(vocabulary);
        command::scripting(vocabulary);
        chat::scripting(vocabulary);
        class::scripting(vocabulary);
        memory::scripting(vocabulary);
        prompt::scripting(vocabulary);
        notification::scripting(vocabulary);
        market::effect::scripting(vocabulary);
        combat::scripting(vocabulary);
        actor::scripting(vocabulary);
        area_instance::scripting(vocabulary);
        stat::scripting(vocabulary);
        party::scripting(vocabulary);
        equipment::scripting(vocabulary);
        roster::scripting(vocabulary);
        interact::scripting(vocabulary);
        dialogue::scripting(vocabulary);
        player::scripting(vocabulary);
    });
}

/// Starts the scripts of every owner that lives as long as a world of the game does.
pub fn spawn_scripts(world: &mut World) {
    crate::core::script::spawn_world_owners(world);
    area::script::spawn_area(world);
}
