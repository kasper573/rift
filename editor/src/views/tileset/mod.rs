pub mod animation;
pub mod inspect;
pub mod preview;
pub mod sheet;
pub mod sounds;
pub mod surfaces;
pub mod terrain;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use ui::chip;
use ui::egui;
use ui::form;
use ui::menu;
use ui::theme;
use ui::words;

use animation::OffsetUnit;
use engine::core::content::{ContentFile, FolderReader, fields_file};
use engine::systems::area::tileset::{MAX_TILE_PX, TileData, TilesetDef, TilesetGrid, Walk};
use serde::Deserialize;
use strum::{EnumString, IntoStaticStr, VariantArray};
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke, Ui, Vec2};

use self::preview::StripPreview;
use crate::app::{BottomTab, Editor};
use crate::import;
use crate::project::{ContentEdit, RowSelection};
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;
use crate::thumbs::{self, Thumbnail};

pub const TERRAINS_PART: &str = "/terrains";
const TILES_PART: &str = "/tiles/";
pub const ANIMATION_PART: &str = "/animation";

#[derive(Default)]
pub struct TilesetViews {
    pub views: HashMap<String, TilesetView>,
    pub reimports: HashMap<String, import::TilesetReimport>,
    pub counts: Option<TilesetCounts>,
}

impl TilesetViews {
    pub fn view(&self, key: &str) -> &TilesetView {
        self.views
            .get(key)
            .expect("ensure_view ran before the tileset view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut TilesetView {
        self.views.entry(key.to_owned()).or_default()
    }

    pub fn surface_use(&self, surface: &str) -> SurfaceUse {
        self.counts
            .as_ref()
            .and_then(|counts| counts.surfaces.get(surface).cloned())
            .unwrap_or_default()
    }
}

pub struct TilesetCounts {
    pub generation: u64,
    pub surfaces: HashMap<String, SurfaceUse>,
    pub with_surface: usize,
}

#[derive(Clone, Default)]
pub struct SurfaceUse {
    pub tiles: usize,
    pub tilesets: BTreeSet<String>,
}

impl SurfaceUse {
    pub fn text(&self) -> String {
        format!(
            "{} tiles in {} tilesets",
            words::thousands(self.tiles),
            self.tilesets.len()
        )
    }
}

#[derive(
    Clone, Copy, PartialEq, Eq, Default, Debug, EnumString, IntoStaticStr, VariantArray, Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TilesetMode {
    #[default]
    Walk,
    Surface,
    Terrain,
    Animation,
}

impl TilesetMode {
    pub fn label(self) -> &'static str {
        self.into()
    }

    fn part(self) -> Option<&'static str> {
        match self {
            TilesetMode::Terrain => Some(TERRAINS_PART),
            TilesetMode::Animation => Some(ANIMATION_PART),
            TilesetMode::Walk | TilesetMode::Surface => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, EnumString, VariantArray, Deserialize)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WalkBrush {
    Walkable,
    #[default]
    Blocks,
    Nothing,
}

impl WalkBrush {
    pub fn walk(self) -> Option<Walk> {
        match self {
            WalkBrush::Walkable => Some(Walk::Walkable),
            WalkBrush::Blocks => Some(Walk::Blocks),
            WalkBrush::Nothing => None,
        }
    }

    pub fn label(self) -> &'static str {
        walk_label(self.walk())
    }
}

