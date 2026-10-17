use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, SendTargets, ToClients};
use bevy_replicon::server::ServerSystems;
use engine::core::content::{Content, ContentId, ContentRow, FilesystemFolder, key_of};
use engine::core::math::{Offset, Pos, Rng};
use engine::core::script::Value;
use engine::core::tiling::{TilePos, Tiles};
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::systems::TICK_HZ;
use engine::systems::area::{self, Area, AreaId, AreaObject};
use engine::systems::combat::Attitude;
use engine::systems::dialogue::{Conversation, ConversationRequest};
use engine::systems::input::map::InputMap;
use engine::systems::interact::InteractRequest;
use engine::systems::item::{self, Inventory, ItemId, ItemStack};
use engine::systems::memory::{self, MemoryId};
use engine::systems::movement::{Position, position};
use engine::systems::notification::{
    Notification, NotificationId, NotificationKind, NotificationSent,
};
use engine::systems::npc::{self, Npc, NpcId, Pack};
use engine::systems::player::snapshot::JoinAs;
use engine::systems::player::{AreaDrawn, ClientId, JoinRequest, Players, SpawnPolicy, SpawnRules};
use engine::systems::script::{Owner as ScriptOwner, OwnerKind, RiftInstances, condition};
use engine::systems::stat::{self, StatKind};
use engine::systems::{GameSettings, WorldArea, from_client, load_content, start_server};

pub const CLOCK: WallClock = WallClock {
    now: UnixMillis(0),
    reset: UtcHour::MIDNIGHT,
};

pub fn content() -> &'static Content {
    static CONTENT: LazyLock<Content> = LazyLock::new(|| {
        load_content(
            Arc::new(FilesystemFolder(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../example_game"),
            )),
            "cargo test failed",
        )
        .unwrap_or_else(|report| panic!("{report}"))
    });
    &CONTENT
}

pub trait Named: Sized {
    fn named(name: &str) -> Self;
}

impl<Row: ContentRow> Named for ContentId<Row> {
    fn named(name: &str) -> ContentId<Row> {
        content()
            .by_key::<Row>(key_of(name).as_str())
            .unwrap_or_else(|| panic!("no {} row named `{name}`", Row::TABLE))
    }
}

pub fn row<Id: Named>(name: &str) -> Id {
    Id::named(name)
}

pub fn spawn_area() -> AreaId {
    content().built::<GameSettings>().initial_spawn.area
}

pub struct Sim {
    pub app: App,
    area: AreaId,
    conns: HashMap<u32, Entity>,
}

impl Sim {
    pub fn area(area: AreaId) -> Sim {
        let mut app = engine::systems::server_app(area, 0, drawn_seed(), CLOCK, content());
        app.insert_resource(SpawnRules {
            policy: SpawnPolicy::Map,
            immortal: true,
        });
        app.init_resource::<Notified>().add_systems(
            bevy_app::PostUpdate,
            collect_notified.before(ServerSystems::Send),
        );
        start_server(&mut app);
        Sim {
            app,
            area,
            conns: HashMap::new(),
        }
    }

    pub fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

    pub fn map(&self) -> &Area {
        area::load(self.app.world().resource::<Content>(), self.area)
    }

    /// One server tick: the wall clock moves with it, as the real server's does.
    pub fn tick(&mut self) {
        let clock = *self.world().resource::<WallClock>();
        self.world().insert_resource(WallClock {
            now: clock.now.after(Seconds(1.0 / TICK_HZ.0)),
            ..clock
        });
        self.app.update();
    }

    pub fn run_until(&mut self, seconds: f32, mut done: impl FnMut(&mut World) -> bool) -> bool {
        let ticks = (seconds * TICK_HZ.0).ceil() as usize;
        for _ in 0..ticks {
            if done(self.world()) {
                return true;
            }
            self.tick();
        }
        done(self.world())
    }

    pub fn connect(&mut self, client: u32) -> Entity {
        let conn = self.world().spawn(ClientId(client)).id();
        self.conns.insert(client, conn);
        conn
    }

    pub fn join(&mut self, client: u32) -> Entity {
        self.connect(client);
        self.enter(client)
    }

    pub fn conn(&self, client: u32) -> Entity {
        self.conns[&client]
    }

    pub fn join_as(&mut self, client: u32, join_as: JoinAs) -> Entity {
        let conn = self.connect(client);
        self.world().entity_mut(conn).insert(join_as);
        self.enter(client)
    }

