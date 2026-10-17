pub mod aspect;
pub mod boost;
pub mod effects;
pub mod widget;

pub use aspect::{ActiveAspects, AspectSource};
pub use boost::{Boosts, HeldBoost};
pub use effects::{ApplyStatus, Boost, RemoveStatus, boosts_of};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::assets::AssetRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::{Content, ContentId, ContentRow, ModuleRule, ModuleSet, RowCheck};
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::core::vfx::VfxId;
use crate::systems::script::{OwnerKind, ScriptModule, ScriptedRow};
use crate::systems::stat;

pub type StatusEffectId = ContentId<StatusEffectDef>;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Statuses>().replicate::<Boosts>();
    stat::source(app, boost::modifiers);
}

/// A buff or debuff an actor carries for a while: the row says how it looks and sounds coming,
/// staying and going, and its script says what it does while it holds.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct StatusEffectDef {
    pub name: String,
    pub icon: AssetRef,
    pub category: StatusCategory,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "StatusVfx::is_empty")]
    pub vfx: StatusVfx,
    #[serde(default, skip_serializing_if = "StatusSfx::is_empty")]
    pub sfx: StatusSfx,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<StatusModule>,
}

/// Only a label other systems filter by, such as a cleanse of every debuff; nothing is built into
/// either.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
pub enum StatusCategory {
    Buff,
    Debuff,
}

#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusVfx {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<VfxId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub during: Option<VfxId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_expire: Option<VfxId>,
}

#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusSfx {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<SfxId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub during: Option<SfxId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_expire: Option<SfxId>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusModule {
    #[serde(untagged)]
    Script(ScriptModule),
}

/// The statuses on an actor, one per effect: applying one it has refreshes its power and clock.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Statuses(pub Vec<StatusInstance>);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct StatusInstance {
    pub effect: StatusEffectId,
    pub power: f32,
    pub since: UnixMillis,
    /// `None` holds until removed.
    pub until: Option<UnixMillis>,
}

impl Statuses {
    pub fn get(&self, effect: StatusEffectId) -> Option<&StatusInstance> {
        self.0.iter().find(|status| status.effect == effect)
    }

    pub fn has(&self, effect: StatusEffectId) -> bool {
        self.get(effect).is_some()
    }

    pub fn count(&self, content: &Content, category: Option<StatusCategory>) -> usize {
        self.0
            .iter()
            .filter(|status| {
                category.is_none_or(|wanted| status.effect.get(content).category == wanted)
            })
            .count()
    }
}

pub fn apply(
    world: &mut World,
    actor: Entity,
    effect: StatusEffectId,
    power: f32,
    lasts: Option<Seconds>,
) {
    if world.get_entity(actor).is_err() {
        return;
    }
    let now = world.resource::<WallClock>().now;
    let fresh = StatusInstance {
        effect,
        power,
        since: now,
        until: lasts.map(|lasts| now.after(lasts)),
    };
    match world.get_mut::<Statuses>(actor) {
        Some(mut statuses) => match statuses.0.iter_mut().find(|held| held.effect == effect) {
            Some(held) => *held = fresh,
            None => statuses.0.push(fresh),
        },
        None => {
            world.entity_mut(actor).insert(Statuses(vec![fresh]));
        }
    }
}

pub fn remove(world: &mut World, actor: Entity, effect: StatusEffectId) -> bool {
    let Some(mut statuses) = world.get_mut::<Statuses>(actor) else {
        return false;
    };
    if !statuses.has(effect) {
        return false;
    }
    statuses.0.retain(|held| held.effect != effect);
    true
}

/// Takes every status of a category off, or every status at all; how many went.
pub fn cleanse(world: &mut World, actor: Entity, category: Option<StatusCategory>) -> usize {
    let content = world.resource::<Content>().clone();
    let Some(mut statuses) = world.get_mut::<Statuses>(actor) else {
        return 0;
    };
    let before = statuses.0.len();
    if before == 0 {
        return 0;
    }
    statuses
        .0
        .retain(|held| category.is_some_and(|wanted| held.effect.get(&content).category != wanted));
    before - statuses.0.len()
}

pub fn has(world: &World, actor: Entity, effect: StatusEffectId) -> bool {
    world
        .get::<Statuses>(actor)
        .is_some_and(|statuses| statuses.has(effect))
}

pub fn power(world: &World, actor: Entity, effect: StatusEffectId) -> f32 {
    world
        .get::<Statuses>(actor)
        .and_then(|statuses| statuses.get(effect))
        .map_or(0.0, |status| status.power)
}

/// Sweeps the statuses whose time ran out; the aspects settle after and raise their going.
pub fn expire(world: &mut World, held: &mut QueryState<(Entity, &Statuses)>) {
    let now = world.resource::<WallClock>().now;
    let due: Vec<Entity> = held
        .iter(world)
        .filter(|(_, statuses)| {
            statuses
                .0
                .iter()
                .any(|status| status.until.is_some_and(|until| until <= now))
        })
        .map(|(entity, _)| entity)
        .collect();
    for entity in due {
        if let Some(mut statuses) = world.get_mut::<Statuses>(entity) {
            statuses
                .0
                .retain(|status| status.until.is_none_or(|until| until > now));
        }
    }
}

impl StatusCategory {
    pub fn label(self) -> &'static str {
        self.into()
    }
}

impl StatusVfx {
    fn is_empty(&self) -> bool {
        self.on_activate.is_none() && self.during.is_none() && self.on_expire.is_none()
    }
}

impl StatusSfx {
    fn is_empty(&self) -> bool {
        self.on_activate.is_none() && self.during.is_none() && self.on_expire.is_none()
    }
}

impl ContentRow for StatusEffectDef {
    const TABLE: &'static str = "status_effect";

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.text("/description", &self.description, 0, 200);
    }
}

impl ModuleSet for StatusEffectDef {
    type Module = StatusModule;

    fn modules(&self) -> &[StatusModule] {
        &self.modules
    }

    fn rule(_: &StatusModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for StatusEffectDef {
    const OWNER: OwnerKind = OwnerKind::StatusEffect;

    fn script(module: &StatusModule) -> Option<&ScriptModule> {
        match module {
            StatusModule::Script(script) => Some(script),
        }
    }
}
