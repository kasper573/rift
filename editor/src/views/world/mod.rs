pub mod arrange;
pub mod atlas;
mod canvas;
mod filters;
mod inspect;
mod interact;
mod toolbar;

pub use arrange::{ArrangeDialog, dialog as arrange_dialog};
pub use filters::{ColorBy, ColorField, MapLens};
pub use inspect::inspect_parts;

use std::fmt;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Ui, Vec2};
use engine::core::content::{Content, ContentRow, Fixture};
use engine::core::math::{Pos, Size};
use engine::core::tiling::Tiles;
use engine::systems::area::world_map::{Card, Note, Region, RegionColor, WorldMap, WorldMapDef};
use engine::systems::area::{self, AreaDef, AreaId};
use serde::Deserialize;
use std::str::FromStr;

use crate::app::Editor;
use crate::project::{ContentEdit, Project, RowSelection, row_file};
use crate::ui::theme;
use crate::value;
use crate::views::area::TILE_PX;
use crate::views::navigation;
use atlas::WorldAtlas;

const AREA_PX_PER_TILE: f32 = 2.0;
const MIN_FOOTPRINT: Vec2 = Vec2::new(62.5, 38.75);
const NEW_CARD_SIZE: u32 = 100;
const REGION_PAD: f32 = 24.0;
const GAP: f32 = 3.0;
const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 8.0;