    pub fn notified(&self, client: u32) -> Vec<NotificationSent> {
        let conn = self.conns[&client];
        let sent = self.app.world().resource::<Notified>().0.iter().cloned();
        let unsent = self
            .read_all::<ToClients<NotificationSent>>()
            .into_iter()
            .filter_map(|message| addressee(&message).map(|to| (to, message.message)));
        sent.chain(unsent)
            .filter(|(to, _)| *to == conn)
            .map(|(_, sent)| sent)
            .collect()
    }

    pub fn forget_notified(&mut self) {
        self.world().resource_mut::<Notified>().0.clear();
    }

    pub fn send<M: Message>(&mut self, client: u32, message: M) {
        let conn = self.conns[&client];
        from_client(self.world(), conn, message);
    }

    pub fn read_all<M: Message + Clone>(&self) -> Vec<M> {
        self.read(&mut MessageCursor::default())
    }

    pub fn read<M: Message + Clone>(&self, cursor: &mut MessageCursor<M>) -> Vec<M> {
        let messages = self.app.world().resource::<Messages<M>>();
        cursor.read(messages).cloned().collect()
    }

    pub fn walkable_near(&self, from: Pos<Tiles>, min: Tiles, max: Tiles) -> Pos<Tiles> {
        let map = self.map();
        let component = map.grid.component(from);
        map.grid
            .nodes()
            .iter()
            .copied()
            .filter(|node| map.grid.component(*node) == component)
            .find(|node| (min..=max).contains(&from.distance(*node)))
            .expect("a reachable walkable tile in range")
    }

    /// The walkable tile nearest to `from` moved by `offset`, in the same walkable region.
    pub fn walkable_toward(&self, from: Pos<Tiles>, offset: Offset<Tiles>) -> Pos<Tiles> {
        let map = self.map();
        let component = map.grid.component(from);
        let target = from + offset;
        map.grid
            .nodes()
            .iter()
            .copied()
            .filter(|node| map.grid.component(*node) == component)
            .min_by(|a, b| {
                target
                    .distance(*a)
                    .partial_cmp(&target.distance(*b))
                    .expect("finite")
            })
            .expect("a reachable walkable tile")
    }

    fn enter(&mut self, client: u32) -> Entity {
        self.send(client, JoinRequest);
        self.tick();
        self.send(client, AreaDrawn);
        *self
            .world()
            .resource::<Players>()
            .0
            .get(&ClientId(client))
            .expect("joined player")
    }
}

pub fn townsperson(sim: &mut Sim, who: NpcId) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Npc)>()
        .iter(world)
        .find(|(_, npc)| npc.def == who)
        .map(|(entity, _)| entity)
        .expect("a resident")
}

pub fn object(sim: &mut Sim, name: &str) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &AreaObject)>()
        .iter(world)
        .find(|(_, object)| object.name.as_deref() == Some(name))
        .map(|(entity, _)| entity)
        .expect("an object on the map")
}

pub fn object_center(sim: &mut Sim, name: &str) -> Pos<Tiles> {
    let entity = object(sim, name);
    sim.world()
        .get::<AreaObject>(entity)
        .expect("an area object")
        .shape
        .center()
}

pub fn remember(sim: &mut Sim, player: Entity, key: MemoryId) {
    memory::set(sim.world(), player, key, Value::Bool(true));
}

pub fn remembered(sim: &mut Sim, player: Entity, key: MemoryId) -> bool {
    memory::get(sim.world(), player, key).truthy()
}

pub fn counted(sim: &mut Sim, player: Entity, key: MemoryId) -> i64 {
    match memory::get(sim.world(), player, key) {
        Value::Int(count) => count,
        other => panic!("{} holds {other:?}, not a count", key.key(content())),
    }
}

pub fn open_with(sim: &mut Sim, client: u32, player: Entity, target: Entity) -> Conversation {
    sim.send(client, InteractRequest { target });
    assert!(
        sim.run_until(20.0, |world| world.get::<Conversation>(player).is_some()),
        "the conversation never opened"
    );
    conversation(sim, player).expect("open")
}

pub fn talk(sim: &mut Sim, client: u32, player: Entity, who: NpcId) -> Conversation {
    let npc = townsperson(sim, who);
    let opened = open_with(sim, client, player, npc);
    assert_eq!(opened.with, Some(npc), "someone else spoke first");
    opened
}

pub fn heard_the_news(sim: &mut Sim, player: Entity) {
    remember(sim, player, row("TobbNewsToday"));
}

pub fn conversation(sim: &mut Sim, player: Entity) -> Option<Conversation> {
    sim.world().get::<Conversation>(player).cloned()
}

pub fn labels(conversation: &Conversation) -> Vec<String> {
    conversation
        .choices
        .iter()
        .map(|choice| choice.label.words(&InputMap::new(content())))
        .collect()
}

