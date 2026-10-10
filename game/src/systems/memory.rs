use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::time::{GameDay, Seconds, UnixMillis, WallClock};
use crate::data::memory::Id;
use crate::systems::player::conn_player;
use crate::systems::rule::{Outcome, Requirement, RuleContext};

#[derive(Clone)]
pub struct MemoryDef {
    pub label: &'static str,
    pub kind: MemoryKind,
    pub resets: Resets,
}

impl crate::core::content::ContentRow for MemoryDef {
    const TABLE: &'static str = "memory";
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
    pub fn recall(&self, content: &Content, key: Id, clock: WallClock) -> Option<u32> {
        let recollection = self.0.get(&key)?;
        let def = key.get(content);
        let expired = match def.kind {
            MemoryKind::Timer(lasts) => clock.now.since(recollection.at) >= lasts,
            MemoryKind::Flag | MemoryKind::Counter { .. } => false,
        };
        let stale = def.resets == Resets::Daily && recollection.day != clock.day();
        (!expired && !stale).then_some(recollection.count)
    }

    pub fn remember(&mut self, content: &Content, key: Id, clock: WallClock) {
        let previous = self.recall(content, key, clock);
        let count = match (key.get(content).kind, previous) {
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

    pub fn recalled_within(
        &self,
        content: &Content,
        key: Id,
        lasts: Seconds,
        clock: WallClock,
    ) -> bool {
        self.recall(content, key, clock).is_some() && clock.now.since(self.0[&key].at) < lasts
    }

    pub fn forget(&mut self, key: Id) {
        self.0.remove(&key);
    }

    pub fn leave_area(&mut self, content: &Content) {
        self.0
            .retain(|key, _| key.get(content).resets != Resets::OnLeavingArea);
    }
}

pub fn recall(world: &World, player: Entity, key: Id) -> Option<u32> {
    let clock = *world.resource::<WallClock>();
    world
        .get::<Memory>(player)?
        .recall(world.resource::<Content>(), key, clock)
}

pub struct Remembers(pub Id);

impl Requirement for Remembers {
    fn met(&self, world: &World, player: Entity) -> bool {
        recall(world, player, self.0).is_some()
    }

    fn describe(&self, content: &Content) -> String {
        self.0.get(content).label.to_owned()
    }
}

pub struct RemembersAtLeast(pub Id, pub u32);

impl Requirement for RemembersAtLeast {
    fn met(&self, world: &World, player: Entity) -> bool {
        recall(world, player, self.0).is_some_and(|count| count >= self.1)
    }

    fn describe(&self, content: &Content) -> String {
        format!("{} {}+", self.0.get(content).label, self.1)
    }
}

pub struct RememberedWithin(pub Id, pub Seconds);

impl Requirement for RememberedWithin {
    fn met(&self, world: &World, player: Entity) -> bool {
        let clock = *world.resource::<WallClock>();
        world.get::<Memory>(player).is_some_and(|memory| {
            memory.recalled_within(world.resource::<Content>(), self.0, self.1, clock)
        })
    }

    fn describe(&self, content: &Content) -> String {
        format!("{} recently", self.0.get(content).label)
    }
}

pub struct Remember(pub Id);

impl Outcome for Remember {
    fn apply(&self, ctx: &mut RuleContext) {
        let clock = *ctx.world.resource::<WallClock>();
        let content = ctx.world.resource::<Content>().clone();
        if let Some(mut memory) = ctx.world.get_mut::<Memory>(ctx.player) {
            memory.remember(&content, self.0, clock);
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

/// Make your character remember something.
#[bevy_terminal::command(name = "remember", access = crate::systems::account::role::is_admin)]
fn remember_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    key: String,
) -> Result<String, String> {
    let player = conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?;
    let key = crate::core::content::named::<MemoryDef>(world, &key)?;
    let clock = *world.resource::<WallClock>();
    let content = world.resource::<Content>().clone();
    let mut memory = world
        .get_mut::<Memory>(player)
        .ok_or_else(|| "your character has no memory".to_owned())?;
    memory.remember(&content, key, clock);
    Ok(format!(
        "{key:?} is now {:?}",
        memory.recall(&content, key, clock)
    ))
}

/// Make your character forget something.
#[bevy_terminal::command(name = "forget", access = crate::systems::account::role::is_admin)]
fn forget_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    key: String,
) -> Result<String, String> {
    let player = conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?;
    let key = crate::core::content::named::<MemoryDef>(world, &key)?;
    world
        .get_mut::<Memory>(player)
        .ok_or_else(|| "your character has no memory".to_owned())?
        .forget(key);
    Ok(format!("{key:?} forgotten"))
}
