use std::collections::{BTreeMap, BTreeSet};

use engine::core::content::ContentRow;
use engine::systems::area::{AreaDef, AreaId, AreaModuleKind};
use engine::systems::item::ItemDef;
use engine::systems::npc::NpcDef;
use engine::systems::quest::QuestDef;
use engine::systems::shop::ShopDef;
use serde::Deserialize;
use serde_json::Value;
use strum::VariantArray;

use super::atlas::{OpenTarget, RowSource, RowSourceKind, WorldAtlas};
use crate::app::Editor;
use crate::project::{ProjectFiles, row_file};
use crate::schema;

#[derive(Clone, Debug, PartialEq)]
pub enum MapLens {
    Row { table: String, key: String },
    Module(AreaModuleKind),
    Text(String),
}

impl MapLens {
    pub fn chip(&self) -> String {
        match self {
            MapLens::Row { table, key } => format!("{table}: {key}"),
            MapLens::Module(kind) => format!("module: {}", kind.tag()),
            MapLens::Text(text) => format!("text: {text}"),
        }
    }

    pub fn crumb(&self) -> String {
        match self {
            MapLens::Row { table, key } => format!("{table}/{key}"),
            MapLens::Module(kind) => format!("module/{}", kind.tag()),
            MapLens::Text(text) => format!("text/{text}"),
        }
    }

    fn group(&self) -> LensGroup<'_> {
        match self {
            MapLens::Row { table, .. } => LensGroup::Table(table),
            MapLens::Module(_) => LensGroup::Module,
            MapLens::Text(_) => LensGroup::Text,
        }
    }

    fn sources(&self, atlas: &WorldAtlas, area: AreaId) -> Vec<RowSource> {
        let info = &atlas.areas[&area];
        let here = |kind: RowSourceKind, name: String| RowSource {
            kind,
            name,
            opens: OpenTarget::Area {
                key: info.key.clone(),
                module: None,
            },
        };
        match self {
            MapLens::Row { table, key } => atlas.sources(area, table, key).to_vec(),
            MapLens::Module(kind) if info.modules.contains(kind) => {
                vec![here(RowSourceKind::Module, kind.tag().to_owned())]
            }
            MapLens::Text(text) if info.text.contains(&text.to_lowercase()) => {
                vec![here(RowSourceKind::Mention, text.clone())]
            }
            MapLens::Module(_) | MapLens::Text(_) => Vec::new(),
        }
    }
}

pub fn lit_areas(
    atlas: &WorldAtlas,
    lenses: &[MapLens],
) -> Option<BTreeMap<AreaId, Vec<RowSource>>> {
    if lenses.is_empty() {
        return None;
    }
    let mut groups: BTreeMap<LensGroup, Vec<&MapLens>> = BTreeMap::new();
    for lens in lenses {
        groups.entry(lens.group()).or_default().push(lens);
    }
    let mut lit = BTreeMap::new();
    for &area in atlas.areas.keys() {
        let mut sources = Vec::new();
        let every_group = groups.values().all(|group| {
            let before = sources.len();
            sources.extend(group.iter().flat_map(|lens| lens.sources(atlas, area)));
            sources.len() > before
        });
        if every_group {
            lit.insert(area, sources);
        }
    }
    Some(lit)
}

