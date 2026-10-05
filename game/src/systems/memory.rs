use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::time::{GameDay, Seconds, UnixMillis, WallClock};
use crate::data::memory::Id;
use crate::systems::player::conn_player;
use crate::systems::rule::{Outcome, Requirement, RuleContext};

pub struct MemoryDef {
    pub label: &'static str,
    pub kind: MemoryKind,
    pub resets: Resets,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemoryKind {
    Flag,
    Counter { step_every: Seconds },
    Timer(Seconds),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resets {
    Never,
    Daily,
    OnLeavingArea,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Memory(BTreeMap<Id, Recollection>);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Recollection {
    count: u32,
    at: UnixMillis,
    day: GameDay,
}

impl Memory {
    pub fn recall(&self, key: Id, clock: WallClock) -> Option<u32> {
        let recollection = self.0.get(&key)?;
        let def = key.get();
        let expired = match def.kind {
            MemoryKind::Timer(lasts) => clock.now.since(recollection.at) >= lasts,
            MemoryKind::Flag | MemoryKind::Counter { .. } => false,
        };
        let stale = def.resets == Resets::Daily && recollection.day != clock.day();
        (!expired && !stale).then_some(recollection.count)
    }

    pub fn remember(&mut self, key: Id, clock: WallClock) {
        let previous = self.recall(key, clock);
        let count = match (key.get().kind, previous) {
            (MemoryKind::Counter { step_every }, Some(count)) => {
                let last = self.0[&key].at;
                if clock.now.since(last) < step_every {
                    return;
                }
                count.saturating_add(1)
            }
            _ => 1,
        };
        self.0.insert(
            key,
            Recollection {
                count,
                at: clock.now,
                day: clock.day(),
            },
        );
    }

    pub fn forget(&mut self, key: Id) {
        self.0.remove(&key);
    }

    pub fn leave_area(&mut self) {
        self.0
            .retain(|key, _| key.get().resets != Resets::OnLeavingArea);
    }
}

pub fn recall(world: &World, player: Entity, key: Id) -> Option<u32> {
    let clock = *world.resource::<WallClock>();
    world.get::<Memory>(player)?.recall(key, clock)
}

pub struct Remembers(pub Id);

impl Requirement for Remembers {
    fn met(&self, world: &World, player: Entity) -> bool {
        recall(world, player, self.0).is_some()
    }

    fn describe(&self) -> String {
        self.0.get().label.to_owned()
    }
}

pub struct RemembersAtLeast(pub Id, pub u32);

impl Requirement for RemembersAtLeast {
    fn met(&self, world: &World, player: Entity) -> bool {
        recall(world, player, self.0).is_some_and(|count| count >= self.1)
    }

    fn describe(&self) -> String {
        format!("{} {}+", self.0.get().label, self.1)
    }
}

pub struct Remember(pub Id);

impl Outcome for Remember {
    fn apply(&self, ctx: &mut RuleContext) {
        let clock = *ctx.world.resource::<WallClock>();
        if let Some(mut memory) = ctx.world.get_mut::<Memory>(ctx.player) {
            memory.remember(self.0, clock);
        }
    }
}

pub struct Forget(pub Id);

impl Outcome for Forget {
    fn apply(&self, ctx: &mut RuleContext) {
        if let Some(mut memory) = ctx.world.get_mut::<Memory>(ctx.player) {
            memory.forget(self.0);
        }
    }
}

impl bevy_terminal::CommandArg for Id {
    fn parse(name: &str, raw: Option<&str>) -> Result<Id, String> {
        crate::core::table::parse_id(name, raw, "memory")
    }
}

/// Make your character remember something.
#[bevy_terminal::command(name = "remember", access = crate::systems::account::role::is_admin)]
fn remember_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    key: Id,
) -> Result<String, String> {
    let player = conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?;
    let clock = *world.resource::<WallClock>();
    let mut memory = world
        .get_mut::<Memory>(player)
        .ok_or_else(|| "your character has no memory".to_owned())?;
    memory.remember(key, clock);
    Ok(format!("{key:?} is now {:?}", memory.recall(key, clock)))
}

/// Make your character forget something.
#[bevy_terminal::command(name = "forget", access = crate::systems::account::role::is_admin)]
fn forget_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    key: Id,
) -> Result<String, String> {
    let player = conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?;
    world
        .get_mut::<Memory>(player)
        .ok_or_else(|| "your character has no memory".to_owned())?
        .forget(key);
    Ok(format!("{key:?} forgotten"))
}
