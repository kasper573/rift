use rift::{ClientId, Entity, Link, LinkStatus, Transport, World};

use crate::core::area::{self, AreaId};
use crate::core::math::{Pos, Tiles};
use crate::core::protocol::{
    Actor, AreaTag, Inventory, ItemId, Name, Owner, Position, Spectate, UseItemRequest, Vitals, Xp,
};
use crate::features::combat::AttackRequest;
use crate::features::movement::{MoveRequest, MoveToPortal};
use crate::features::player::{JoinRequest, RespawnRequest};
use crate::features::spectate::SpectateRequest;

pub struct MmoClient {
    pub link: Link,
}

impl MmoClient {
    pub fn connect(address: &str, token: &str) -> std::io::Result<Self> {
        Ok(Self {
            link: Link::tcp(address, token)?,
        })
    }

    pub fn with_transport(transport: Box<dyn Transport>) -> Self {
        Self {
            link: Link::new(transport),
        }
    }

    pub fn poll(&mut self) {
        self.link.poll();
    }

    pub fn status(&self) -> LinkStatus {
        self.link.status()
    }

    pub fn world(&self) -> &World {
        &self.link.client.world
    }

    pub fn join(&mut self) {
        self.link.send(&JoinRequest {});
    }
    pub fn spectate(&mut self, watch: Option<ClientId>) {
        self.link.send(&SpectateRequest { watch });
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        let portal = self.my_area().and_then(|area| {
            area::areas()
                .get(area.0 as usize)?
                .portals
                .iter()
                .position(|portal| portal.rect.contains(Pos::new(Tiles(x), Tiles(y))))
        });
        match portal {
            Some(index) => self.link.send(&MoveToPortal {
                pos: Pos::new(Tiles(x), Tiles(y)),
                portal: index as u32,
            }),
            None => self.link.send(&MoveRequest {
                pos: Pos::new(Tiles(x), Tiles(y)),
            }),
        }
    }
    pub fn attack(&mut self, target: Entity) {
        self.link.send(&AttackRequest { target });
    }
    pub fn respawn(&mut self) {
        self.link.send(&RespawnRequest {});
    }
    pub fn use_item(&mut self, slot: u32) {
        self.link.send(&UseItemRequest { slot });
    }

    pub fn drain<E: rift::Wire + 'static>(&mut self) -> Vec<E> {
        self.link.client.drain_events()
    }

    pub fn my_entity(&self) -> Option<Entity> {
        let me = self.link.client.id?;
        self.world()
            .iter::<Owner>()
            .find(|(_, owner)| owner.client == me)
            .map(|(entity, _)| entity)
    }

    pub fn my_position(&self) -> Option<Pos<Tiles>> {
        let entity = self.my_entity()?;
        self.world().get::<Position>(entity).map(|p| p.pos)
    }

    pub fn my_health(&self) -> Option<f32> {
        let entity = self.my_entity()?;
        self.world().get::<Vitals>(entity).map(|v| v.health)
    }

    pub fn my_xp(&self) -> Option<u32> {
        let entity = self.my_entity()?;
        self.world().get::<Xp>(entity).map(|xp| xp.amount)
    }

    pub fn my_inventory(&self) -> Vec<ItemId> {
        self.my_entity()
            .and_then(|entity| self.world().get::<Inventory>(entity))
            .map_or_else(Vec::new, |inventory| inventory.items)
    }

    pub fn is_dead(&self) -> bool {
        self.my_health().is_some_and(|health| health <= 0.0)
    }

    pub fn my_area(&self) -> Option<AreaId> {
        let entity = self.my_entity()?;
        self.world().get::<AreaTag>(entity).map(|tag| tag.area)
    }

    pub fn is_spectating(&self) -> bool {
        self.my_entity()
            .is_some_and(|entity| self.world().has::<Spectate>(entity))
    }

    pub fn watching(&self) -> Option<ClientId> {
        self.world().get::<Spectate>(self.my_entity()?)?.watch
    }

    pub fn players(&self) -> Vec<(ClientId, String)> {
        let me = self.link.client.id;
        let world = self.world();
        let mut players: Vec<(ClientId, String)> = world
            .iter::<Owner>()
            .filter(|&(entity, ref owner)| Some(owner.client) != me && world.has::<Actor>(entity))
            .map(|(entity, owner)| {
                let name = world
                    .get::<Name>(entity)
                    .map_or_else(String::new, |n| n.name);
                (owner.client, name)
            })
            .collect();
        players.sort_unstable_by_key(|(id, _)| *id);
        players
    }
}
