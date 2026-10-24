use engine::core::content::{ContentFile, FolderReader, parent_of};
use engine::core::units::{Cell, CellPos};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::grid::Flip;
use engine::systems::area::scene::{ModuleNode, Scene, SceneGrids, SceneNode};
use engine::systems::area::tileset::{SurfaceDef, TileData, TilesetDef, Walk};
use engine::systems::area::{AreaModule, MapSize, Scenery, SceneryTile, Tilemap};
use strum::VariantArray;
use ui::dialog;
use ui::egui;
use ui::egui::{Color32, Pos2, Rect as ERect, RichText, Sense, Stroke, Ui, Vec2};
use ui::form;
use ui::ink;
use ui::input;
use ui::paint;
use ui::theme;

use super::sounds;
use super::{
    ANIMATION_PART, OpenTileset, SheetLayout, SheetRect, TileBlueprintDialog, TilesetMode,
    TilesetView, WalkBrush, change_tileset, tile_mut,
};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::{ContentEdit, RowSelection};
use crate::rows;
use crate::tabs::file_title;
use ui::input::InputField;

const WALKABLE: Color32 = Color32::from_rgb(110, 200, 110);
const BLOCKS: Color32 = Color32::from_rgb(230, 90, 90);
const SURFACE_COLORS: [Color32; 6] = [
    Color32::from_rgb(230, 190, 90),
    Color32::from_rgb(120, 200, 110),
    Color32::from_rgb(170, 180, 200),
    Color32::from_rgb(200, 140, 80),
    Color32::from_rgb(200, 120, 200),
    Color32::from_rgb(90, 180, 220),
];
const MIN_TILE: f32 = 8.0;
const MAX_TILE: f32 = 64.0;

pub struct SheetCanvas {
    pub layout: SheetLayout,
    pub hover: Option<u32>,
    pub response: egui::Response,
}

pub enum TileBrush {
    Walk(WalkBrush),
    Surface(Option<String>),
}

impl TileBrush {
    pub fn of(view: &TilesetView) -> Option<TileBrush> {
        match view.mode {
            TilesetMode::Walk => Some(TileBrush::Walk(view.walk)),
            TilesetMode::Surface => Some(TileBrush::Surface(view.surface.clone())),
            TilesetMode::Terrain | TilesetMode::Animation => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            TileBrush::Walk(brush) => format!("walk: {}", brush.label()),
            TileBrush::Surface(surface) => {
                format!("surface: {}", surface.as_deref().unwrap_or("none"))
            }
        }
    }

    pub fn changes(&self, tileset: &OpenTileset, tile: u32) -> bool {
        match self {
            TileBrush::Walk(brush) => tileset.def.walk(tile) != brush.walk(),
            TileBrush::Surface(surface) => tileset.surface(tile) != surface.as_deref(),
        }
    }

