//! What an actor looks like: its own appearance, a model and the parts it wears besides, with what
//! it has on laid over it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use super::held::{self, Held};
use super::material;
use super::part::ModelPartId;
use super::skeleton::{Colours, EquipmentSlot};
use super::{ActorModel, Hitbox, ModelId};
use crate::core::content::{Content, RowCheck};
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault};
use crate::systems::item::{ItemId, Wear, WornLook};
use crate::systems::script::event::NPC;
use crate::systems::script::{EffectCall, Reach, RiftScripting, RunnerKind};

/// An actor's own look without its gear: a model, parts of the model's skeleton laid over the
/// model's, and colours painted over the model's.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Appearance {
    pub model: ModelId,
    pub parts: Vec<ModelPartId>,
    pub colours: Colours,
}

impl Appearance {
    pub fn of(model: ModelId) -> Appearance {
        Appearance {
            model,
            parts: Vec::new(),
            colours: Colours::default(),
        }
    }
}

/// The file form leaves out what is empty; the wire form always carries it, as a binary format
/// cannot tell an omitted field from a missing one.
#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppearanceRow {
    model: ModelId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<ModelPartId>,
    #[serde(default, skip_serializing_if = "Colours::is_empty")]
    colours: Colours,
}

impl Serialize for Appearance {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return AppearanceRow {
                model: self.model,
                parts: self.parts.clone(),
                colours: self.colours.clone(),
            }
            .serialize(serializer);
        }
        (self.model, &self.parts, &self.colours).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Appearance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Appearance, D::Error> {
        if deserializer.is_human_readable() {
            let AppearanceRow {
                model,
                parts,
                colours,
            } = AppearanceRow::deserialize(deserializer)?;
            return Ok(Appearance {
                model,
                parts,
                colours,
            });
        }
        let (model, parts, colours) =
            <(ModelId, Vec<ModelPartId>, Colours)>::deserialize(deserializer)?;
        Ok(Appearance {
            model,
            parts,
            colours,
        })
    }
}

impl JsonSchema for Appearance {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Appearance")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        AppearanceRow::json_schema(generator)
    }

    fn inline_schema() -> bool {
        true
    }
}

/// The parts a body draws, in the order they were laid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyLook {
    pub parts: Vec<LaidPart>,
}

/// A part a body draws, and the colours it is painted in over its skeleton's.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidPart {
    pub part: ModelPartId,
    pub colours: Colours,
}

/// What a body looks like wearing `worn`: laid in order, its model's parts, then its appearance's,
/// then those of each worn item that has a look on the model's skeleton, in the order the skeleton
/// lists its slots. Every part is painted in the appearance's colours over the model's, and a worn
/// item's parts in the item's colours over those.
pub fn look(
    content: &Content,
    appearance: &Appearance,
    worn: &BTreeMap<EquipmentSlot, ItemId>,
) -> BodyLook {
    let model = appearance.model.get(content);
    let skeleton = model.skeleton.get(content);
    let colours = appearance.colours.over(&model.colours);
    let own = model
        .parts
        .iter()
        .chain(&appearance.parts)
        .map(|&part| LaidPart {
            part,
            colours: colours.clone(),
        });
    let worn_parts = skeleton
        .slots
        .iter()
        .filter_map(|slot| worn.get(slot))
        .filter_map(|item| worn_look(content, *item, appearance.model))
        .flat_map(|look| {
            let colours = look.colours.over(&colours);
            look.parts.iter().map(move |&part| LaidPart {
                part,
                colours: colours.clone(),
            })
        });
    laid(content, own.chain(worn_parts).collect::<Vec<_>>())
}

