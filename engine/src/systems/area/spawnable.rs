use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::object::{self, AreaObject, ObjectShape};
use super::{AreaTag, PropId};
use crate::core::content::{
    Content, ContentFile, ContentId, ModuleRule, ModuleSet, NamedContent, Nearby, RowCheck,
    SchemaMark,
};
use crate::core::math::Pos;
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, NEXT, REFUSED, ScriptFault, ValueType,
};
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::{Degrees, Meters, Seconds};
use crate::systems::WorldArea;
use crate::systems::movement;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, OwnerRow, RiftScripting, RunnerKind, ScriptModule,
    ScriptedRow, effect as effects,
};

pub type SpawnableId = ContentId<SpawnableDef>;

/// A thing a script spawns into the world while the game plays: a campfire, a trap, a banner.
/// Where it is spawned it stands as an object of the area does, under its title, looking like its
/// prop and running its scripts, for everyone there, until a script despawns it or its time is
/// up. It keeps no ground, so whoever walks there walks through it, and it is not saved: a server
/// that restarts starts without it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SpawnableDef {
    pub title: String,
    pub look: PropId,
    /// How long it stands once spawned; for good without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lasts: Option<Seconds>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<SpawnableModule>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpawnableModule {
    Script(ScriptModule),
}

/// Spawns a spawnable into the world for everyone there: where someone stands, or at a point,
/// such as the one a skill was aimed at, on the nearest ground one can walk.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("area"))]
pub struct SpawnObject {
    pub spawnable: SpawnableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub at: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// An object a script spawned: what it is, and when its time is up.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SpawnedObject {
    pub spawnable: SpawnableId,
    pub until: Option<UnixMillis>,
}

/// Stands `spawnable` in the area this world serves, on the ground one can walk nearest `at`;
/// nothing where there is none.
pub fn spawn(world: &mut World, spawnable: SpawnableId, at: Pos<Meters>) -> Option<Entity> {
    let content = world.resource::<Content>().clone();
    let def = spawnable.get(&content);
    let area = super::load(&content, world.resource::<WorldArea>().0);
    let at = area.mesh().nearest(at)?;
    let now = world.resource::<WallClock>().now;
    let standing = AreaObject {
        index: None,
        name: None,
        title: Some(def.title.clone()),
        shape: ObjectShape::Point(at),
        look: Some(def.look.path(&content).to_owned()),
        yaw: Degrees(0.0),
        warp: None,
    };
    let stood = object::stand(world, standing, OwnerRow::of_row(spawnable));
    world.entity_mut(stood).insert(SpawnedObject {
        spawnable,
        until: def.lasts.map(|lasts| now.after(lasts)),
    });
    Some(stood)
}

/// Despawns every spawned object whose time is up.
pub fn expire(world: &mut World, spawned: &mut QueryState<(Entity, &SpawnedObject)>) {
    let now = world.resource::<WallClock>().now;
    let due: Vec<Entity> = spawned
        .iter(world)
        .filter(|(_, spawned)| spawned.until.is_some_and(|until| until <= now))
        .map(|(entity, _)| entity)
        .collect();
    for entity in due {
        object::remove(world, entity);
    }
}

impl EffectDef<RiftScripting> for SpawnObject {
    fn needs_subject(&self) -> bool {
        self.at.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let at = match &self.at {
            Some(expr) => {
                let (eval, ty) = cx.typed(expr)?;
                if !matches!(ty, ValueType::Actor | ValueType::Point) {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("at is {}, not an actor or a point", ty.name()),
                    ));
                }
                eval
            }
            None => cx.actor(None)?,
        };
        let (spawnable, refused) = (self.spawnable, self.refused.clone());
        Ok(Box::new(move |world, _, cx| {
            let aimed = at(world, cx);
            let here = world.resource::<WorldArea>().0;
            let point = aimed.as_point().or_else(|| {
                let stood = aimed.as_actor()?;
                let there = world.get::<AreaTag>(stood)?.area == here;
                there.then(|| movement::position(world, stood)).flatten()
            });
            match point.and_then(|point| spawn(world, spawnable, point)) {
                Some(_) => EffectStep::Done,
                None => effects::refuse(world, cx, &refused, "Nothing can stand there"),
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::SpawnsObject(self.spawnable)]
    }
}

impl ContentFile for SpawnableDef {
    const TABLE: &'static str = "spawnable";

    fn fresh(near: &Nearby) -> Result<SpawnableDef, String> {
        Ok(SpawnableDef {
            title: near.name(),
            look: near.file()?,
            lasts: None,
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/title", &self.title, 1, 40);
        if let Some(lasts) = self.lasts {
            check.at_least("/lasts", lasts.0, 1.0, "seconds");
        }
    }
}

impl NamedContent for SpawnableDef {
    fn shown_name(&self) -> &str {
        &self.title
    }
}

impl ModuleSet for SpawnableDef {
    type Module = SpawnableModule;

    fn modules(&self) -> &[SpawnableModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<SpawnableModule> {
        &mut self.modules
    }

    fn rule(_: &SpawnableModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for SpawnableDef {
    const OWNER: RunnerKind = RunnerKind::Spawnable;

    fn script(module: &SpawnableModule) -> Option<&ScriptModule> {
        match module {
            SpawnableModule::Script(script) => Some(script),
        }
    }
}
