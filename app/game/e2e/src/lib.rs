//! Game-contract scenarios, written once against [`Stage`]/[`Player`] and run twice: in-process
//! over the wire protocol (tests/standalone.rs) and as real input to the wasm client in a
//! headless browser (tests/browser).

use std::fmt::Debug;

use world::core::math::{Pos, Tiles};

pub mod scenarios;
pub mod sim;

#[derive(Clone, Debug)]
pub struct Seen {
    pub entity: u32,
    pub owner: Option<u32>,
    pub name: Option<String>,
    pub actor: bool,
    pub pos: Pos<Tiles>,
    pub health: Option<f32>,
    pub max: Option<f32>,
    pub area: Option<world::core::area::AreaId>,
    pub action: Option<u8>,
    pub spectate: bool,
    pub xp: Option<u32>,
    pub inventory: Vec<world::ItemId>,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub open: bool,
    pub client: Option<u32>,
    pub spectating: bool,
    pub watching: Option<u32>,
    pub actors: Vec<Seen>,
}

impl View {
    pub fn me(&self) -> Option<&Seen> {
        let me = self.client?;
        self.actors.iter().find(|seen| seen.owner == Some(me))
    }
    pub fn player_of(&self, owner: u32) -> Option<&Seen> {
        self.actors
            .iter()
            .find(|seen| seen.owner == Some(owner) && seen.actor)
    }
    pub fn npcs(&self) -> impl Iterator<Item = &Seen> {
        self.actors
            .iter()
            .filter(|seen| seen.owner.is_none() && seen.actor)
    }
}

/// Driven only through what a real player can do. Commands that take time are progressive:
/// issue them again while polling, the way a player keeps clicking.
pub trait Player {
    fn view(&mut self) -> View;
    fn client_id(&mut self) -> u32 {
        self.view()
            .client
            .expect("client id arrives with the first snapshot")
    }
    fn move_to(&mut self, x: f32, y: f32);
    fn attack(&mut self, entity: u32);
    fn respawn(&mut self);
    fn watch(&mut self, owner: u32);
    fn use_item(&mut self, slot: u32);
}

/// Hosts the running game and connects clients to it. Dropping a [`Player`] disconnects it.
pub trait Stage {
    fn player(&mut self) -> Box<dyn Player>;
    fn spectator(&mut self) -> Box<dyn Player>;
    fn unentitled_spectator(&mut self) -> Box<dyn Player>;
    fn step(&mut self, seconds: f32);
}

pub fn eventually(stage: &mut dyn Stage, seconds: f32, mut ready: impl FnMut() -> bool) -> bool {
    const STEP: f32 = 0.1;
    let steps = (seconds / STEP).ceil() as usize;
    for _ in 0..steps {
        if ready() {
            return true;
        }
        stage.step(STEP);
    }
    ready()
}

pub fn view_of(client: &rift::Client) -> View {
    let world = &client.world;
    let actors = world
        .iter::<world::Position>()
        .map(|(entity, position)| Seen {
            entity: entity.0,
            owner: world.get::<world::Owner>(entity).map(|o| o.client.0),
            name: world.get::<world::Name>(entity).map(|n| n.name),
            actor: world.has::<world::Actor>(entity),
            pos: position.pos,
            health: world.get::<world::Vitals>(entity).map(|v| v.health),
            max: world.get::<world::Vitals>(entity).map(|v| v.max),
            area: world.get::<world::AreaTag>(entity).map(|t| t.area),
            action: world.get::<world::Actor>(entity).map(|a| a.action),
            spectate: world.has::<world::Spectate>(entity),
            xp: world.get::<world::Xp>(entity).map(|xp| xp.amount),
            inventory: world
                .get::<world::Inventory>(entity)
                .map_or_else(Vec::new, |inventory| inventory.items),
        })
        .collect();
    let me = client.id.and_then(|me| {
        world
            .iter::<world::Owner>()
            .find(|(_, owner)| owner.client == me)
            .map(|(entity, _)| entity)
    });
    View {
        open: client.id.is_some(),
        client: client.id.map(|c| c.0),
        spectating: me.is_some_and(|entity| world.has::<world::Spectate>(entity)),
        watching: me
            .and_then(|entity| world.get::<world::Spectate>(entity))
            .and_then(|s| s.watch)
            .map(|c| c.0),
        actors,
    }
}
