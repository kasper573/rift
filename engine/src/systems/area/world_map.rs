use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    ContentId, ContentProblem, ContentRow, Fixture, Loading, RowCheck, serial,
};
use crate::core::math::Pos;

pub type WorldMapId = ContentId<WorldMapDef>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum WorldMap {
    World,
}

impl Fixture for WorldMap {
    type Row = WorldMapDef;
}

pub const MIN_SIZE: u32 = 10;
pub const MAX_SIZE: u32 = 400;
pub const MAX_NOTE: usize = 280;

#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMapDef {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cards: BTreeMap<super::AreaId, Card>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
}

impl ContentRow for WorldMapDef {
    const TABLE: &'static str = "world_map";

    fn check(&self, cx: &mut RowCheck) {
        let mut placed: Vec<&super::AreaId> = Vec::new();
        for (index, region) in self.regions.iter().enumerate() {
            cx.text(format!("/regions/{index}/name"), &region.name, 1, 60);
            for (at, area) in region.cards.iter().enumerate() {
                if placed.contains(&area) {
                    cx.error(
                        format!("/regions/{index}/cards/{at}"),
                        "duplicate name",
                        format!("{area:?} in two regions"),
                        "a card in at most one region",
                    );
                }
                placed.push(area);
            }
        }
        for (index, note) in self.notes.iter().enumerate() {
            cx.text(format!("/notes/{index}/text"), &note.text, 1, MAX_NOTE);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldMapTiles {}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<WorldMapTiles>,
    pub size: u32,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub pinned: bool,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    pub cards: Vec<super::AreaId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<RegionColor>,
}

#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::IntoStaticStr,
    strum::VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RegionColor {
    Amber,
    Green,
    Blue,
    Red,
    Violet,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub text: String,
    pub x: f32,
    pub y: f32,
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<WorldMapDef>(&content) {
        for (area, card) in &def.cards {
            if !(MIN_SIZE..=MAX_SIZE).contains(&card.size) {
                loading.row_error(
                    id,
                    &format!("/cards/{}/size", area.key(&content)),
                    ContentProblem::new(
                        "out of range",
                        card.size,
                        format!("percent {MIN_SIZE} to {MAX_SIZE}"),
                    ),
                );
            }
        }
    }
    let only = WorldMap::World.key();
    for id in content.ids::<WorldMapDef>() {
        let key = id.key(&content);
        if key != only {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "wrong key",
                    format!("world_map/{key}"),
                    format!("one world_map row, the {only} fixture"),
                )
                .near(Some(format!("{only:?}"))),
            );
        }
    }
}