/// The parts left showing once `layers` are laid in order: each part replaces the parts before it
/// that cover any region it covers, and once all are laid, every part covering a region one of them
/// hides is taken off.
pub fn laid(content: &Content, layers: impl IntoIterator<Item = LaidPart>) -> BodyLook {
    let mut parts: Vec<LaidPart> = Vec::new();
    for layer in layers {
        let covers = &layer.part.get(content).covers;
        parts.retain(|laid| {
            laid.part != layer.part
                && !laid
                    .part
                    .get(content)
                    .covers
                    .iter()
                    .any(|region| covers.contains(region))
        });
        parts.push(layer);
    }
    let hidden: Vec<_> = parts
        .iter()
        .flat_map(|laid| laid.part.get(content).hides.iter().cloned())
        .collect();
    parts.retain(|laid| {
        !laid
            .part
            .get(content)
            .covers
            .iter()
            .any(|region| hidden.contains(region))
    });
    BodyLook { parts }
}

/// The parts `item` lays over a body of `model`: those of its look on the model's skeleton, none
/// when it has no look there or is not worn on the body.
pub fn worn_parts(content: &Content, item: ItemId, model: ModelId) -> Vec<ModelPartId> {
    worn_look(content, item, model).map_or_else(Vec::new, |look| look.parts.clone())
}

fn worn_look(content: &Content, item: ItemId, model: ModelId) -> Option<&WornLook> {
    let skeleton = model.get(content).skeleton;
    match item.get(content).wear() {
        Some(Wear::Worn { looks, .. }) => looks.iter().find(|look| look.skeleton == skeleton),
        _ => None,
    }
}

/// Every part an appearance lays over its model is made on the model's skeleton, and every colour
/// it paints is one of that skeleton's tints.
pub fn check(content: &Content, appearance: &Appearance, pointer: &str, cx: &mut RowCheck) {
    let skeleton = appearance.model.get(content).skeleton;
    material::check_colours(
        content,
        skeleton,
        &appearance.colours,
        &format!("{pointer}/colours"),
        cx,
    );
    for (index, part) in appearance.parts.iter().enumerate() {
        let made_on = part.get(content).skeleton;
        if made_on != skeleton {
            cx.error(
                format!("{pointer}/parts/{index}"),
                "part of another skeleton",
                format!(
                    "{} is made on {}",
                    part.path(content),
                    made_on.path(content)
                ),
                format!("a part made on {}", skeleton.path(content)),
            );
        }
    }
}

/// Dresses the npc whose script this is in a new appearance; one its model cannot wear is refused
/// as the game loads.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct SetAppearance {
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for SetAppearance {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let mut fits = RowCheck::default();
        check(cx.content, &self.appearance, "/appearance", &mut fits);
        if let Some(error) = fits.into_errors().into_iter().next() {
            return Err(ScriptFault::new(error.problem.rule, error.problem.expected));
        }
        let appearance = self.appearance.clone();
        Ok(Box::new(move |world, _, cx| {
            if let Some(npc) = cx.owner {
                change(world, npc, appearance.clone());
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        Vec::new()
    }
}

/// Gives `actor` a new appearance: it is hit as the new model is, and its hands show what they held
/// as far as the new body has hands.
pub fn change(world: &mut World, actor: Entity, appearance: Appearance) {
    let content = world.resource::<Content>().clone();
    let model = ActorModel::get(&content, appearance.model);
    if let Some(shown) = world.get::<Held>(actor) {
        let mut items: Vec<ItemId> = Vec::new();
        for &item in shown.hands.values() {
            if !items.contains(&item) {
                items.push(item);
            }
        }
        let shown = Held {
            hands: held::listed(&content, model.hands().len(), &items),
        };
        world.entity_mut(actor).insert(shown);
    }
    world
        .entity_mut(actor)
        .insert((Hitbox(model.hitbox()), appearance));
}

/// The model an actor wears, when it has an appearance.
pub fn model_of(world: &World, actor: Entity) -> Option<&ActorModel> {
    let appearance = world.get::<Appearance>(actor)?;
    Some(ActorModel::get(
        world.resource::<Content>(),
        appearance.model,
    ))
}
