use std::collections::HashMap;

use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replication::FromClient;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::area::transition::{self, Crossing};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::npc::{self, Pack};
use engine::systems::player::snapshot::JoinAs;
use engine::systems::player::{
    self, AreaDrawn, ClientId, JoinRequest, Players, SpawnPolicy, SpawnRules,
};
use engine::systems::{GameSettings, WorldArea, from_client, worlds};
use engine::systems::{identity, region};

use crate::Sim;

#[derive(Resource, Default)]
struct Packs(HashMap<u32, Pack>);

/// Has the servers of several areas confer, as a game server's worlds do between ticks: each
/// learns who plays in the others, and what one asked be done to a player of another is done.
pub fn confer(sims: &mut [&mut Sim]) {
    let mut views: Vec<Option<&mut World>> =
        sims.iter_mut().map(|sim| Some(&mut sim.world)).collect();
    worlds::confer(&mut views);
}

impl Sim {
    /// Makes this world the server of the area under `key`, ready for players of the game's
    /// settings to join it.
    pub fn serve(&mut self, key: &str) -> &mut Sim {
        let area = self.id::<AreaDef>(key);
        let settings = self
            .world
            .resource::<Content>()
            .built::<GameSettings>()
            .clone();
        let world = &mut self.world;
        world.insert_resource(region::Region::of(world.resource::<Content>()));
        world.insert_resource(settings);
        world.insert_resource(WorldArea(area));
        world.insert_resource(identity::Names::for_world(0));
        world.insert_resource(SpawnRules {
            policy: SpawnPolicy::Map,
            immortal: true,
        });
        self
    }

    /// The pack a test labels `label`, drawn from the world's names the first time it's named, so
    /// npcs spawned under one label stand together.
    pub fn pack(&mut self, label: u32) -> Pack {
        if let Some(&pack) = self.world.get_resource_or_init::<Packs>().0.get(&label) {
            return pack;
        }
        let pack = npc::next_pack(&mut self.world);
        self.world.resource_mut::<Packs>().0.insert(label, pack);
        pack
    }

    /// A client connected, joined, and drew the area it arrived in, the way anyone joins: its
    /// character, ready to take commands.
    pub fn join(&mut self, client: u32) -> Entity {
        let conn = self.connect(client);
        self.enter(client, conn)
    }

    /// A client joined as a character it brought along, the way a server hands one over to
    /// another: that character.
    pub fn join_as(&mut self, client: u32, join_as: JoinAs) -> Entity {
        let conn = self.connect(client);
        self.world.entity_mut(conn).insert(join_as);
        self.enter(client, conn)
    }

    pub fn connect(&mut self, client: u32) -> Entity {
        self.world.spawn(ClientId(client)).id()
    }

    /// Lets clients send `M`, so a system answering it may run before any client has.
    pub fn accept<M: Message>(&mut self) -> &mut Sim {
        self.message::<FromClient<M>>()
    }

    /// Writes `message` as sent by the client on `conn`, for the system that answers it.
    pub fn send<M: Message>(&mut self, conn: Entity, message: M) {
        self.accept::<M>();
        from_client(&mut self.world, conn, message);
    }

    /// Hands `player` over from this area's server to `to`'s, the way a crossing does, landing at
    /// `at` and drawing the area there: the character as it arrived.
    pub fn cross(&mut self, player: Entity, to: &mut Sim, at: Pos<Meters>) -> Entity {
        let dest_area = to.world.resource::<WorldArea>().0;
        self.world.entity_mut(player).insert(Crossing {
            dest_area,
            dest: at,
        });
        self.hand_over(to)
    }

    /// Hands the player on their way out of this area over to `to`'s server, the area they are
    /// bound for, where they draw what they arrive to: the character as it arrived.
    pub fn hand_over(&mut self, to: &mut Sim) -> Entity {
        let traveler = transition::departing(&mut self.world)
            .pop()
            .expect("a traveler");
        assert_eq!(
            traveler.dest_area,
            to.world.resource::<WorldArea>().0,
            "the traveler is bound for the area they are handed to"
        );
        let client = traveler.client;
        to.connect(client.0);
        let arrived = transition::arrive(&mut to.world, traveler);
        let conn = to.conn(client.0);
        to.send(conn, AreaDrawn);
        arrived
    }

    pub fn player(&self, client: u32) -> Entity {
        self.world.resource::<Players>().0[&ClientId(client)]
    }

    /// Whether `player` remembers the boolean memory under `key`.
    pub fn remembers(&self, player: Entity, key: &str) -> bool {
        memory::get(&self.world, player, self.id::<MemoryDef>(key)).truthy()
    }

    pub fn conn(&mut self, client: u32) -> Entity {
        let mut conns = self.world.query::<(Entity, &ClientId)>();
        conns
            .iter(&self.world)
            .find(|(_, id)| id.0 == client)
            .map(|(conn, _)| conn)
            .expect("a connected client")
    }

    fn enter(&mut self, client: u32, conn: Entity) -> Entity {
        self.send(conn, JoinRequest);
        player::join(&mut self.world);
        self.send(conn, AreaDrawn);
        player::arrival_holds(&mut self.world);
        self.player(client)
    }
}
