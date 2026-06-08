//! The in-process stage: a real `Cluster` over the wire protocol — the standalone game, minus
//! sockets.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rift::{Client, ClientId, Cluster, Feature, Wire};
use world::core::area;
use world::core::math::{Pos, Tiles};
use world::features::{
    actions, combat, movement, player, regen, replication, spectate, visibility,
};
use world::{
    AttackRequest, Entity, Identity, JoinRequest, MoveRequest, MoveToPortal, RespawnRequest,
    SPECTATE_ROLE, SpectateRequest, UseItemRequest,
};

use crate::{Player, Stage, View};

pub struct SimStage {
    core: Rc<RefCell<Core>>,
}

impl SimStage {
    pub fn new(features: &[Feature]) -> Self {
        Self {
            core: Rc::new(RefCell::new(Core {
                cluster: Cluster::new(features, &world::zones(), world::spawn_zone()),
                clients: HashMap::new(),
                next: 1,
                dt: 1.0 / world::TICK_HZ,
            })),
        }
    }

    pub fn full() -> Self {
        Self::new(&world::features())
    }

    /// Minus NPCs: deterministic and interference-free; player behavior is identical.
    pub fn players_only() -> Self {
        Self::new(&[
            replication::feature,
            actions::feature,
            regen::feature,
            movement::input,
            combat::feature,
            movement::step,
            player::feature,
            spectate::feature,
            visibility::feature,
        ])
    }

    pub fn connect(&mut self, roles: &[&str]) -> SimPlayer {
        let id = {
            let mut core = self.core.borrow_mut();
            let id = ClientId(core.next);
            core.next += 1;
            let identity = Identity {
                id: format!("user-{}", id.0),
                name: format!("user-{}", id.0),
                roles: roles.iter().map(|role| (*role).to_owned()).collect(),
            };
            core.cluster.connect_as(id, Some(Arc::new(identity)));
            core.clients.insert(id, Client::new());
            core.tick();
            id
        };
        SimPlayer {
            core: Rc::clone(&self.core),
            id,
        }
    }
}

impl Stage for SimStage {
    fn player(&mut self) -> Box<dyn Player> {
        let mut player = self.connect(&[]);
        player.join();
        assert!(
            crate::eventually(self, 5.0, || player.view().me().is_some()),
            "a joining player must spawn"
        );
        Box::new(player)
    }
    fn spectator(&mut self) -> Box<dyn Player> {
        let mut player = self.connect(&[SPECTATE_ROLE]);
        player.spectate(None);
        assert!(
            crate::eventually(self, 5.0, || player.view().spectating),
            "an entitled spectator must be admitted"
        );
        Box::new(player)
    }
    fn unentitled_spectator(&mut self) -> Box<dyn Player> {
        let mut player = self.connect(&[]);
        player.spectate(None);
        self.step(0.2);
        Box::new(player)
    }
    fn step(&mut self, seconds: f32) {
        let mut core = self.core.borrow_mut();
        let ticks = ((seconds / core.dt).round() as usize).max(1);
        for _ in 0..ticks {
            core.tick();
        }
    }
}

impl SimPlayer {
    pub fn join(&mut self) {
        self.core.borrow_mut().send(self.id, &JoinRequest {});
    }
    pub fn spectate(&mut self, watch: Option<u32>) {
        self.core.borrow_mut().send(
            self.id,
            &SpectateRequest {
                watch: watch.map(ClientId),
            },
        );
    }
}

struct Core {
    cluster: Cluster,
    clients: HashMap<ClientId, Client>,
    next: u32,
    dt: f32,
}

impl Core {
    fn tick(&mut self) {
        let dt = self.dt;
        for (id, packet) in self.cluster.tick(dt) {
            if let Some(client) = self.clients.get_mut(&id) {
                client.receive(&packet);
            }
        }
    }
    fn send<E: Wire + 'static>(&mut self, id: ClientId, event: &E) {
        if let Some(client) = self.clients.get_mut(&id) {
            let packet = client.send(event);
            self.cluster.receive(id, &packet);
        }
    }
}

pub struct SimPlayer {
    core: Rc<RefCell<Core>>,
    id: ClientId,
}

impl SimPlayer {
    /// For contracts no real client expresses (e.g. a plain move onto a portal tile).
    pub fn send<E: Wire + 'static>(&mut self, event: &E) {
        self.core.borrow_mut().send(self.id, event);
    }
}

impl Player for SimPlayer {
    fn view(&mut self) -> View {
        let core = self.core.borrow();
        match core.clients.get(&self.id) {
            Some(client) => crate::view_of(client),
            None => View::default(),
        }
    }
    fn move_to(&mut self, x: f32, y: f32) {
        let area = self
            .view()
            .me()
            .and_then(|me| me.area)
            .unwrap_or(world::core::area::AreaId(0));
        let portal = area::areas()
            .get(area.0 as usize)
            .map(|area| &area.portals)
            .and_then(|portals| {
                portals
                    .iter()
                    .position(|portal| portal.rect.contains(Pos::new(Tiles(x), Tiles(y))))
            });
        let mut core = self.core.borrow_mut();
        match portal {
            Some(index) => core.send(
                self.id,
                &MoveToPortal {
                    pos: Pos::new(Tiles(x), Tiles(y)),
                    portal: index as u32,
                },
            ),
            None => core.send(
                self.id,
                &MoveRequest {
                    pos: Pos::new(Tiles(x), Tiles(y)),
                },
            ),
        }
    }
    fn attack(&mut self, entity: u32) {
        self.core.borrow_mut().send(
            self.id,
            &AttackRequest {
                target: Entity(entity),
            },
        );
    }
    fn respawn(&mut self) {
        self.core.borrow_mut().send(self.id, &RespawnRequest {});
    }
    fn watch(&mut self, owner: u32) {
        self.spectate(Some(owner));
    }
    fn use_item(&mut self, slot: u32) {
        self.core
            .borrow_mut()
            .send(self.id, &UseItemRequest { slot });
    }
}

impl Drop for SimPlayer {
    fn drop(&mut self) {
        let mut core = self.core.borrow_mut();
        core.cluster.disconnect(self.id);
        core.clients.remove(&self.id);
        core.tick();
    }
}
