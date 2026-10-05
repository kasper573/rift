use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, FromClient, ServerState};
use bevy_state::prelude::NextState;
use bevy_time::TimeUpdateStrategy;
use game::core::assets::{AssetService, FilesystemSource};
use game::core::math::{Pos, Rng};
use game::core::tiling::{TilePos, Tiles};
use game::core::time::{UnixMillis, UtcHour, WallClock};
use game::data;
use game::systems::TICK_HZ;
use game::systems::area::{self, Area};
use game::systems::player::{ClientId, Immortal, JoinRequest, Players};

pub const CLOCK: WallClock = WallClock {
    now: UnixMillis(0),
    reset: UtcHour::MIDNIGHT,
};

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

    pub fn join(&mut self, client: u32) -> Entity {
        let conn = self.world().spawn(ClientId(client)).id();
        self.conns.insert(client, conn);
        self.send(client, JoinRequest);
        self.tick();
        *self
            .world()
            .resource::<Players>()
            .0
            .get(&ClientId(client))
            .expect("joined player")
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
        map.walkable_nodes
            .iter()
            .copied()
            .filter(|node| map.grid.component(*node) == component)
            .find(|node| (min..=max).contains(&from.distance(*node)))
            .expect("a reachable walkable tile in range")
    }
}