pub fn tag_line(sources: &[RowSource]) -> String {
    let mut counts: BTreeMap<RowSourceKind, usize> = BTreeMap::new();
    for source in sources {
        *counts.entry(source.kind).or_default() += 1;
    }
    counts
        .iter()
        .map(|(kind, count)| kind.tag(*count))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, VariantArray, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorTable {
    Npc,
    Item,
    Quest,
    Shop,
    Area,
}

impl ColorTable {
    pub fn table(self) -> &'static str {
        match self {
            ColorTable::Npc => NpcDef::TABLE,
            ColorTable::Item => ItemDef::TABLE,
            ColorTable::Quest => QuestDef::TABLE,
            ColorTable::Shop => ShopDef::TABLE,
            ColorTable::Area => AreaDef::TABLE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ColorField {
    pub table: ColorTable,
    pub path: Vec<String>,
}

impl ColorField {
    pub fn all(editor: &Editor) -> Vec<ColorField> {
        ColorTable::VARIANTS
            .iter()
            .filter_map(|&table| Some((table, editor.shapes.table(table.table())?)))
            .flat_map(|(table, shape)| {
                schema::number_paths(&shape.shape)
                    .into_iter()
                    .map(move |path| ColorField { table, path })
            })
            .collect()
    }

    pub fn name(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }

    pub fn path_text(&self) -> String {
        self.path.join(" · ")
    }

    pub fn text(&self) -> String {
        format!("{} · {}", self.table.table(), self.path_text())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorBy {
    pub field: ColorField,
    pub of: ColorOf,
    pub show: ColorShow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray)]
pub enum ColorOf {
    Placed,
    Hostile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray)]
pub enum ColorShow {
    Highest,
    Lowest,
}

impl ColorOf {
    pub fn label(self) -> &'static str {
        match self {
            ColorOf::Placed => "npcs placed here",
            ColorOf::Hostile => "hostile npcs placed here",
        }
    }
}

impl ColorShow {
    pub fn label(self) -> &'static str {
        match self {
            ColorShow::Highest => "the highest",
            ColorShow::Lowest => "the lowest",
        }
    }
}

impl ColorBy {
    pub fn new(field: ColorField) -> ColorBy {
        let of = match field.table {
            ColorTable::Npc => ColorOf::Hostile,
            _ => ColorOf::Placed,
        };
        ColorBy {
            field,
            of,
            show: ColorShow::Highest,
        }
    }

    pub fn chip(&self) -> String {
        format!("color: {} {}", self.rows(), self.field.name())
    }

    pub fn sentence(&self) -> String {
        format!(
            "colored by {} {} of {}s",
            self.show.label(),
            self.field.name(),
            self.rows()
        )
    }

    fn rows(&self) -> &'static str {
        match (self.field.table, self.of) {
            (ColorTable::Npc, ColorOf::Hostile) => "hostile npc",
            (table, _) => table.table(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct AreaRange {
    pub low: f64,
    pub high: f64,
}

impl AreaRange {
    pub fn text(&self) -> String {
        if self.low == self.high {
            self.low.to_string()
        } else {
            format!("{}–{}", self.low, self.high)
        }
    }

    pub fn shown(&self, show: ColorShow) -> f64 {
        match show {
            ColorShow::Highest => self.high,
            ColorShow::Lowest => self.low,
        }
    }

    fn widen(range: Option<AreaRange>, value: f64) -> AreaRange {
        match range {
            Some(range) => AreaRange {
                low: range.low.min(value),
                high: range.high.max(value),
            },
            None => AreaRange {
                low: value,
                high: value,
            },
        }
    }
}

pub struct ColorRanges {
    pub areas: BTreeMap<AreaId, AreaRange>,
    pub low: f64,
    pub high: f64,
}

impl ColorRanges {
    pub fn of(atlas: &WorldAtlas, files: &ProjectFiles, color: &ColorBy) -> ColorRanges {
        let areas: BTreeMap<AreaId, AreaRange> = atlas
            .areas
            .keys()
            .filter_map(|&area| Some((area, range_in(atlas, files, area, color)?)))
            .collect();
        let shown = areas.values().map(|range| range.shown(color.show));
        let low = shown.clone().fold(f64::INFINITY, f64::min);
        let high = shown.fold(f64::NEG_INFINITY, f64::max);
        if areas.is_empty() {
            ColorRanges {
                areas,
                low: 0.0,
                high: 1.0,
            }
        } else {
            ColorRanges { areas, low, high }
        }
    }

    pub fn fraction(&self, value: f64) -> f32 {
        ((value - self.low) / (self.high - self.low).max(1e-9)) as f32
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LensGroup<'a> {
    Table(&'a str),
    Module,
    Text,
}

fn range_in(
    atlas: &WorldAtlas,
    files: &ProjectFiles,
    area: AreaId,
    color: &ColorBy,
) -> Option<AreaRange> {
    let info = &atlas.areas[&area];
    let table = color.field.table;
    let keys: BTreeSet<&str> = match table {
        ColorTable::Area => BTreeSet::from([info.key.as_str()]),
        ColorTable::Npc => info
            .residents
            .iter()
            .chain(&info.populations)
            .filter(|placed| color.of == ColorOf::Placed || atlas.hostile(placed.npc))
            .map(|placed| placed.key.as_str())
            .collect(),
        _ => info.rows_found(table.table()).collect(),
    };
    let mut range = None;
    for key in keys {
        let Some(row) = files.json(&row_file(table.table(), key)) else {
            continue;
        };
        for value in numbers_at(&row, &color.field.path) {
            range = Some(AreaRange::widen(range, value));
        }
    }
    range
}

fn numbers_at(value: &Value, path: &[String]) -> Vec<f64> {
    let Some((head, rest)) = path.split_first() else {
        return match value {
            Value::Number(number) => number.as_f64().into_iter().collect(),
            Value::Array(items) => items.iter().filter_map(Value::as_f64).collect(),
            _ => Vec::new(),
        };
    };
    match value {
        Value::Object(fields) => match fields.get(head) {
            Some(field) => numbers_at(field, rest),
            None => fields
                .get("modules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|module| module["type"] == *head)
                .flat_map(|module| numbers_at(module, rest))
                .collect(),
        },
        Value::Array(items) => items
            .iter()
            .flat_map(|item| numbers_at(item, path))
            .collect(),
        _ => Vec::new(),
    }
}
