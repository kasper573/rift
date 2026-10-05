use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, FromClient, ServerState};
use bevy_state::prelude::NextState;
use game::core::assets::{AssetService, FilesystemSource};
use game::core::math::Rng;
use game::core::time::{UnixMillis, UtcHour, WallClock};
use game::data;
use game::systems::player::{ClientId, JoinRequest, Players};

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
}

impl Sim {
    pub fn area(area: data::area::Id) -> Sim {
        let mut app = game::systems::server_app(area, 0, CLOCK);
        app.insert_resource(assets());
        app.insert_resource(Rng::new(1));
        app.finish();
        app.cleanup();
        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();
        Sim { app }
    }

    pub fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

    pub fn tick(&mut self) {
        self.app.update();
    }

    pub fn join(&mut self, client: u32) -> Entity {
        let client = ClientId(client);
        let world = self.world();
        let conn = world.spawn(client).id();
        world.write_message(FromClient {
            client_id: Sender::Client(conn),
            message: JoinRequest,
        });
        self.tick();
        *self
            .world()
            .resource::<Players>()
            .0
            .get(&client)
            .expect("joined player")
    }

    pub fn read_all<M: Message + Clone>(&self) -> Vec<M> {
        self.read(&mut MessageCursor::default())
    }

    pub fn read<M: Message + Clone>(&self, cursor: &mut MessageCursor<M>) -> Vec<M> {
        let messages = self.app.world().resource::<Messages<M>>();
        cursor.read(messages).cloned().collect()
    }
}
