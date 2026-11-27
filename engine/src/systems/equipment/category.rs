use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentFile, ContentId, NamedContent, Nearby, RowCheck};
use crate::core::text::LineText;
use crate::systems::item::ItemKind;
use crate::systems::messages;

pub type EquipmentCategoryId = ContentId<EquipmentCategoryDef>;

/// A kind of equipment, by the name players are told: what every equipment item is one of, and
/// what a skill can ask its user to have equipped.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentCategoryDef {
    pub name: String,
}

impl ContentFile for EquipmentCategoryDef {
    const TABLE: &'static str = "equipment_category";

    fn fresh(near: &Nearby) -> Result<EquipmentCategoryDef, String> {
        Ok(EquipmentCategoryDef { name: near.name() })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 64);
    }
}

impl NamedContent for EquipmentCategoryDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

/// Whether `actor` has equipped an item of any of `categories`; always, when none are asked for.
pub fn equips_any(world: &World, actor: Entity, categories: &[EquipmentCategoryId]) -> bool {
    if categories.is_empty() {
        return true;
    }
    let content = world.resource::<Content>();
    super::worn(world, actor).any(|item| match &item.get(content).kind {
        ItemKind::Equipment { category, .. } => categories.contains(category),
        _ => false,
    })
}

/// What having any of `categories` equipped reads as, where a player reads what a skill needs.
pub fn wanted(content: &Content, categories: &[EquipmentCategoryId]) -> LineText {
    let all = messages::of(content);
    let names = categories
        .iter()
        .map(|category| LineText::plain(category.get(content).name.clone()))
        .collect();
    all.condition.worn_any(all.interface.either(names))
}