pub fn pick(sim: &mut Sim, client: u32, player: Entity, label: &str) -> Option<Conversation> {
    let now = conversation(sim, player).expect("talking");
    sim.send(
        client,
        ConversationRequest::Pick {
            step: now.step,
            choice: choice(&now, label),
        },
    );
    sim.tick();
    conversation(sim, player)
}

pub fn choice(conversation: &Conversation, label: &str) -> u32 {
    conversation
        .choices
        .iter()
        .position(|choice| choice.label.words(&InputMap::new(content())) == label)
        .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", labels(conversation))) as u32
}

pub fn locked(conversation: &Conversation, label: &str) -> bool {
    conversation.choices[choice(conversation, label) as usize].locked
}

pub fn told(sim: &Sim, client: u32) -> Vec<Notification> {
    sim.notified(client)
        .into_iter()
        .map(|sent| sent.notification)
        .collect()
}

pub fn errors(sim: &Sim, client: u32) -> Vec<String> {
    sim.notified(client)
        .into_iter()
        .filter(|sent| matches!(sent.notification.kind, NotificationKind::Error { .. }))
        .map(|sent| sent.notification.text.words(&InputMap::new(content())))
        .collect()
}

pub fn leave(sim: &mut Sim, client: u32, player: Entity) {
    if let Some(now) = conversation(sim, player) {
        sim.send(client, ConversationRequest::Leave { step: now.step });
        sim.tick();
    }
}

pub fn give(sim: &mut Sim, player: Entity, item: ItemId, count: u32) {
    sim.world()
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(content(), &[], &[ItemStack::new(item, count)])
        .expect("room");
}

pub fn count(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .count(item)
}

pub fn slot_of(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .slots
        .iter()
        .position(|stack| stack.item == item)
        .expect("in the bag") as u32
}

pub fn health(sim: &mut Sim, who: Entity) -> f32 {
    stat::effective(sim.world(), who, StatKind::Health)
}

pub fn place(sim: &mut Sim, entity: Entity, at: Pos<Tiles>) {
    sim.world().entity_mut(entity).insert(Position::at(at));
}

pub fn spoken(id: NotificationId) -> Notification {
    let def = id.get(content());
    def.notification(condition::spoken(
        &World::new(),
        Entity::PLACEHOLDER,
        &def.text,
    ))
}

pub fn settle(sim: &mut Sim) {
    sim.run_until(0.5, |_| false);
}

pub fn later(sim: &mut Sim, seconds: f32) {
    let clock = *sim.world().resource::<WallClock>();
    sim.world().insert_resource(WallClock {
        now: clock.now.after(Seconds(seconds)),
        ..clock
    });
    settle(sim);
}

/// A hostile npc that neither wanders nor fights back: its scripts are dropped at spawn, so a
/// test can count on where it stands.
pub fn target_dummy(sim: &mut Sim, what: NpcId, at: Pos<Tiles>) -> Entity {
    let world = sim.world();
    let area = world.resource::<WorldArea>().0;
    let dummy = npc::spawn(world, what, at, area, Pack(u32::MAX));
    world
        .resource_mut::<RiftInstances>()
        .remove_owner(ScriptOwner::Entity(OwnerKind::Npc, dummy));
    world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.tick();
    dummy
}

pub fn slay(sim: &mut Sim, player: Entity, what: NpcId) {
    let world = sim.world();
    let at = position(world, player).expect("player position");
    let area = world.resource::<WorldArea>().0;
    let victim = npc::spawn(world, what, at, area, Pack(u32::MAX));
    item::reserve(world, victim, player, Seconds(0.0));
    engine::systems::combat::inflict(world, victim, Some(player), 1_000_000.0);
    sim.run_until(0.2, |_| false);
}

// Tests hold on any seed: the one drawn shows in a failing test's output, to play it out again.
pub fn drawn_seed() -> u64 {
    let seed = Rng::random_seed();
    eprintln!("rng seed {seed}");
    seed
}

#[derive(Resource, Default)]
struct Notified(Vec<(Entity, NotificationSent)>);

fn collect_notified(
    mut sent: MessageReader<ToClients<NotificationSent>>,
    mut notified: ResMut<Notified>,
) {
    for message in sent.read() {
        if let Some(conn) = addressee(message) {
            notified.0.push((conn, message.message.clone()));
        }
    }
}

fn addressee(message: &ToClients<NotificationSent>) -> Option<Entity> {
    match message.targets {
        SendTargets::Single(Sender::Client(conn)) => Some(conn),
        _ => None,
    }
}
