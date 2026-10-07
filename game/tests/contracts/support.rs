use std::collections::HashMap;
use std::str::FromStr;

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, FromClient, SendTargets, ServerState, ToClients};
use bevy_replicon::server::ServerSystems;
use bevy_state::prelude::NextState;
use bevy_time::TimeUpdateStrategy;
use game::core::assets::{AssetService, FilesystemSource};
use game::core::math::{Pos, Rng};
use game::core::tiling::{TilePos, Tiles};
use game::core::time::Seconds;
use game::core::time::{UnixMillis, UtcHour, WallClock};
use game::data;
use game::data::item::Id as ItemId;
use game::data::notification::Id as NotificationId;
use game::data::npc::Id as NpcId;
use game::data::prop::Id as PropId;
use game::systems::TICK_HZ;
use game::systems::area::{self, Area};
use game::systems::combat::Died;
use game::systems::dialogue::{Conversation, ConversationRequest};
use game::systems::input::map::InputMap;
use game::systems::interact::InteractRequest;
use game::systems::item::{self, Inventory, ItemStack};
use game::systems::memory::Memory;
use game::systems::movement::position;
use game::systems::notification::{Notification, NotificationKind, NotificationSent};
use game::systems::npc::{self, Npc, Pack};
use game::systems::player::{ClientId, Immortal, JoinRequest, Players};
use game::systems::prop::Prop;

pub const CLOCK: WallClock = WallClock {
    now: UnixMillis(0),
    reset: UtcHour::MIDNIGHT,
};

pub fn row<Id: FromStr>(name: &str) -> Id {
    name.parse()
        .unwrap_or_else(|_| panic!("no row named `{name}`"))
}

pub fn assets() -> AssetService {
    AssetService::new(FilesystemSource(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets"),
    ))
}

pub struct Sim {
    pub app: App,
    area: data::area::Id,
    conns: HashMap<u32, Entity>,
}

impl Sim {
    pub fn area(area: data::area::Id) -> Sim {
        let mut app = game::systems::server_app(area, 0, CLOCK);
        app.insert_resource(assets());
        app.insert_resource(Rng::new(1));
        app.insert_resource(Immortal(true));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(TICK_HZ.period()));
        app.init_resource::<Notified>().add_systems(
            bevy_app::PostUpdate,
            collect_notified.before(ServerSystems::Send),
        );
        app.finish();
        app.cleanup();
        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();
        Sim {
            app,
            area,
            conns: HashMap::new(),
        }
    }

    pub fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

    pub fn map(&self) -> &'static Area {
        let assets = self.app.world().resource::<AssetService>().clone();
        assets.resolve(self.area.get().map, area::build_area)
    }

    pub fn tick(&mut self) {
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
        self.send(client, JoinRequest);
        self.tick();
        *self
            .world()
            .resource::<Players>()
            .0
            .get(&ClientId(client))
            .expect("joined player")
    }

    pub fn conn(&self, client: u32) -> Entity {
        self.conns[&client]
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
        self.world().write_message(FromClient {
            client_id: Sender::Client(conn),
            message,
        });
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

pub fn prop(sim: &mut Sim, which: PropId) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Prop)>()
        .iter(world)
        .find(|(_, prop)| prop.def == which)
        .map(|(entity, _)| entity)
        .expect("a fixture on the map")
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
    let clock = *sim.world().resource::<WallClock>();
    sim.world()
        .get_mut::<Memory>(player)
        .expect("memory")
        .remember(row("TobbNewsToday"), clock);
}

pub fn conversation(sim: &mut Sim, player: Entity) -> Option<Conversation> {
    sim.world().get::<Conversation>(player).cloned()
}

pub fn labels(conversation: &Conversation) -> Vec<String> {
    conversation
        .choices
        .iter()
        .map(|choice| choice.label.words(&InputMap::default()))
        .collect()
}

pub fn pick(sim: &mut Sim, client: u32, player: Entity, label: &str) -> Option<Conversation> {
    let now = conversation(sim, player).expect("talking");
    let choice =
        now.choices
            .iter()
            .position(|choice| choice.label.words(&InputMap::default()) == label)
            .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", labels(&now))) as u32;
    sim.send(
        client,
        ConversationRequest::Pick {
            step: now.step,
            choice,
        },
    );
    sim.tick();
    conversation(sim, player)
}

pub fn locked(conversation: &Conversation, label: &str) -> bool {
    conversation
        .choices
        .iter()
        .find(|choice| choice.label.words(&InputMap::default()) == label)
        .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", labels(conversation)))
        .locked
}

pub fn errors(sim: &Sim, client: u32) -> Vec<String> {
    sim.notified(client)
        .into_iter()
        .filter(|sent| matches!(sent.notification.kind, NotificationKind::Error { .. }))
        .map(|sent| sent.notification.text.words(&InputMap::default()))
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
        .exchange(&[], &[ItemStack::new(item, count)])
        .expect("room");
}

pub fn count(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .count(item)
}

pub fn spoken(id: NotificationId) -> Notification {
    id.get().for_player(&World::new(), Entity::PLACEHOLDER)
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

pub fn slay(sim: &mut Sim, player: Entity, what: NpcId) {
    let world = sim.world();
    let at = position(world, player).expect("player position");
    let area = world.resource::<game::systems::WorldArea>().0;
    let victim = npc::spawn(world, what, at, area, Pack(u32::MAX));
    item::reserve(world, victim, player, Seconds(0.0));
    world.write_message(Died {
        entity: victim,
        killer: player,
    });
    sim.run_until(0.2, |_| false);
}
