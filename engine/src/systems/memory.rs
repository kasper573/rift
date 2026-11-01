use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentFile, ContentId, NamedContent, Nearby};
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault, Value, ValueType,
};
use crate::core::time::{GameDay, UnixMillis, WallClock};
use crate::core::units::Seconds;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, RiftScripting, effect as effects,
};

pub type MemoryId = ContentId<MemoryDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct MemoryDef {
    pub label: String,
    #[serde(rename = "type")]
    pub ty: ValueType,
    pub resets: Resets,
}

impl ContentFile for MemoryDef {
    const TABLE: &'static str = "memory";

    fn fresh(near: &Nearby) -> Result<MemoryDef, String> {
        Ok(MemoryDef {
            label: near.name(),
            ty: ValueType::Boolean,
            resets: Resets::Never,
        })
    }
}

impl NamedContent for MemoryDef {
    fn shown_name(&self) -> &str {
        &self.label
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resets {
    Never,
    Daily,
    LeavingArea,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Memory(BTreeMap<MemoryId, Recollection>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Recollection {
    value: Value,
    at: UnixMillis,
    day: GameDay,
}

impl Recollection {
    fn holds(&self, resets: Resets, clock: WallClock) -> bool {
        resets != Resets::Daily || self.day == clock.day()
    }
}

impl Memory {
    pub fn get(&self, content: &Content, key: MemoryId, clock: WallClock) -> Value {
        let def = key.get(content);
        match self.0.get(&key) {
            Some(recollection) if recollection.holds(def.resets, clock) => {
                recollection.value.clone()
            }
            _ => def.ty.zero(),
        }
    }

    pub fn is_set(&self, content: &Content, key: MemoryId, clock: WallClock) -> bool {
        self.get(content, key, clock).truthy()
    }

    pub fn set(&mut self, content: &Content, key: MemoryId, value: Value, clock: WallClock) {
        let ty = key.get(content).ty;
        self.0.insert(
            key,
            Recollection {
                value: value.widened(ty),
                at: clock.now,
                day: clock.day(),
            },
        );
    }

    pub fn set_within(
        &self,
        content: &Content,
        key: MemoryId,
        lasts: Seconds,
        clock: WallClock,
    ) -> bool {
        let def = key.get(content);
        self.0.get(&key).is_some_and(|recollection| {
            recollection.holds(def.resets, clock) && clock.now.since(recollection.at) < lasts
        })
    }

    pub fn forget(&mut self, key: MemoryId) {
        self.0.remove(&key);
    }

    pub fn keys(&self) -> impl Iterator<Item = MemoryId> + '_ {
        self.0.keys().copied()
    }

    pub fn leave_area(&mut self, content: &Content) {
        self.0
            .retain(|key, _| key.get(content).resets != Resets::LeavingArea);
    }
}

pub fn get(world: &World, player: Entity, key: MemoryId) -> Value {
    let clock = *world.resource::<WallClock>();
    let content = world.resource::<Content>();
    world.get::<Memory>(player).map_or_else(
        || key.get(content).ty.zero(),
        |memory| memory.get(content, key, clock),
    )
}

pub fn set_within(world: &World, player: Entity, key: MemoryId, lasts: Seconds) -> bool {
    let clock = *world.resource::<WallClock>();
    world
        .get::<Memory>(player)
        .is_some_and(|memory| memory.set_within(world.resource::<Content>(), key, lasts, clock))
}

pub fn set(world: &mut World, player: Entity, key: MemoryId, value: Value) {
    let clock = *world.resource::<WallClock>();
    let content = world.resource::<Content>().clone();
    if let Some(mut memory) = world.get_mut::<Memory>(player) {
        memory.set(&content, key, value, clock);
    }
}

pub fn forget(world: &mut World, player: Entity, key: MemoryId) {
    if let Some(mut memory) = world.get_mut::<Memory>(player) {
        memory.forget(key);
    }
}

/// Sets a memory, timed now.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "memory"))]
pub struct Remember {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub memory: MemoryId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Sets memories back to zero.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "memory"))]
pub struct Forget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<MemoryId>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Remember {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let ty = self.memory.get(cx.content).ty;
        let to = cx.opt_expr(self.to.as_ref(), ty)?;
        let memory = self.memory;
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                let value = match &to {
                    Some(to) => to(world, cx),
                    None => Value::Bool(true).widened(ty),
                };
                set(world, player, memory, value);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Remembers(self.memory)]
    }
}

impl EffectDef<RiftScripting> for Forget {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let memories = self.memories.clone();
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                for &memory in &memories {
                    forget(world, player, memory);
                }
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        self.memories
            .iter()
            .map(|memory| EffectReach::Remembers(*memory))
            .collect()
    }
}
