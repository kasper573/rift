//! The client-side session: a bevy_replicon client app behind a pollable facade, fed by a
//! byte-frame [`Transport`]. Each frame on the wire is one replicon message prefixed with its
//! channel id byte.

use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::bytes::Bytes;
use bevy_replicon::prelude::{
    AuthMethod, ClientMessages, ClientState, RepliconPlugins, RepliconSharedPlugin,
};
use bevy_state::prelude::NextState;

use crate::core::area::{self, AreaId};
use crate::core::math::{Pos, Tiles};
use crate::core::protocol::{
    self, Actor, AreaTag, AttackRequest, ClientId, Inventory, ItemId, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Spectate, SpectateRequest, UseItemRequest,
    Vitals, Welcome, Xp,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkStatus {
    Connecting,
    Open,
    Closed,
}

/// A byte-frame pipe to the server; the wasm build bridges a JS WebSocket plugin, the native
/// build wraps [`WsTransport`].
pub trait Transport {
    fn send(&mut self, packet: &[u8]);
    fn poll(&mut self, sink: &mut dyn FnMut(&[u8]));
    fn status(&self) -> LinkStatus;
}

pub struct MmoClient {
    app: App,
    transport: Box<dyn Transport>,
    id: Option<ClientId>,
    opened: bool,
}

impl MmoClient {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(address: &str, token: &str) -> std::io::Result<MmoClient> {
        Ok(MmoClient::with_transport(Box::new(WsTransport::connect(
            address, token,
        )?)))
    }

    // No TimePlugin: nothing in the client-side plugins reads `Time`, and the std `Instant`
    // behind it has no clock on wasm32-unknown-unknown.
    pub fn with_transport(transport: Box<dyn Transport>) -> MmoClient {
        let mut app = App::new();
        app.add_plugins(bevy_state::app::StatesPlugin);
        let plugins = bevy_app::PluginGroup::build(RepliconPlugins).set(RepliconSharedPlugin {
            auth_method: AuthMethod::None,
        });
        #[cfg(feature = "host")]
        let plugins = plugins
            .disable::<bevy_replicon::server::ServerPlugin>()
            .disable::<bevy_replicon::server::message::ServerMessagePlugin>();
        app.add_plugins(plugins);
        protocol::protocol(&mut app);
        app.finish();
        MmoClient {
            app,
            transport,
            id: None,
            opened: false,
        }
    }

    /// Pumps one frame: transport bytes in, one app update, queued messages out.
    pub fn poll(&mut self) {
        match (self.transport.status(), self.opened) {
            (LinkStatus::Open, false) => {
                self.set_state(ClientState::Connected);
                self.opened = true;
            }
            (LinkStatus::Closed, true) => {
                self.set_state(ClientState::Disconnected);
                self.opened = false;
            }
            _ => {}
        }
        let mut messages = self.app.world_mut().resource_mut::<ClientMessages>();
        self.transport.poll(&mut |frame| {
            if let Some((&channel, payload)) = frame.split_first() {
                messages.insert_received(channel as usize, Bytes::copy_from_slice(payload));
            }
        });
        self.app.update();
        if let Some(welcome) = self.drain::<Welcome>().pop() {
            self.id = Some(welcome.id);
        }
        if self.transport.status() == LinkStatus::Open {
            let frames: Vec<(usize, Bytes)> = self
                .app
                .world_mut()
                .resource_mut::<ClientMessages>()
                .drain_sent()
                .collect();
            for (channel, payload) in frames {
                let mut frame = Vec::with_capacity(payload.len() + 1);
                frame.push(u8::try_from(channel).expect("replicon channels fit a byte"));
                frame.extend_from_slice(&payload);
                self.transport.send(&frame);
            }
        }
    }

    pub fn status(&self) -> LinkStatus {
        self.transport.status()
    }

    /// The id the server greeted this session with; `None` until the welcome arrives.
    pub fn id(&self) -> Option<ClientId> {
        self.id
    }

    pub fn world(&self) -> &World {
        self.app.world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.app.world_mut()
    }

    pub fn join(&mut self) {
        self.app.world_mut().write_message(JoinRequest);
    }
    pub fn spectate(&mut self, watch: Option<ClientId>) {
        self.app
            .world_mut()
            .write_message(SpectateRequest { watch });
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        let portal = self.my_area().and_then(|area| {
            area::areas()
                .get(area.0 as usize)?
                .portals
                .iter()
                .position(|portal| portal.rect.contains(Pos::new(x, y)))
        });
        let world = self.app.world_mut();
        match portal {
            Some(index) => {
                world.write_message(MoveToPortal {
                    pos: Pos::new(x, y),
                    portal: index as u32,
                });
            }
            None => {
                world.write_message(MoveRequest {
                    pos: Pos::new(x, y),
                });
            }
        }
    }
    pub fn attack(&mut self, target: Entity) {
        self.app.world_mut().write_message(AttackRequest { target });
    }
    pub fn respawn(&mut self) {
        self.app.world_mut().write_message(RespawnRequest);
    }
    pub fn use_item(&mut self, slot: u32) {
        self.app.world_mut().write_message(UseItemRequest { slot });
    }

    /// Server announcements of the given type received since the last drain.
    pub fn drain<M: Message>(&mut self) -> Vec<M> {
        self.app
            .world_mut()
            .resource_mut::<Messages<M>>()
            .drain()
            .collect()
    }

    pub fn my_entity(&mut self) -> Option<Entity> {
        let me = self.id?;
        let world = self.app.world_mut();
        let mut owners = world.query::<(Entity, &Owner)>();
        owners
            .iter(world)
            .find(|(_, owner)| owner.client == me)
            .map(|(entity, _)| entity)
    }

    pub fn my_position(&mut self) -> Option<Pos<Tiles>> {
        let entity = self.my_entity()?;
        self.world().get::<Position>(entity).map(|p| p.pos)
    }

    pub fn my_health(&mut self) -> Option<f32> {
        let entity = self.my_entity()?;
        self.world().get::<Vitals>(entity).map(|v| v.health)
    }

    pub fn my_xp(&mut self) -> Option<u32> {
        let entity = self.my_entity()?;
        self.world().get::<Xp>(entity).map(|xp| xp.amount)
    }

    pub fn my_inventory(&mut self) -> Vec<ItemId> {
        self.my_entity()
            .and_then(|entity| self.world().get::<Inventory>(entity))
            .map_or_else(Vec::new, |inventory| inventory.items.clone())
    }

    pub fn is_dead(&mut self) -> bool {
        self.my_health().is_some_and(|health| health <= 0.0)
    }

    pub fn my_area(&mut self) -> Option<AreaId> {
        let entity = self.my_entity()?;
        self.world().get::<AreaTag>(entity).map(|tag| tag.area)
    }

    pub fn is_spectating(&mut self) -> bool {
        self.my_entity()
            .is_some_and(|entity| self.world().get::<Spectate>(entity).is_some())
    }

    pub fn watching(&mut self) -> Option<ClientId> {
        let entity = self.my_entity()?;
        self.world().get::<Spectate>(entity)?.watch
    }

    pub fn players(&mut self) -> Vec<(ClientId, String)> {
        let me = self.id;
        let world = self.app.world_mut();
        let mut owners = world.query::<(&Owner, Option<&Name>, Has<Actor>)>();
        let mut players: Vec<(ClientId, String)> = owners
            .iter(world)
            .filter(|(owner, _, actor)| Some(owner.client) != me && *actor)
            .map(|(owner, name, _)| {
                (
                    owner.client,
                    name.map_or_else(String::new, |n| n.name.clone()),
                )
            })
            .collect();
        players.sort_unstable_by_key(|(id, _)| *id);
        players
    }

    fn set_state(&mut self, state: ClientState) {
        self.app
            .world_mut()
            .resource_mut::<NextState<ClientState>>()
            .set(state);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::WsTransport;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::ErrorKind;
    use std::net::TcpStream;

    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    use super::{LinkStatus, Transport};

    /// A non-blocking native WebSocket: the handshake is synchronous, then reads poll.
    pub struct WsTransport {
        socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    }

    impl WsTransport {
        pub fn connect(address: &str, token: &str) -> std::io::Result<WsTransport> {
            let url = if token.is_empty() {
                format!("ws://{address}/ws")
            } else {
                format!("ws://{address}/ws?accessToken={token}")
            };
            let (mut socket, _) = tungstenite::connect(&url)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
                stream.set_nonblocking(true)?;
            }
            Ok(WsTransport {
                socket: Some(socket),
            })
        }
    }

    impl Transport for WsTransport {
        fn send(&mut self, packet: &[u8]) {
            let Some(socket) = &mut self.socket else {
                return;
            };
            match socket.send(Message::binary(packet.to_vec())) {
                Ok(()) => {}
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => self.socket = None,
            }
        }

        fn poll(&mut self, sink: &mut dyn FnMut(&[u8])) {
            let Some(socket) = &mut self.socket else {
                return;
            };
            match socket.flush() {
                Ok(())
                | Err(tungstenite::Error::Io(_))
                | Err(tungstenite::Error::WriteBufferFull(_)) => {}
                Err(_) => {
                    self.socket = None;
                    return;
                }
            }
            loop {
                match socket.read() {
                    Ok(Message::Binary(bytes)) => sink(&bytes),
                    Ok(Message::Close(_)) => {
                        self.socket = None;
                        return;
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                        return;
                    }
                    Err(_) => {
                        self.socket = None;
                        return;
                    }
                }
            }
        }

        fn status(&self) -> LinkStatus {
            match self.socket {
                Some(_) => LinkStatus::Open,
                None => LinkStatus::Closed,
            }
        }
    }
}