    pub fn paint(&self, data: &mut TileData, reader: &FolderReader) {
        match self {
            TileBrush::Walk(brush) => data.walk = brush.walk(),
            TileBrush::Surface(surface) => {
                data.surface = surface
                    .as_deref()
                    .and_then(|key| reader.id_of::<SurfaceDef>(key));
            }
        }
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let mode = editor.views.tileset.view(&tileset.key).mode;
    let surfaces: Vec<String> = sounds::surfaces(editor)
        .into_iter()
        .map(|surface| surface.key)
        .collect();
    let canvas = canvas(editor, ui, tileset, |painter, tile, cell| match mode {
        TilesetMode::Walk => walk_mark(painter, cell, tileset.def.walk(tile)),
        TilesetMode::Surface => {
            if let Some(surface) = tileset.surface(tile) {
                surface_mark(painter, cell, surface_index(&surfaces, surface));
            }
        }
        TilesetMode::Terrain | TilesetMode::Animation => {}
    });
    paint(editor, ui, tileset, &canvas);
    select(editor, ui, tileset, &canvas);
    draw_selection(editor, ui, &tileset.key, &canvas);
}

pub fn canvas(
    editor: &mut Editor,
    ui: &mut Ui,
    tileset: &OpenTileset,
    mut mark: impl FnMut(&egui::Painter, u32, ERect),
) -> SheetCanvas {
    let key = tileset.key.as_str();
    let sheet = &tileset.sheet;
    let zoom = editor.views.tileset.view(key).zoom;
    let shown = (sheet.fit(ui.available_width() - 16.0, MIN_TILE) * zoom)
        .clamp(MIN_TILE, MAX_TILE)
        .floor();
    egui::ScrollArea::both()
        .id_salt(("tileset-sheet", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(sheet.size(shown), Sense::click_and_drag());
            let layout = SheetLayout { rect, shown };
            let painter = ui.painter_at(rect.expand(2.0));
            painter.rect_filled(rect, 0.0, theme::CANVAS);
            sheet.draw_sheet(&painter, rect);
            for tile in 0..sheet.count() {
                let cell = layout.tile_rect(sheet, tile);
                if cell.intersects(ui.clip_rect()) {
                    mark(&painter, tile, cell);
                }
            }
            layout.draw_lines(
                &painter,
                sheet,
                Stroke::new(1.0, Color32::from_black_alpha(90)),
            );
            let hover = response
                .hover_pos()
                .and_then(|pos| layout.tile_at(sheet, pos));
            if let Some(tile) = hover {
                painter.rect_stroke(
                    layout.tile_rect(sheet, tile),
                    0.0,
                    Stroke::new(1.5, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
            let (held, scroll, gesture) = ui.input(|i| {
                (
                    EditorKey::Zoom.held(i),
                    i.smooth_scroll_delta.y,
                    i.zoom_delta(),
                )
            });
            let factor = match (gesture != 1.0, held && scroll != 0.0) {
                (true, _) => gesture,
                (false, true) if scroll > 0.0 => 1.25,
                (false, true) => 0.8,
                (false, false) => 1.0,
            };
            if factor != 1.0 && response.hovered() {
                let view = editor.views.tileset.view_mut(key);
                view.zoom = (view.zoom * factor).clamp(0.25, 8.0);
            }
            SheetCanvas {
                layout,
                hover,
                response,
            }
        })
        .inner
}

pub fn select(editor: &mut Editor, ui: &Ui, tileset: &OpenTileset, canvas: &SheetCanvas) {
    let key = tileset.key.as_str();
    let sheet = &tileset.sheet;
    let mode = editor.views.tileset.view(key).mode;
    let pointer = Pointer::read(ui);
    let selects = pointer.extend || mode == TilesetMode::Animation;
    if selects
        && pointer.pressed
        && canvas.response.hovered()
        && let Some(tile) = canvas.hover
    {
        let cell = sheet.cell(tile);
        let view = editor.views.tileset.view_mut(key);
        view.anchor = Some(cell);
        view.selection = Some(SheetRect::spanning(cell, cell));
        view.tile = Some(tile);
    }
    let anchor = editor.views.tileset.view(key).anchor;
    if let Some(anchor) = anchor
        && pointer.down
        && let Some(pos) = pointer.pos
    {
        let cell = canvas.layout.clamped_cell(sheet, pos);
        editor.views.tileset.view_mut(key).selection = Some(SheetRect::spanning(anchor, cell));
    }
    if anchor.is_some() && pointer.released {
        let view = editor.views.tileset.view_mut(key);
        view.anchor = None;
        match (mode, view.tile) {
            (TilesetMode::Animation, _) => super::select_part(editor, key, ANIMATION_PART),
            (_, Some(tile)) => super::select_tile(editor, key, tile),
            (_, None) => {}
        }
    }
    if !selects
        && canvas.response.clicked()
        && let Some(tile) = canvas.hover
    {
        super::select_tile(editor, key, tile);
    }
}

pub fn draw_selection(editor: &Editor, ui: &Ui, key: &str, canvas: &SheetCanvas) {
    let Some(selection) = editor.views.tileset.view(key).selection else {
        return;
    };
    let rect = canvas.layout.block_rect(selection);
    let painter = ui.painter_at(canvas.layout.rect.expand(2.0));
    painter.rect_filled(rect, 0.0, ink::wash(theme::SELECTION, 0.12));
    paint::dashed_rect(&painter, rect, Stroke::new(2.0, theme::SELECTION), 6.0, 4.0);
}

pub fn walk_legend(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let view = editor.views.tileset.view_mut(key);
    for brush in WalkBrush::VARIANTS {
        let on = view.walk == *brush;
        if legend_row(ui, on, brush.label(), |painter, swatch| {
            walk_mark(painter, swatch, brush.walk());
        }) {
            view.walk = *brush;
        }
    }
}

pub fn surface_legend(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let surfaces = sounds::surfaces(editor);
    let view = editor.views.tileset.view_mut(key);
    for (index, surface) in surfaces.iter().enumerate() {
        let on = view.surface.as_deref() == Some(surface.key.as_str());
        if legend_row(ui, on, &surface.name.to_lowercase(), |painter, swatch| {
            surface_mark(painter, swatch, index);
        }) {
            view.surface = Some(surface.key.clone());
        }
    }
    if legend_row(ui, view.surface.is_none(), "none", |_, _| {}) {
        view.surface = None;
    }
}

pub fn blueprint_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    tileset: &OpenTileset,
    mut dialog: TileBlueprintDialog,
) -> Option<TileBlueprintDialog> {
    let key = tileset.key.as_str();
    let selection = editor.views.tileset.view(key).selection?;
    let mut answer = None;
    dialog::card(ctx, "tileset-blueprint", 420.0, |ui| {
        ui.label(
            RichText::new("Make a blueprint from these tiles")
                .size(16.0)
                .strong(),
        );
        ui.add_space(6.0);
        ui.label(theme::muted(format!(
            "{} tiles of {key}, {} × {} from {}, {}, become one scenery part, sorted at its \
             bottom row.",
            selection.count(),
            selection.w,
            selection.h,
            selection.x,
            selection.y
        )));
        ui.add_space(8.0);
        form::kv(ui, "key", |ui| {
            let response = InputField::new(&mut dialog.name).width(240.0).show(ui);
            input::default_focus(ui, &response);
        });
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(10.0);
        answer = dialog::answer_row(ui, "Make blueprint");
    });
    match answer {
        None => Some(dialog),
        Some(dialog::Answer::Cancel) => None,
        Some(dialog::Answer::Confirm) => {
            match make_blueprint(editor, tileset, selection, dialog.name.trim()) {
                Ok(()) => None,
                Err(error) => Some(TileBlueprintDialog {
                    error: Some(error),
                    ..dialog
                }),
            }
        }
    }
}

struct Pointer {
    pressed: bool,
    down: bool,
    released: bool,
    pos: Option<Pos2>,
    extend: bool,
}

impl Pointer {
    fn read(ui: &Ui) -> Pointer {
        ui.input(|i| Pointer {
            pressed: i.pointer.primary_pressed(),
            down: i.pointer.primary_down(),
            released: i.pointer.primary_released(),
            pos: i.pointer.latest_pos(),
            extend: EditorKey::ExtendSelection.held(i),
        })
    }
}

fn paint(editor: &mut Editor, ui: &Ui, tileset: &OpenTileset, canvas: &SheetCanvas) {
    let key = tileset.key.as_str();
    let Some(brush) = TileBrush::of(editor.views.tileset.view(key)) else {
        return;
    };
    let pointer = Pointer::read(ui);
    if pointer.extend {
        return;
    }
    if pointer.pressed && canvas.response.hovered() {
        editor.views.tileset.view_mut(key).stroke = Some(Vec::new());
    }
    let stroke = editor.views.tileset.view(key).stroke.as_ref();
    if (pointer.down || pointer.pressed)
        && let Some(tile) = pointer
            .pos
            .and_then(|pos| canvas.layout.tile_at(&tileset.sheet, pos))
        && stroke.is_some_and(|stroke| !stroke.contains(&tile))
    {
        if brush.changes(tileset, tile) {
            let edit = ContentEdit::new(format!("Paint {} on {key}", brush.label())).merging();
            change_tileset(editor, key, edit, |def, reader| {
                brush.paint(tile_mut(def, tile), reader);
            });
        }
        if let Some(stroke) = &mut editor.views.tileset.view_mut(key).stroke {
            stroke.push(tile);
        }
        super::select_tile(editor, key, tile);
    }
    if pointer.released {
        editor.views.tileset.view_mut(key).stroke = None;
    }
}

fn make_blueprint(
    editor: &mut Editor,
    tileset: &OpenTileset,
    selection: SheetRect,
    name: &str,
) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let folder = parent_of(&tileset.key);
    let path = rows::new_path(&project.files, folder, BlueprintDef::TABLE, name);
    if let Some(problem) =
        rows::name_problem(&project.files, folder, name, rows::name_kind(&path), None)
    {
        return Err(problem);
    }
    let reader = project.reader();
    let id = reader
        .id_of::<TilesetDef>(&tileset.key)
        .ok_or("the tileset doesn't read")?;
    let tile = project
        .read::<TilesetDef>(&tileset.key)
        .map_err(|_| "the tileset doesn't read")?
        .tile;
    let tiles = selection
        .rows(tileset.sheet.columns())
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|tile| {
                    Some(SceneryTile {
                        tile,
                        flip: Flip::None,
                    })
                })
                .collect()
        })
        .collect();
    let part = AreaModule::Scenery(Scenery {
        at: CellPos::new(0, 0).bounds().min(),
        tileset: id,
        tiles,
        sort: Some(selection.h as f32 - 0.5),
        flip: Flip::None,
        blocks: Vec::new(),
    });
    let map = Tilemap {
        size: MapSize {
            width: selection.w,
            height: selection.h,
        },
        tile,
    };
    let made = BlueprintDef {
        variables: Default::default(),
        scene: Scene {
            nodes: vec![SceneNode::Module(ModuleNode::new(part))],
            grids: SceneGrids::empty(&map),
        },
        map,
    };
    let mut edit = ContentEdit::new(format!(
        "Make blueprint {name} from {} tiles of {}",
        selection.count(),
        file_title(&tileset.key)
    ));
    edit.write_content(project, &path, &made)?;
    editor.try_apply(edit)?;
    editor.toast(format!("{} is ready to stamp", file_title(&path)));
    editor.select(RowSelection::row(BlueprintDef::TABLE, &path));
    Ok(())
}