pub fn walk_label(walk: Option<Walk>) -> &'static str {
    walk.map_or("says nothing", <&str>::from)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SheetCell {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SheetRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl SheetRect {
    pub fn spanning(a: SheetCell, b: SheetCell) -> SheetRect {
        SheetRect {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: a.x.abs_diff(b.x) + 1,
            h: a.y.abs_diff(b.y) + 1,
        }
    }

    pub fn offsets(self) -> impl Iterator<Item = SheetCell> {
        (0..self.h).flat_map(move |y| (0..self.w).map(move |x| SheetCell { x, y }))
    }

    pub fn tile_at(self, offset: SheetCell, columns: u32) -> u32 {
        (self.y + offset.y) * columns + self.x + offset.x
    }

    pub fn tiles(self, columns: u32) -> impl Iterator<Item = u32> {
        self.offsets()
            .map(move |offset| self.tile_at(offset, columns))
    }

    pub fn rows(self, columns: u32) -> Vec<Vec<u32>> {
        (0..self.h)
            .map(|y| {
                (0..self.w)
                    .map(|x| self.tile_at(SheetCell { x, y }, columns))
                    .collect()
            })
            .collect()
    }

    pub fn count(self) -> u32 {
        self.w * self.h
    }

    pub fn text(self) -> String {
        format!(
            "{} tiles · {}, {} · {} × {}",
            self.count(),
            self.x,
            self.y,
            self.w,
            self.h
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TileSheetDialog {
    MakeBlueprint(TileBlueprintDialog),
    NewSurface(NewSurfaceDialog),
    NewTerrainSet(NewTerrainSetDialog),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TileBlueprintDialog {
    pub name: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewSurfaceDialog {
    pub name: String,
    pub footstep: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewTerrainSetDialog {
    pub name: String,
    pub terrains: String,
    pub error: Option<String>,
}

pub struct TilesetView {
    pub mode: TilesetMode,
    pub zoom: f32,
    pub walk: WalkBrush,
    pub surface: Option<String>,
    pub set: Option<String>,
    pub terrain: usize,
    pub selection: Option<SheetRect>,
    pub anchor: Option<SheetCell>,
    pub tile: Option<u32>,
    pub closeup: Option<u32>,
    pub stroke: Option<Vec<u32>>,
    pub dialog: Option<TileSheetDialog>,
    pub preview: StripPreview,
    pub frame: Option<usize>,
    pub offsets_in: OffsetUnit,
    pub missing_shown: BTreeSet<String>,
}

impl Default for TilesetView {
    fn default() -> TilesetView {
        TilesetView {
            mode: TilesetMode::default(),
            zoom: 1.0,
            walk: WalkBrush::default(),
            surface: None,
            set: None,
            terrain: 1,
            selection: None,
            anchor: None,
            tile: None,
            closeup: None,
            stroke: None,
            dialog: None,
            preview: StripPreview::default(),
            frame: None,
            offsets_in: OffsetUnit::default(),
            missing_shown: BTreeSet::new(),
        }
    }
}

impl TilesetView {
    pub fn toggle_frame(&mut self, index: usize) {
        self.frame = (self.frame != Some(index)).then_some(index);
    }

    pub fn pick_set(&mut self, set: String) {
        self.set = Some(set);
        self.terrain = 1;
    }
}

#[derive(Clone)]
pub struct TilesetSheet {
    pub grid: TilesetGrid,
    pub thumb: Option<Thumbnail>,
}

impl TilesetSheet {
    pub fn of(editor: &mut Editor, ctx: &egui::Context, def: &TilesetDef) -> TilesetSheet {
        let thumb = editor.thumb(ctx, def.image.as_str());
        let grid = match &thumb {
            Some(thumb) => def.grid(thumb.size[0] as u32, thumb.size[1] as u32),
            None => def.grid(def.tile, def.tile),
        };
        TilesetSheet { grid, thumb }
    }

    pub fn columns(&self) -> u32 {
        self.grid.columns
    }

    pub fn count(&self) -> u32 {
        self.grid.count()
    }

    pub fn cell(&self, tile: u32) -> SheetCell {
        SheetCell {
            x: tile % self.grid.columns,
            y: tile / self.grid.columns,
        }
    }

    pub fn tile(&self, cell: SheetCell) -> Option<u32> {
        (cell.x < self.grid.columns && cell.y < self.grid.rows)
            .then_some(cell.y * self.grid.columns + cell.x)
    }

    pub fn fit(&self, width: f32, smallest: f32) -> f32 {
        (width / self.grid.columns as f32)
            .clamp(smallest, 34.0)
            .floor()
    }

    pub fn size(&self, shown: f32) -> Vec2 {
        Vec2::new(
            self.grid.columns as f32 * shown,
            self.grid.rows as f32 * shown,
        )
    }

    pub fn draw(&self, painter: &egui::Painter, rect: ERect, tile: u32) {
        if let Some(thumb) = &self.thumb {
            let region = self.grid.region(tile);
            let uv = thumbs::uv(
                thumb.size,
                region.origin.x,
                region.origin.y,
                region.size.width,
                region.size.height,
            );
            painter.image(thumb.texture.id(), rect, uv, Color32::WHITE);
        }
    }

    pub fn draw_sheet(&self, painter: &egui::Painter, rect: ERect) {
        if let Some(thumb) = &self.thumb {
            let used = (self.grid.columns * self.grid.tile) as f32;
            let tall = (self.grid.rows * self.grid.tile) as f32;
            let uv = thumbs::uv(thumb.size, 0.0, 0.0, used, tall);
            painter.image(thumb.texture.id(), rect, uv, Color32::WHITE);
        }
    }
}

#[derive(Clone, Copy)]
pub struct SheetLayout {
    pub rect: ERect,
    pub shown: f32,
}

impl SheetLayout {
    pub fn cell_rect(&self, cell: SheetCell) -> ERect {
        ERect::from_min_size(
            self.rect.min + Vec2::new(cell.x as f32 * self.shown, cell.y as f32 * self.shown),
            Vec2::splat(self.shown),
        )
    }

    pub fn tile_rect(&self, sheet: &TilesetSheet, tile: u32) -> ERect {
        self.cell_rect(sheet.cell(tile))
    }

    pub fn block_rect(&self, block: SheetRect) -> ERect {
        ERect::from_min_size(
            self.cell_rect(SheetCell {
                x: block.x,
                y: block.y,
            })
            .min,
            Vec2::new(block.w as f32 * self.shown, block.h as f32 * self.shown),
        )
    }

    pub fn tile_at(&self, sheet: &TilesetSheet, pos: Pos2) -> Option<u32> {
        if !self.rect.contains(pos) {
            return None;
        }
        sheet.tile(self.unclamped_cell(pos))
    }

    pub fn clamped_cell(&self, sheet: &TilesetSheet, pos: Pos2) -> SheetCell {
        let cell = self.unclamped_cell(pos.clamp(self.rect.min, self.rect.max));
        SheetCell {
            x: cell.x.min(sheet.grid.columns - 1),
            y: cell.y.min(sheet.grid.rows - 1),
        }
    }

    pub fn draw_lines(&self, painter: &egui::Painter, sheet: &TilesetSheet, stroke: Stroke) {
        for x in 0..=sheet.grid.columns {
            let sx = self.rect.min.x + x as f32 * self.shown;
            painter.line_segment(
                [
                    Pos2::new(sx, self.rect.min.y),
                    Pos2::new(sx, self.rect.max.y),
                ],
                stroke,
            );
        }
        for y in 0..=sheet.grid.rows {
            let sy = self.rect.min.y + y as f32 * self.shown;
            painter.line_segment(
                [
                    Pos2::new(self.rect.min.x, sy),
                    Pos2::new(self.rect.max.x, sy),
                ],
                stroke,
            );
        }
    }

    fn unclamped_cell(&self, pos: Pos2) -> SheetCell {
        let offset = (pos - self.rect.min) / self.shown;
        SheetCell {
            x: offset.x.max(0.0) as u32,
            y: offset.y.max(0.0) as u32,
        }
    }
}

pub struct OpenTileset {
    pub key: String,
    pub def: Arc<TilesetDef>,
    pub reader: Arc<FolderReader>,
    pub sheet: TilesetSheet,
}

impl OpenTileset {
    pub fn read(editor: &mut Editor, ctx: &egui::Context, key: &str) -> Option<OpenTileset> {
        let reader = editor.reader()?;
        let def = editor.read::<TilesetDef>(key)?.ok()?;
        let sheet = TilesetSheet::of(editor, ctx, &def);
        Some(OpenTileset {
            key: key.to_owned(),
            def,
            reader,
            sheet,
        })
    }

    pub fn surface(&self, tile: u32) -> Option<&str> {
        self.def
            .surface(tile)
            .and_then(|surface| self.reader.path_of(surface))
    }

    pub fn terrains(&self, set: &str) -> &[String] {
        self.def.terrains.get(set).map_or(&[], Vec::as_slice)
    }

    pub fn corners(&self, tile: u32, set: &str) -> Option<&[String; 4]> {
        self.def.data(tile)?.corners.get(set)
    }

    pub fn animated(&self, tile: u32) -> bool {
        !self.def.animation(tile).is_empty()
    }
}

pub fn change_tileset(
    editor: &mut Editor,
    key: &str,
    edit: ContentEdit,
    change: impl FnOnce(&mut TilesetDef, &FolderReader),
) -> bool {
    try_change_tileset(editor, key, edit, change).is_ok()
}

pub fn try_change_tileset(
    editor: &mut Editor,
    key: &str,
    mut edit: ContentEdit,
    change: impl FnOnce(&mut TilesetDef, &FolderReader),
) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let reader = project.reader();
    let read = project.read::<TilesetDef>(key);
    let mut def = Arc::unwrap_or_clone(read.map_err(|_| unreadable(editor, key))?);
    change(&mut def, &reader);
    def.tiles.retain(|_, data| !is_blank(data));
    edit.write_content_in(&reader, key, &def)?;
    editor.try_apply(edit)
}

pub fn tile_mut(def: &mut TilesetDef, tile: u32) -> &mut TileData {
    def.tiles.entry(tile).or_default()
}

pub fn tile_size(tileset: &TilesetDef) -> Result<u32, String> {
    Some(tileset.tile)
        .filter(|tile| (1..=MAX_TILE_PX).contains(tile))
        .ok_or_else(|| format!("the tileset's tile is not 1 to {MAX_TILE_PX} px"))
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if !editor.views.tileset.views.contains_key(key) {
        reset_view(editor, key);
    }
}

/// Starts the view of the tileset at `key` afresh: nothing picked, at its first mode and zoom.
pub fn reset_view(editor: &mut Editor, key: &str) {
    let surface = sounds::surfaces(editor)
        .into_iter()
        .next()
        .map(|surface| surface.key);
    let view = TilesetView {
        surface,
        ..TilesetView::default()
    };
    editor.views.tileset.views.insert(key.to_owned(), view);
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    ensure_view(editor, key);
    refresh_counts(editor);
    let ctx = ui.ctx().clone();
    sidebar::row(ui, "tileset", |ui| {
        row_form::show_row(editor, ui, TilesetDef::TABLE, key)
    });
    let Some(tileset) = OpenTileset::read(editor, &ctx, key) else {
        ui.label(theme::muted(unreadable(editor, key)));
        return;
    };
    let reimporting = editor.views.tileset.reimports.contains_key(key);
    let selection = editor.selection.clone();
    sidebar::inspector(editor, ui, "tileset", |editor, ui| {
        if selection.table == TilesetDef::TABLE
            && selection.keys.first().is_some_and(|k| k == key)
            && !selection.parts.is_empty()
        {
            inspect_parts(editor, ui, &selection);
        } else {
            form::nothing_selected(
                ui,
                "Select tiles, a terrain or an animation to inspect it here.",
            );
        }
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            if reimporting {
                import::show_tileset_reimport(editor, ui, key);
                return;
            }
            header(editor, ui, &tileset);
            ui.add_space(6.0);
            match editor.views.tileset.view(key).mode {
                TilesetMode::Terrain => terrain::show(editor, ui, &tileset),
                TilesetMode::Animation => animation::show(editor, ui, &tileset),
                TilesetMode::Walk | TilesetMode::Surface => sheet::show(editor, ui, &tileset),
            }
        });
    if !reimporting {
        status(editor, &tileset);
    }
    dialogs(editor, &ctx, &tileset);
}

pub fn set_mode(editor: &mut Editor, key: &str, mode: TilesetMode) {
    let view = editor.views.tileset.view_mut(key);
    view.mode = mode;
    view.anchor = None;
    view.stroke = None;
    let tile = view.tile;
    match (mode.part(), tile) {
        (Some(part), _) => select_part(editor, key, part),
        (None, Some(tile)) => select_tile(editor, key, tile),
        (None, None) => editor.select(RowSelection::row(TilesetDef::TABLE, key)),
    }
}

pub fn select_tile(editor: &mut Editor, key: &str, tile: u32) {
    editor.views.tileset.view_mut(key).tile = Some(tile);
    select_part(editor, key, &format!("{TILES_PART}{tile}"));
}

pub fn tile_of_part(part: &str) -> Option<u32> {
    part.strip_prefix(TILES_PART)?.parse().ok()
}

pub fn select_part(editor: &mut Editor, key: &str, part: &str) {
    editor.select(RowSelection::parts(
        TilesetDef::TABLE,
        key,
        vec![part.to_owned()],
    ));
}

pub fn inspect_parts(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    if let Some(key) = selection.keys.first()
        && selection
            .parts
            .first()
            .is_some_and(|part| part == import::IMPORT_PART)
    {
        let key = key.clone();
        egui::ScrollArea::vertical()
            .id_salt(("tileset-import", &key))
            .show(ui, |ui| import::inspect_tileset_reimport(editor, ui, &key));
        return;
    }
    inspect::show(editor, ui, selection);
}

pub fn bottom_tabs(editor: &Editor, key: &str) -> Vec<BottomTab> {
    match editor.views.tileset.views.get(key).map(|view| view.mode) {
        Some(TilesetMode::Surface) => vec![BottomTab::Surfaces],
        Some(TilesetMode::Terrain) => vec![BottomTab::Mixes],
        Some(TilesetMode::Animation) => vec![BottomTab::Timeline],
        Some(TilesetMode::Walk) | None => Vec::new(),
    }
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, key: &str, tab: BottomTab) {
    ensure_view(editor, key);
    let ctx = ui.ctx().clone();
    let Some(tileset) = OpenTileset::read(editor, &ctx, key) else {
        return;
    };
    match tab {
        BottomTab::Surfaces => surfaces::bottom(editor, ui, key),
        BottomTab::Mixes => terrain::mixes(editor, ui, &tileset),
        BottomTab::Timeline => animation::timeline(editor, ui, &tileset),
        _ => {}
    }
}

fn is_blank(data: &TileData) -> bool {
    data.walk.is_none()
        && data.surface.is_none()
        && data.corners.is_empty()
        && data.animation.is_empty()
}

fn unreadable(editor: &Editor, key: &str) -> String {
    let present = editor
        .project
        .as_ref()
        .is_some_and(|project| project.files.contains(&fields_file(key)));
    if !present {
        format!("{} is gone", shown_path(key))
    } else {
        format!(
            "{} does not read as a tileset; its row on the left shows why",
            shown_path(key)
        )
    }
}

fn header(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let mode = editor.views.tileset.view(key).mode;
    let mut picked = None;
    let mut import = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            menu::dropdown(ui, "Import", |ui| {
                if ui.button("Onto this tileset…").clicked() {
                    import = Some(Reimport::Tiles);
                    ui.close();
                }
                if ui.button("An animation onto its tiles…").clicked() {
                    import = Some(Reimport::Animation);
                    ui.close();
                }
            });
            ui.add_space(8.0);
            for option in TilesetMode::VARIANTS.iter().rev() {
                if chip::mode(ui, option.label(), *option == mode).clicked() {
                    picked = Some(*option);
                }
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(theme::mono(shown_path(key)).color(theme::CODE)).truncate(),
                );
            });
        });
    });
    ui.horizontal_wrapped(|ui| match mode {
        TilesetMode::Walk => sheet::walk_legend(editor, ui, key),
        TilesetMode::Surface => sheet::surface_legend(editor, ui, key),
        TilesetMode::Terrain => terrain::legend(editor, ui, tileset),
        TilesetMode::Animation => animation::legend(ui),
    });
    if let Some(mode) = picked {
        set_mode(editor, key, mode);
    }
    match import {
        Some(Reimport::Tiles) => import::start_for_tileset(editor, key),
        Some(Reimport::Animation) => import::start_animation(editor, key),
        None => {}
    }
}

