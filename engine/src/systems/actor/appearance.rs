//! What an actor looks like: its own appearance, a model and the parts it wears besides, with what
//! it has on laid over it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use super::part::ModelPartId;
use super::skeleton::EquipmentSlot;
use super::{ActorModel, ModelId};
use crate::core::content::{Content, RowCheck};
use crate::systems::item::{ItemId, Wear};

/// An actor's own look without its gear: a model, and parts of the model's skeleton laid over the
/// model's.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Appearance {
    pub model: ModelId,
    pub parts: Vec<ModelPartId>,
}

impl Appearance {
    pub fn of(model: ModelId) -> Appearance {
        Appearance {
            model,
            parts: Vec::new(),
        }
    }
}

/// The file form leaves no parts out; the wire form always carries them, as a binary format cannot
/// tell an omitted field from a missing one.
#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppearanceRow {
    model: ModelId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<ModelPartId>,
}

impl Serialize for Appearance {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return AppearanceRow {
                model: self.model,
                parts: self.parts.clone(),
            }
            .serialize(serializer);
        }
        (self.model, &self.parts).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Appearance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Appearance, D::Error> {
        if deserializer.is_human_readable() {
            let AppearanceRow { model, parts } = AppearanceRow::deserialize(deserializer)?;
            return Ok(Appearance { model, parts });
        }
        let (model, parts) = <(ModelId, Vec<ModelPartId>)>::deserialize(deserializer)?;
        Ok(Appearance { model, parts })
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
    pub parts: Vec<ModelPartId>,
}

/// What a body looks like wearing `worn`: laid in order, its model's parts, then its appearance's,
/// then those of each worn item that has a look on the model's skeleton, in the order the skeleton
/// lists its slots.
pub fn look(
    content: &Content,
    appearance: &Appearance,
    worn: &BTreeMap<EquipmentSlot, ItemId>,
) -> BodyLook {
    let model = appearance.model.get(content);
    let skeleton = model.skeleton.get(content);
    let worn_parts = skeleton
        .slots
        .iter()
        .filter_map(|slot| worn.get(slot))
        .flat_map(|item| worn_parts(content, *item, appearance.model));
    laid(
        content,
        model
            .parts
            .iter()
            .chain(&appearance.parts)
            .copied()
            .chain(worn_parts),
    )
}

/// The parts left showing once `layers` are laid in order: each part replaces the parts before it
/// that cover any region it covers, and once all are laid, every part covering a region one of them
/// hides is taken off.
pub fn laid(content: &Content, layers: impl IntoIterator<Item = ModelPartId>) -> BodyLook {
    let mut parts: Vec<ModelPartId> = Vec::new();
    for part in layers {
        let covers = &part.get(content).covers;
        parts.retain(|laid| {
            *laid != part
                && !laid
                    .get(content)
                    .covers
                    .iter()
                    .any(|region| covers.contains(region))
        });
        parts.push(part);
    }
    let hidden: Vec<_> = parts
        .iter()
        .flat_map(|part| part.get(content).hides.iter().cloned())
        .collect();
    parts.retain(|part| {
        !part
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
    let skeleton = model.get(content).skeleton;
    match item.get(content).wear() {
        Some(Wear::Worn { looks, .. }) => looks
            .iter()
            .find(|look| look.skeleton == skeleton)
            .map_or_else(Vec::new, |look| look.parts.clone()),
        _ => Vec::new(),
    }
}

/// Every part an appearance lays over its model is made on the model's skeleton.
pub fn check(content: &Content, appearance: &Appearance, pointer: &str, cx: &mut RowCheck) {
    let skeleton = appearance.model.get(content).skeleton;
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

/// The model an actor wears, when it has an appearance.
pub fn model_of(world: &World, actor: Entity) -> Option<&ActorModel> {
    let appearance = world.get::<Appearance>(actor)?;
    Some(ActorModel::get(
        world.resource::<Content>(),
        appearance.model,
    ))
}