fn legend_row(
    ui: &mut Ui,
    on: bool,
    label: &str,
    swatch: impl FnOnce(&egui::Painter, ERect),
) -> bool {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        let painter = ui.painter();
        swatch(painter, rect);
        painter.rect_stroke(
            rect,
            2.0,
            Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        if on {
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(1.5, Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
        ui.label(if on {
            theme::strong(label)
        } else {
            theme::muted(label)
        });
    })
    .response
    .interact(Sense::click())
    .clicked()
}

fn walk_mark(painter: &egui::Painter, cell: ERect, walk: Option<Walk>) {
    match walk {
        Some(Walk::Walkable) => paint::dotted(painter, cell, WALKABLE),
        Some(Walk::Blocks) => paint::hatched(painter, cell, BLOCKS, cell.width()),
        None => {}
    }
}

fn surface_mark(painter: &egui::Painter, cell: ERect, index: usize) {
    let color = SURFACE_COLORS[index % SURFACE_COLORS.len()];
    match index % 4 {
        0 => paint::dotted(painter, cell, color),
        1 => paint::hatched(painter, cell, color, cell.width()),
        2 => paint::gridded(painter, cell, color),
        _ => paint::striped(painter, cell, color),
    }
}

fn surface_index(surfaces: &[String], surface: &str) -> usize {
    surfaces
        .iter()
        .position(|key| key == surface)
        .unwrap_or(surfaces.len())
}