enum Reimport {
    Tiles,
    Animation,
}

fn refresh_counts(editor: &mut Editor) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    if editor
        .views
        .tileset
        .counts
        .as_ref()
        .is_some_and(|counts| counts.generation == project.generation)
    {
        return;
    }
    let mut surfaces: HashMap<String, SurfaceUse> = HashMap::new();
    let mut with_surface = 0;
    for id in content.ids::<TilesetDef>() {
        let tileset = id.path(&content);
        for surface in id
            .get(&content)
            .tiles
            .values()
            .filter_map(|data| data.surface)
        {
            with_surface += 1;
            let entry = surfaces
                .entry(surface.path(&content).to_owned())
                .or_default();
            entry.tiles += 1;
            entry.tilesets.insert(tileset.to_owned());
        }
    }
    editor.views.tileset.counts = Some(TilesetCounts {
        generation: project.generation,
        surfaces,
        with_surface,
    });
}

fn status(editor: &mut Editor, tileset: &OpenTileset) {
    let view = editor.views.tileset.view(&tileset.key);
    let tiles = || tileset.def.tiles.values();
    let mut left = Vec::new();
    if let Some(tile) = view.tile {
        let cell = tileset.sheet.cell(tile);
        left.push(format!("tile {tile} · {}, {}", cell.x, cell.y));
    }
    match view.mode {
        TilesetMode::Walk => {
            let walk_said = tiles().filter(|data| data.walk.is_some()).count();
            let with_surface = tiles().filter(|data| data.surface.is_some()).count();
            left.push(format!("painting walk: {}", view.walk.label()));
            left.push(format!(
                "{} tiles · {walk_said} say walkable or blocks · {with_surface} have a surface",
                tileset.sheet.count()
            ));
        }
        TilesetMode::Surface => {
            if let Some(tile) = view.tile {
                left.push(format!(
                    "surface: {}",
                    tileset.surface(tile).unwrap_or("none")
                ));
            }
            left.push(format!(
                "brush: {}",
                view.surface.as_deref().unwrap_or("none")
            ));
            let with_surface = editor
                .views
                .tileset
                .counts
                .as_ref()
                .map_or(0, |counts| counts.with_surface);
            left.push(format!(
                "{} surfaces · {} tiles have one",
                sounds::surfaces(editor).len(),
                words::thousands(with_surface)
            ));
        }
        TilesetMode::Terrain => {
            if let Some(set) = &view.set {
                let painting = tileset
                    .terrains(set)
                    .get(view.terrain)
                    .map_or("", String::as_str);
                left.push(format!("terrain: {set} · painting {painting}"));
            }
            let with_corners = tiles().filter(|data| !data.corners.is_empty()).count();
            left.push(format!(
                "{} terrain sets · {with_corners} tiles with corners",
                tileset.def.terrains.len()
            ));
        }
        TilesetMode::Animation => {
            if let Some(selection) = view.selection {
                let animated = selection
                    .tiles(tileset.sheet.columns())
                    .filter(|tile| tileset.animated(*tile))
                    .count();
                left.push(format!(
                    "{animated} animated tiles selected · {}, {} to {}, {}",
                    selection.x,
                    selection.y,
                    selection.x + selection.w - 1,
                    selection.y + selection.h - 1
                ));
            }
            let animated = tiles().filter(|data| !data.animation.is_empty()).count();
            left.push(format!("{animated} animated tiles in this tileset"));
        }
    }
    editor.status.left.extend(left);
}

fn dialogs(editor: &mut Editor, ctx: &egui::Context, tileset: &OpenTileset) {
    let Some(dialog) = editor.views.tileset.view(&tileset.key).dialog.clone() else {
        return;
    };
    let next = match dialog {
        TileSheetDialog::MakeBlueprint(dialog) => {
            sheet::blueprint_dialog(editor, ctx, tileset, dialog)
                .map(TileSheetDialog::MakeBlueprint)
        }
        TileSheetDialog::NewSurface(dialog) => {
            surfaces::new_surface_dialog(editor, ctx, &tileset.key, dialog)
                .map(TileSheetDialog::NewSurface)
        }
        TileSheetDialog::NewTerrainSet(dialog) => {
            terrain::new_set_dialog(editor, ctx, tileset, dialog)
                .map(TileSheetDialog::NewTerrainSet)
        }
    };
    editor.views.tileset.view_mut(&tileset.key).dialog = next;
}