#[derive(Default)]
pub struct WorldView {
    pub center: Pos<Tiles>,
    pub zoom: f32,
    pub fitted: bool,
    pub lenses: Vec<MapLens>,
    pub search: String,
    pub color: Option<ColorBy>,
    pub route: Option<MapRoute>,
    pub route_from: Option<String>,
    pub quest_line: Option<String>,
    pub layers: MapLayers,
    pub hover: Option<MapTileHit>,
    pub menu_target: Option<MapTileHit>,
    pub pending_look: Option<MapLook>,
    atlas: Option<Arc<WorldAtlas>>,
    drag: Option<interact::MapDrag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct MapRoute {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapTileHit {
    pub area: String,
    pub at: Pos<Tiles>,
}

pub struct MapLook {
    pub area: String,
    pub zoom: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapLayers {
    pub warps: bool,
    pub residents: bool,
    pub populations: bool,
    pub givers: bool,
    pub regions: bool,
}

impl Default for MapLayers {
    fn default() -> MapLayers {
        MapLayers {
            warps: true,
            residents: false,
            populations: false,
            givers: false,
            regions: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapPart {
    Card(String),
    Edge(String, String),
    Region(usize),
    Note(usize),
}

impl fmt::Display for MapPart {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MapPart::Card(key) => write!(f, "card:{key}"),
            MapPart::Edge(from, to) => write!(f, "edge:{from}/{to}"),
            MapPart::Region(index) => write!(f, "region:{index}"),
            MapPart::Note(index) => write!(f, "note:{index}"),
        }
    }
}

impl FromStr for MapPart {
    type Err = String;

    fn from_str(text: &str) -> Result<MapPart, String> {
        let unknown = || format!("{text:?} is not a part of the world map");
        let (kind, rest) = text.split_once(':').ok_or_else(unknown)?;
        match kind {
            "card" => Ok(MapPart::Card(rest.to_owned())),
            "edge" => rest
                .split_once('/')
                .map(|(from, to)| MapPart::Edge(from.to_owned(), to.to_owned()))
                .ok_or_else(unknown),
            "region" => rest.parse().map(MapPart::Region).map_err(|_| unknown()),
            "note" => rest.parse().map(MapPart::Note).map_err(|_| unknown()),
            _ => Err(unknown()),
        }
    }
}

pub fn region_color(color: Option<RegionColor>) -> Color32 {
    match color {
        None => Color32::from_rgb(150, 150, 170),
        Some(RegionColor::Amber) => theme::WARN,
        Some(RegionColor::Green) => theme::OK,
        Some(RegionColor::Blue) => Color32::from_rgb(90, 160, 255),
        Some(RegionColor::Red) => theme::DANGER,
        Some(RegionColor::Violet) => Color32::from_rgb(170, 120, 240),
    }
}

pub fn region_color_name(color: Option<RegionColor>) -> &'static str {
    color.map_or("grey", <&str>::from)
}

pub struct OpenWorldMap {
    pub content: Content,
    pub atlas: Arc<WorldAtlas>,
    pub layout: WorldMapDef,
}

impl OpenWorldMap {
    pub fn of(editor: &mut Editor) -> Option<OpenWorldMap> {
        let project = editor.project.as_ref()?;
        let content = project.content.clone()?;
        let atlas = editor.views.world.atlas(project, &content);
        let layout = laid_out(&atlas, WorldMap::World.get(&content));
        Some(OpenWorldMap {
            content,
            atlas,
            layout,
        })
    }

    pub fn card_rect(&self, area: AreaId, card: &Card) -> Rect {
        card_rect(self.atlas.size(area), card)
    }

    pub fn art_rect(&self, area: AreaId, card: &Card) -> Rect {
        art_rect(self.atlas.size(area), card)
    }

    pub fn bounds(&self, layout: &WorldMapDef) -> Option<Rect> {
        layout
            .cards
            .iter()
            .map(|(area, card)| self.card_rect(*area, card))
            .reduce(|a, b| a.union(b))
    }

    pub fn region_rect(&self, layout: &WorldMapDef, region: &Region) -> Option<Rect> {
        region
            .cards
            .iter()
            .filter_map(|area| {
                layout
                    .cards
                    .get(area)
                    .map(|card| self.card_rect(*area, card))
            })
            .reduce(|a, b| a.union(b))
            .map(|rect| rect.expand(REGION_PAD))
    }

    pub fn card_ids(&self, parts: &[MapPart]) -> Vec<AreaId> {
        parts
            .iter()
            .filter_map(|part| match part {
                MapPart::Card(key) => self.atlas.id(key),
                _ => None,
            })
            .filter(|area| self.layout.cards.contains_key(area))
            .collect()
    }

    pub fn change(
        &self,
        editor: &mut Editor,
        edit: ContentEdit,
        change: impl FnOnce(&mut WorldMapDef),
    ) -> bool {
        let mut next = self.layout.clone();
        change(&mut next);
        self.write(editor, edit, &next)
    }

    pub fn write(&self, editor: &mut Editor, mut edit: ContentEdit, layout: &WorldMapDef) -> bool {
        let Some(json) = value::untyped(&self.content, layout) else {
            return false;
        };
        edit.write_json(layout_file(), &json);
        editor.apply(edit)
    }
}

impl WorldView {
    pub fn request_fit(&mut self) {
        self.fitted = false;
    }

    pub fn look_at(&mut self, map: &OpenWorldMap, area: AreaId, zoom: f32) {
        if let Some(card) = map.layout.cards.get(&area) {
            self.center = navigation::pos_from_egui(map.card_rect(area, card).center());
            self.zoom = zoom;
            self.fitted = true;
        }
    }

    fn atlas(&mut self, project: &Project, content: &Content) -> Arc<WorldAtlas> {
        match &self.atlas {
            Some(atlas) if atlas.generation == project.generation => atlas.clone(),
            _ => {
                let atlas = Arc::new(WorldAtlas::build(
                    content,
                    &project.files,
                    &project.usages,
                    project.generation,
                ));
                self.atlas = Some(atlas.clone());
                atlas
            }
        }
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    let Some(map) = OpenWorldMap::of(editor) else {
        ui.label(theme::muted(
            "The folder does not load; the map shows the last layout once it does.",
        ));
        return;
    };
    if let Some(look) = editor.views.world.pending_look.take()
        && let Some(area) = map.atlas.id(&look.area)
    {
        editor.views.world.look_at(&map, area, look.zoom);
    }
    toolbar::show(editor, ui, &map);
    canvas::show(editor, ui, &map);
}

pub fn selected(editor: &Editor) -> Vec<MapPart> {
    if editor.selection.table != WorldMapDef::TABLE {
        return Vec::new();
    }
    editor
        .selection
        .parts
        .iter()
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub fn select(editor: &mut Editor, parts: Vec<MapPart>) {
    editor.select(RowSelection::parts(
        WorldMapDef::TABLE,
        WorldMap::World.key(),
        parts.iter().map(MapPart::to_string).collect(),
    ));
}

pub fn card_beside(project: &Project, edit: &mut ContentEdit, original: &str, copy: &str) {
    let Some(content) = project.content.clone() else {
        return;
    };
    let Some(area) = content.by_key::<AreaDef>(original) else {
        return;
    };
    let Some(card) = WorldMap::World.get(&content).cards.get(&area) else {
        return;
    };
    let size = area::load(&content, area).size;
    let at = right_of(card_rect(size, card), footprint(size, card.size));
    let file = layout_file();
    let Some(mut row) = project.files.json_owned(&file) else {
        return;
    };
    let Ok(beside) = value::json(&Card {
        size: card.size,
        ..new_card(at)
    }) else {
        return;
    };
    row["cards"][copy] = beside;
    edit.write_json(file, &row);
}

fn layout_file() -> String {
    row_file(WorldMapDef::TABLE, WorldMap::World.key())
}

fn laid_out(atlas: &WorldAtlas, stored: &WorldMapDef) -> WorldMapDef {
    let mut layout = stored.clone();
    for &area in atlas.areas.keys() {
        if layout.cards.contains_key(&area) {
            continue;
        }
        let size = footprint(atlas.size(area), NEW_CARD_SIZE);
        let neighbour = atlas.exits(area).iter().find_map(|other| {
            layout
                .cards
                .get(other)
                .map(|card| card_rect(atlas.size(*other), card))
        });
        let bounds = layout
            .cards
            .iter()
            .map(|(other, card)| card_rect(atlas.size(*other), card))
            .reduce(|a, b| a.union(b));
        let at = match (neighbour, bounds) {
            (Some(neighbour), _) => right_of(neighbour, size),
            (None, Some(bounds)) => Pos2::new(
                bounds.max.x + GAP + size.x / 2.0,
                bounds.min.y + size.y / 2.0,
            ),
            (None, None) => Pos2::ZERO,
        };
        layout.cards.insert(area, new_card(at));
    }
    layout
}

fn shows_areas(zoom: f32) -> bool {
    zoom * TILE_PX > AREA_PX_PER_TILE
}

fn card_at(card: &Card) -> Pos2 {
    Pos2::new(card.at.x, card.at.y)
}

fn card_rect(size: Size<Tiles>, card: &Card) -> Rect {
    Rect::from_center_size(card_at(card), footprint(size, card.size))
}

fn art_rect(size: Size<Tiles>, card: &Card) -> Rect {
    Rect::from_center_size(card_at(card), art_size(size, card.size))
}

fn art_size(size: Size<Tiles>, percent: u32) -> Vec2 {
    Vec2::new(size.width, size.height) * (percent as f32 / 100.0)
}

fn footprint(size: Size<Tiles>, percent: u32) -> Vec2 {
    art_size(size, percent).max(MIN_FOOTPRINT)
}

fn right_of(rect: Rect, size: Vec2) -> Pos2 {
    Pos2::new(rect.max.x + GAP + size.x / 2.0, rect.center().y)
}

fn new_card(at: Pos2) -> Card {
    let mut card = Card {
        at: Pos::origin(),
        size: NEW_CARD_SIZE,
        pinned: false,
    };
    move_card(&mut card, at);
    card
}

fn move_card(card: &mut Card, at: Pos2) {
    card.at = Pos::new(hundredths(at.x), hundredths(at.y));
}

fn hundredths(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn tidy_regions(layout: &mut WorldMapDef) {
    let cards = &layout.cards;
    for region in &mut layout.regions {
        region.cards.retain(|area| cards.contains_key(area));
    }
    layout.regions.retain(|region| !region.cards.is_empty());
}

fn region_from_selection(editor: &mut Editor, map: &OpenWorldMap) {
    let cards = map.card_ids(&selected(editor));
    if cards.is_empty() {
        editor.toast("Select the cards the region holds first");
        return;
    }
    let mut next = map.layout.clone();
    for region in &mut next.regions {
        region.cards.retain(|area| !cards.contains(area));
    }
    next.regions.retain(|region| !region.cards.is_empty());
    let name = format!("Region {}", next.regions.len() + 1);
    let index = next.regions.len();
    next.regions.push(Region {
        name: name.clone(),
        cards,
        color: None,
    });
    if map.write(
        editor,
        ContentEdit::new(format!("Add {name} to the world map")),
        &next,
    ) {
        select(editor, vec![MapPart::Region(index)]);
    }
}

fn add_note(editor: &mut Editor, map: &OpenWorldMap, at: Pos<Tiles>) {
    let index = map.layout.notes.len();
    let added = map.change(editor, ContentEdit::new("Add a note"), |layout| {
        layout.notes.push(Note {
            text: "Note".to_owned(),
            x: hundredths(at.x),
            y: hundredths(at.y),
        });
    });
    if added {
        select(editor, vec![MapPart::Note(index)]);
    }
}
