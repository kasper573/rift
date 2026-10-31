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

use crate::core::assets::ImageRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::{
    Content, ContentFile, ContentId, ModuleRule, ModuleSet, NamedContent, Nearby, RowCheck,
};
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::core::vfx::VfxId;
use crate::systems::mirror::{self, Effect, Ghost};
use crate::systems::script::{RunnerKind, ScriptModule, ScriptedRow};

pub type StatusEffectId = ContentId<StatusEffectDef>;

pub fn register(app: &mut App) {
    use crate::systems::mirror::AppMirrorExt;
    app.replicate::<Statuses>().replicate_private::<Boosts>();
}

/// A buff or debuff an actor carries for a while: the row says how it looks and sounds coming,
/// staying and going, and its script says what it does while it holds.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct StatusEffectDef {
    pub name: String,
    pub icon: ImageRef,
    pub category: StatusCategory,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Phased::is_empty")]
    pub vfx: Phased<VfxId>,
    #[serde(default, skip_serializing_if = "Phased::is_empty")]
    pub sfx: Phased<SfxId>,
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

/// What a status shows or plays coming, staying and going.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phased<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_expire: Option<T>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusModule {
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

    /// How many statuses of a category it holds, or of any when none is named.
    pub fn count(&self, content: &Content, category: Option<StatusCategory>) -> usize {
        self.0
            .iter()
            .filter(|status| status.in_category(content, category))
            .count()
    }
}

impl StatusInstance {
    /// Whether the status falls under a category; every status falls under none named.
    pub fn in_category(&self, content: &Content, category: Option<StatusCategory>) -> bool {
        category.is_none_or(|wanted| self.effect.get(content).category == wanted)
    }
}

pub fn apply(
    world: &mut World,
    actor: Entity,
    effect: StatusEffectId,
    power: f32,
    lasts: Option<Seconds>,
) {
    if world.get_entity(actor).is_err()
        || mirror::post(
            world,
            actor,
            None,
            Effect::Status {
                effect,
                power,
                lasts,
            },
        )
    {
        return;
    }
    let now = world.resource::<WallClock>().now;
    let fresh = StatusInstance {
        effect,
        power,
        since: now,
        until: lasts.map(|lasts| now.after(lasts)),
    };
    if let Some(mut statuses) = world.get_mut::<Statuses>(actor) {
        match statuses.0.iter_mut().find(|held| held.effect == effect) {
            Some(held) => *held = fresh,
            None => statuses.0.push(fresh),
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
        .retain(|held| !held.in_category(&content, category));
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

pub fn expire(world: &mut World, held: &mut QueryState<(Entity, &Statuses), Without<Ghost>>) {
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

impl<T> Default for Phased<T> {
    fn default() -> Phased<T> {
        Phased {
            on_activate: None,
            during: None,
            on_expire: None,
        }
    }
}

impl<T> Phased<T> {
    fn is_empty(&self) -> bool {
        self.on_activate.is_none() && self.during.is_none() && self.on_expire.is_none()
    }
}

impl ContentFile for StatusEffectDef {
    const TABLE: &'static str = "status_effect";

    fn fresh(near: &Nearby) -> Result<StatusEffectDef, String> {
        Ok(StatusEffectDef {
            name: near.name(),
            icon: near.image()?,
            category: StatusCategory::Buff,
            description: String::new(),
            vfx: Phased::default(),
            sfx: Phased::default(),
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.text("/description", &self.description, 0, 200);
    }
}

impl NamedContent for StatusEffectDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for StatusEffectDef {
    type Module = StatusModule;

    fn modules(&self) -> &[StatusModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<StatusModule> {
        &mut self.modules
    }

    fn rule(_: &StatusModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for StatusEffectDef {
    const OWNER: RunnerKind = RunnerKind::StatusEffect;

    fn script(module: &StatusModule) -> Option<&ScriptModule> {
        match module {
            StatusModule::Script(script) => Some(script),
        }
    }
}
