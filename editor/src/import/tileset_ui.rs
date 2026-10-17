use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Color32, Context, Rect, Sense, TextureHandle, Ui, Vec2};
use engine::systems::area::grid::{self, PALETTE_FILE, PaletteRow};
use image::RgbaImage;
use serde_json::{Value, json};

use super::formats::{self, AnimationExport, ExportFormat};
use super::model_ui::TextureCache;
use super::records::{DueReimport, ImportRecord};
use super::tileset::{self, AnimPlan, TileGridCut, TileGridSettings, TilePlan};
use crate::app::Editor;
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::tabs::EditorTab;
use crate::ui::widgets;
use crate::ui::{dialogs, theme};
use crate::value::RowValue;
use crate::views::area;
use crate::views::tileset::{ANIMATION_PART, TABLE};
use crate::{forms, rows, schema};

pub struct TilesetDialog {
    pub key: String,
    pub existing: bool,
    pub export: AnimationExport,
    pub texture: Option<TextureHandle>,
    pub tile: String,
    pub margin: String,
    pub spacing: String,
    pub extruded: String,
    pub cut: Result<TilesetCut, String>,
    pub error: Option<String>,
}

pub struct TilesetCut {
    pub settings: TileGridSettings,
    pub grid: TileGridCut,
}

pub struct TilesetReimport {
    pub export: AnimationExport,
    pub settings: TileGridSettings,
    pub image: RgbaImage,
    pub old: RgbaImage,
    pub plan: TilePlan,
    pub textures: TextureCache,
    pub while_closed: bool,
}

pub struct AnimationDialog {
    pub key: String,
    pub export: AnimationExport,
    pub block: (u32, u32),
    pub tileset: RgbaImage,
    pub tile: u32,
    pub plan: Result<AnimPlan, String>,
    pub textures: TextureCache,
    pub error: Option<String>,
}

fn image_of(editor: &Editor, row: &Value) -> Option<RgbaImage> {
    let image = row["image"].as_str()?;
    let bytes = editor
        .project
        .as_ref()?
        .files
        .get(&format!("assets/{image}"))?;
    formats::decode_image_bytes(bytes).ok()
}

fn palettes_of(editor: &Editor) -> Vec<(String, Vec<PaletteRow>)> {
    editor
        .project
        .as_ref()
        .map(|project| area::grid::palettes(&project.files))
        .unwrap_or_default()
}

fn recorded_settings(record: Option<&ImportRecord>, tile: u32) -> TileGridSettings {
    let grid = record.map(|record| &record.mapping["grid"]);
    let read = |name: &str, fallback: u32| {
        grid.and_then(|grid| grid[name].as_u64())
            .map(|n| n as u32)
            .unwrap_or(fallback)
    };
    TileGridSettings {
        tile: read("tile", tile),
        margin: read("margin", 0),
        spacing: read("spacing", 0),
        extruded: read("extruded", 0),
    }
}

fn settings_value(settings: TileGridSettings) -> Value {
    json!({ "grid": {
        "tile": settings.tile,
        "margin": settings.margin,
        "spacing": settings.spacing,
        "extruded": settings.extruded,
    }})
}

pub fn land_new(editor: &mut Editor, export: AnimationExport) -> Option<super::ImportDialog> {
    let wanted = export
        .files
        .first()
        .map(|file| formats::stem(file).replace(['-', ' '], "_").to_lowercase())
        .unwrap_or_else(|| "new_tileset".to_owned());
    let key = editor
        .project
        .as_ref()
        .map(|project| rows::free_key(&project.files, TABLE, &wanted))
        .unwrap_or(wanted);
    Some(super::ImportDialog::Tileset(Box::new(new_dialog(
        key, false, export, 16,
    ))))
}

fn new_dialog(key: String, existing: bool, export: AnimationExport, tile: u32) -> TilesetDialog {
    let mut dialog = TilesetDialog {
        key,
        existing,
        export,
        texture: None,
        tile: tile.to_string(),
        margin: "0".to_owned(),
        spacing: "0".to_owned(),
        extruded: "0".to_owned(),
        cut: Err(String::new()),
        error: None,
    };
    recut(&mut dialog);
    dialog
}

fn recut(dialog: &mut TilesetDialog) {
    let parse = |text: &str, what: &str| -> Result<u32, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("{what} is a whole number of pixels"))
    };
    let settings = (|| {
        Ok::<_, String>(TileGridSettings {
            tile: parse(&dialog.tile, "tile")?,
            margin: parse(&dialog.margin, "margin")?,
            spacing: parse(&dialog.spacing, "spacing")?,
            extruded: parse(&dialog.extruded, "extruded")?,
        })
    })();
    let Some(sheet) = dialog.export.sheet.as_ref() else {
        dialog.cut = Err("not a grid".to_owned());
        return;
    };
    dialog.cut = settings.and_then(|settings| {
        tileset::cut_grid(sheet, settings).map(|grid| TilesetCut { settings, grid })
    });
}

pub fn land(
    editor: &mut Editor,
    key: &str,
    export: AnimationExport,
    again: Option<&DueReimport>,
) -> Option<super::ImportDialog> {
    let Some(row) = editor.row_value(TABLE, key) else {
        return land_new(editor, export);
    };
    let tile = row["tile"].as_u64().unwrap_or(16) as u32;
    let Some(old) = image_of(editor, &row) else {
        return Some(super::ImportDialog::Tileset(Box::new(new_dialog(
            key.to_owned(),
            true,
            export,
            tile,
        ))));
    };
    if export.kind != ExportFormat::Grid {
        return land_animation(editor, key, export, again);
    }
    let record = editor.imports.record_for(TABLE, key).cloned();
    let settings = recorded_settings(record.as_ref(), tile);
    let sheet = export.sheet.as_ref()?;
    let cut = match tileset::cut_grid(sheet, settings) {
        Ok(cut) => cut,
        Err(error) => {
            editor.toast(format!("Not imported onto {key}: {error}"));
            return None;
        }
    };
    let palettes = palettes_of(editor);
    let plan = tileset::tile_plan(&old, &row, &cut.image, key, &palettes);
    let draft = TilesetReimport {
        export,
        settings,
        image: cut.image,
        old,
        plan,
        textures: TextureCache::default(),
        while_closed: again.is_some_and(|due| due.while_closed),
    };
    editor.open_tab(EditorTab::Tileset {
        key: key.to_owned(),
    });
    editor.select(RowSelection::parts(
        TABLE,
        key,
        vec![super::IMPORT_PART.to_owned()],
    ));
    editor.views.tileset.reimports.insert(key.to_owned(), draft);
    None
}

pub fn land_animation(
    editor: &mut Editor,
    key: &str,
    export: AnimationExport,
    again: Option<&DueReimport>,
) -> Option<super::ImportDialog> {
    let row = editor.row_value(TABLE, key)?;
    let Some(tileset_image) = image_of(editor, &row) else {
        editor.toast(format!(
            "tileset/{key} has no image yet; import its grid first"
        ));
        return None;
    };
    let mut export = export;
    if export.kind == ExportFormat::Grid {
        let tile = row["tile"].as_u64().unwrap_or(16) as u32;
        if let Err(error) = export.cut_grid(tile, tile) {
            editor.toast(format!("Not imported: {error}"));
            return None;
        }
    }
    let record = again.and_then(|due| editor.imports.record_with(TABLE, key, &due.part));
    let block = record
        .and_then(|record| {
            let block = &record.mapping["animation"];
            Some((
                block["column"].as_u64()? as u32,
                block["row"].as_u64()? as u32,
            ))
        })
        .or_else(|| {
            editor
                .views
                .tileset
                .views
                .get(key)
                .and_then(|view| view.selection.as_ref())
                .map(|rect| (rect.x, rect.y))
        })
        .unwrap_or((0, 0));
    let tile = row["tile"].as_u64().unwrap_or(16).max(1) as u32;
    let mut dialog = AnimationDialog {
        key: key.to_owned(),
        export,
        block,
        tileset: tileset_image,
        tile,
        plan: Err(String::new()),
        textures: TextureCache::default(),
        error: None,
    };
    replan(&mut dialog, &row);
    Some(super::ImportDialog::Animation(Box::new(dialog)))
}

fn replan(dialog: &mut AnimationDialog, row: &Value) {
    let gif = dialog.export.kind == ExportFormat::Gif;
    dialog.plan = tileset::anim_plan(&dialog.tileset, row, &dialog.export, gif, dialog.block);
}

pub fn dialog(
    editor: &mut Editor,
    ctx: &Context,
    mut dialog: TilesetDialog,
) -> Option<TilesetDialog> {
    let mut close = false;
    let mut create = false;
    let width = 1150.0;
    let file = dialog.export.files.first().cloned().unwrap_or_default();
    let (iw, ih) = dialog
        .export
        .sheet
        .as_ref()
        .map(|sheet| sheet.dimensions())
        .unwrap_or((0, 0));
    dialogs::card(ctx, "import-tileset", width, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🗋").size(16.0).color(theme::MUTED));
            ui.label(theme::strong("Import a tileset").size(17.0));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::code(super::show_path(editor, &file)));
            ui.label(theme::muted(format!("· a tileset on a grid · {iw} × {ih}")));
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(575.0, 415.0), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 3.0, theme::CANVAS);
            if dialog.texture.is_none()
                && let Some(sheet) = dialog.export.sheet.as_ref()
            {
                let size = [sheet.width() as usize, sheet.height() as usize];
                dialog.texture = Some(ctx.load_texture(
                    format!("import-tileset-{}", formats::name(&file)),
                    egui::ColorImage::from_rgba_unmultiplied(size, sheet.as_raw()),
                    egui::TextureOptions::NEAREST,
                ));
            }
            if let Some(texture) = &dialog.texture
                && iw > 0
            {
                let scale = (rect.width() / iw as f32).min(rect.height() / ih as f32);
                let shown =
                    Rect::from_min_size(rect.min, Vec2::new(iw as f32 * scale, ih as f32 * scale));
                painter.image(
                    texture.id(),
                    shown,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if let Ok(cut) = &dialog.cut {
                    let settings = cut.settings;
                    let pitch =
                        (settings.tile + 2 * settings.extruded + settings.spacing) as f32 * scale;
                    let stroke = egui::Stroke::new(1.0, Color32::from_white_alpha(28));
                    for column in 0..=cut.grid.columns {
                        let x =
                            shown.min.x + settings.margin as f32 * scale + column as f32 * pitch;
                        painter.line_segment(
                            [egui::pos2(x, shown.min.y), egui::pos2(x, shown.max.y)],
                            stroke,
                        );
                    }
                    for row in 0..=cut.grid.rows {
                        let y = shown.min.y + settings.margin as f32 * scale + row as f32 * pitch;
                        painter.line_segment(
                            [egui::pos2(shown.min.x, y), egui::pos2(shown.max.x, y)],
                            stroke,
                        );
                    }
                }
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(width - 575.0 - 40.0);
                ui.horizontal(|ui| {
                    ui.label(theme::muted("into"));
                    ui.label(theme::mono("tileset/").color(theme::CODE));
                    if dialog.existing {
                        ui.label(theme::mono(&dialog.key));
                        ui.label(theme::faint("this row, no image yet"));
                    } else {
                        ui.add(
                            egui::TextEdit::singleline(&mut dialog.key)
                                .desired_width(160.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.label(theme::faint("a new row"));
                    }
                });
                widgets::section(ui, "Grid");
                let mut changed = false;
                for (label, text) in [
                    ("tile", &mut dialog.tile),
                    ("margin", &mut dialog.margin),
                    ("spacing", &mut dialog.spacing),
                    ("extruded", &mut dialog.extruded),
                ] {
                    ui.horizontal(|ui| {
                        ui.allocate_ui(Vec2::new(80.0, 20.0), |ui| {
                            ui.label(theme::muted(label));
                        });
                        ui.scope(|ui| {
                            ui.set_max_width(170.0);
                            let response = forms::boxed_input(ui, Some("px"), |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(text)
                                        .desired_width(110.0)
                                        .font(egui::TextStyle::Monospace),
                                )
                            });
                            changed |= response.changed();
                        });
                    });
                }
                if changed {
                    recut(&mut dialog);
                }
                widgets::section(ui, "Checks");
                match &dialog.cut {
                    Ok(cut) => {
                        for text in [
                            format!(
                                "Divides into {} columns and {} rows: {} tiles.",
                                cut.grid.columns,
                                cut.grid.rows,
                                cut.grid.columns * cut.grid.rows
                            ),
                            format!(
                                "{} drawn, {} empty, kept in place.",
                                cut.grid.drawn, cut.grid.empty
                            ),
                            "Converted to sRGB with straight alpha.".to_owned(),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(theme::ok("✓"));
                                ui.label(theme::muted(text));
                            });
                        }
                    }
                    Err(error) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::warn("⚠"));
                            ui.label(theme::muted(error));
                        });
                    }
                }
                ui.add_space(8.0);
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let key = dialog.key.trim().to_owned();
            let problem = if dialog.existing {
                None
            } else {
                editor
                    .project
                    .as_ref()
                    .and_then(|project| rows::key_problem(&project.files, TABLE, &key))
            };
            let enabled = dialog.cut.is_ok() && problem.is_none();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(
                        egui::RichText::new(format!("Create tileset/{key}")).color(Color32::WHITE),
                    )
                    .fill(theme::ACCENT),
                )
                .clicked()
            {
                create = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            let status = dialog.error.clone().or(problem).unwrap_or_else(|| {
                format!(
                    "writes assets/{} and the row; the export stays where it is",
                    image_path(&key)
                )
            });
            ui.label(theme::muted(status));
        });
    });
    if create {
        match create_tileset(editor, &dialog) {
            Ok(()) => return None,
            Err(error) => dialog.error = Some(error),
        }
    }
    if close {
        return None;
    }
    Some(dialog)
}

fn image_path(key: &str) -> String {
    format!("tilesets/{}.png", key.replace('_', "-"))
}

pub fn apply_now(editor: &mut Editor, mut dialog: TilesetDialog) -> Option<TilesetDialog> {
    match create_tileset(editor, &dialog) {
        Ok(()) => None,
        Err(error) => {
            dialog.error = Some(error);
            Some(dialog)
        }
    }
}

pub fn apply_animation_now(
    editor: &mut Editor,
    mut dialog: AnimationDialog,
) -> Option<AnimationDialog> {
    let row = editor.row_value(TABLE, &dialog.key)?;
    match apply_animation(editor, &dialog, &row) {
        Ok(()) => None,
        Err(error) => {
            dialog.error = Some(error);
            Some(dialog)
        }
    }
}

fn create_tileset(editor: &mut Editor, dialog: &TilesetDialog) -> Result<(), String> {
    let key = dialog.key.trim().to_owned();
    let cut = dialog.cut.as_ref().map_err(|error| error.clone())?;
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut row = if dialog.existing {
        editor.row_value(TABLE, &key).ok_or("the row is gone")?
    } else {
        if let Some(problem) = rows::key_problem(&project.files, TABLE, &key) {
            return Err(problem);
        }
        let shape = editor
            .shapes
            .table(TABLE)
            .ok_or("the tileset table has no shape")?;
        schema::template(&shape.shape, &shape.shape)
    };
    let image = image_path(&key);
    row["image"] = Value::String(image.clone());
    row["tile"] = json!(cut.settings.tile);
    let png = formats::encode_png(&cut.grid.image)?;
    let names = ImportRecord::of_files(&dialog.export.files, TABLE, &key).names();
    let mut edit = ContentEdit::new(format!("Imported {names} as tileset/{key}"));
    edit.write(format!("assets/{image}"), Arc::from(png));
    edit.write_json(row_file(TABLE, &key), &row);
    editor.try_apply(edit)?;
    let mut record = ImportRecord::of_files(&dialog.export.files, TABLE, &key);
    record.mapping = settings_value(cut.settings);
    editor.imports.remember(record);
    editor.select(RowSelection::row(TABLE, &key));
    editor.open_tab(EditorTab::Tileset { key });
    Ok(())
}

pub fn show_reimport(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let ctx = ui.ctx().clone();
    let Some(draft) = editor.views.tileset.reimports.get_mut(key) else {
        return;
    };
    let plan = draft.plan.clone();
    let example = plan.example;
    let old_hash = formats::pixel_hash(&draft.old);
    let new_hash = formats::pixel_hash(&draft.image);
    let old_texture = draft.textures.of(&ctx, &old_hash, &draft.old);
    let new_texture = draft.textures.of(&ctx, &new_hash, &draft.image);
    let panel = Vec2::new(346.0, 250.0);
    ui.horizontal_top(|ui| {
        for (title, texture, image, columns, tile_index, caption) in [
            (
                format!("Before · {} columns", plan.old_columns),
                &old_texture,
                &draft.old,
                plan.old_columns,
                example.map(|(old, _)| old),
                example.map(|(old, _)| {
                    format!(
                        "tile {old} · column {}, row {}",
                        old % plan.old_columns.max(1),
                        old / plan.old_columns.max(1)
                    )
                }),
            ),
            (
                format!("After · {} columns", plan.new_columns),
                &new_texture,
                &draft.image,
                plan.new_columns,
                example.map(|(_, new)| new),
                example.map(|(old, new)| {
                    let same = old % plan.old_columns.max(1) == new % plan.new_columns.max(1)
                        && old / plan.old_columns.max(1) == new / plan.new_columns.max(1);
                    if same {
                        format!("tile {new} · same column and row")
                    } else {
                        format!(
                            "tile {new} · column {}, row {}",
                            new % plan.new_columns.max(1),
                            new / plan.new_columns.max(1)
                        )
                    }
                }),
            ),
        ] {
            ui.vertical(|ui| {
                ui.set_width(panel.x);
                widgets::section(ui, &title);
                let (rect, _) = ui.allocate_exact_size(panel, Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, theme::CANVAS);
                let (iw, ih) = image.dimensions();
                let scale = (rect.width() / iw.max(1) as f32).min(rect.height() / ih.max(1) as f32);
                let shown =
                    Rect::from_min_size(rect.min, Vec2::new(iw as f32 * scale, ih as f32 * scale));
                painter.image(
                    texture.id(),
                    shown,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if plan.new_columns > plan.old_columns && columns == plan.new_columns {
                    let x = shown.min.x + (plan.old_columns * plan.tile) as f32 * scale;
                    let dashes = egui::Shape::dashed_line(
                        &[egui::pos2(x, shown.min.y), egui::pos2(x, shown.max.y)],
                        egui::Stroke::new(1.0, theme::SELECTION),
                        4.0,
                        3.0,
                    );
                    painter.extend(dashes);
                }
                if let Some(index) = tile_index {
                    let tile = plan.tile as f32 * scale;
                    let cell = Rect::from_min_size(
                        egui::pos2(
                            shown.min.x + (index % columns.max(1)) as f32 * tile,
                            shown.min.y + (index / columns.max(1)) as f32 * tile,
                        ),
                        Vec2::splat(tile),
                    );
                    painter.rect_stroke(
                        cell,
                        0.0,
                        egui::Stroke::new(2.0, theme::WARN),
                        egui::StrokeKind::Outside,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(cell.center().x, cell.max.y),
                            egui::pos2(cell.center().x, rect.max.y + 10.0),
                        ],
                        egui::Stroke::new(1.0, theme::WARN),
                    );
                }
                ui.add_space(14.0);
                if let Some(caption) = caption {
                    ui.label(
                        theme::mono(caption)
                            .color(theme::TEXT)
                            .background_color(theme::INPUT),
                    );
                }
            });
            ui.add_space(24.0);
        }
    });
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.set_max_width(panel.x * 2.0 + 24.0);
        ui.label(theme::muted(format!(
            "{} new tiles, no data yet (K6).",
            plan.new_tiles
        )));
    });
    ui.add_space(20.0);
    for rewrite in &plan.rewrites {
        widgets::section(
            ui,
            &format!(
                "{} · {} entr{} rewritten",
                grid::palette_file(&rewrite.area),
                rewrite.entries,
                if rewrite.entries == 1 { "y" } else { "ies" }
            ),
        );
        let text = match rewrite.first {
            Some((old, new)) => format!("- {key},{old},\n+ {key},{new},\n…"),
            None => "…".to_owned(),
        };
        egui::Frame::new()
            .fill(theme::INPUT)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(4.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.set_width(panel.x - 20.0);
                for line in text.lines() {
                    let color = match line.chars().next() {
                        Some('-') => theme::DANGER,
                        Some('+') => theme::OK,
                        _ => theme::MUTED,
                    };
                    ui.label(theme::mono(line).color(color));
                }
            });
        ui.add_space(8.0);
    }
    editor.status.left = vec![
        format!("tileset/{key} · importing onto it"),
        format!(
            "{} walk · {} surfaces · {} animations carried over",
            plan.walk, plan.surfaces, plan.animations
        ),
    ];
}

pub fn inspect_reimport(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let mut apply = false;
    let mut cancel = false;
    let Some(draft) = editor.views.tileset.reimports.get_mut(key) else {
        return;
    };
    let names = ImportRecord::of_files(&draft.export.files, TABLE, key).names();
    ui.label(
        theme::mono(format!("tileset/{key} › import"))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(6.0);
    widgets::card_frame(true).show(ui, |ui| {
        ui.vertical(|ui| {
            widgets::card_title(ui, &format!("importing {names}"));
            for (mark, color, text, count) in [
                ("✓", theme::OK, "found by their pixels", draft.plan.found),
                (
                    "↻",
                    theme::WARN,
                    "renumbered, longer rows",
                    draft.plan.renumbered,
                ),
                ("+", theme::TEXT, "new, no data yet", draft.plan.new_tiles),
                ("✕", theme::MUTED, "gone", draft.plan.gone.len()),
            ]
            .into_iter()
            .chain((draft.plan.blank_data > 0).then_some((
                "·",
                theme::FAINT,
                "transparent, their data carried nothing",
                draft.plan.blank_data,
            ))) {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(mark).color(color));
                    ui.label(theme::muted(text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::mono(count.to_string()));
                    });
                });
            }
        });
    });
    widgets::section(ui, "It rewrites");
    ui.horizontal(|ui| {
        widgets::badge(ui, "blueprint");
        ui.label(theme::mono(format!("tileset/{key}")));
        ui.label(theme::muted("tile data"));
    });
    for rewrite in &draft.plan.rewrites {
        ui.horizontal(|ui| {
            widgets::badge(ui, "area");
            ui.label(theme::mono(format!("area/{}", rewrite.area)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::muted(format!(
                    "{} palette entr{}",
                    rewrite.entries,
                    if rewrite.entries == 1 { "y" } else { "ies" }
                )));
            });
        });
    }
    if !draft.plan.gone_used.is_empty() {
        widgets::section(ui, "Gone but still painted");
        let gone: Vec<(u32, Vec<tileset::PaletteUse>)> = draft
            .plan
            .gone_used
            .iter()
            .map(|(tile, uses)| (*tile, uses.clone()))
            .collect();
        let new_count = draft.plan.new_count();
        for (tile, uses) in gone {
            let where_ = uses
                .iter()
                .map(|u| format!("{} ({})", u.area, u.entries))
                .collect::<Vec<_>>()
                .join(", ");
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::warn("⚠"));
                ui.label(theme::muted(format!(
                    "tile {tile} is gone; painted in {where_}"
                )));
            });
            ui.horizontal(|ui| {
                ui.label(theme::muted("replace with"));
                let current = draft.plan.replacements.get(&tile).copied().flatten();
                let mut text = current.map(|t| t.to_string()).unwrap_or_default();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(70.0)
                        .hint_text("tile"),
                );
                if response.changed() {
                    let picked = text.trim().parse::<u32>().ok().filter(|t| *t < new_count);
                    draft.plan.replacements.insert(tile, picked);
                }
            });
        }
    }
    ui.add_space(8.0);
    let ready = draft.plan.ready();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                ready.is_ok(),
                egui::Button::new(egui::RichText::new("Import, as one step").color(Color32::WHITE))
                    .fill(theme::ACCENT),
            )
            .clicked()
        {
            apply = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if let Err(reason) = ready {
        widgets::hint(ui, &reason);
    }
    if apply {
        apply_reimport(editor, key);
    }
    if cancel {
        editor.views.tileset.reimports.remove(key);
    }
}

pub fn apply_reimport(editor: &mut Editor, key: &str) {
    let Some(draft) = editor.views.tileset.reimports.get(key) else {
        return;
    };
    let Some(row) = editor.row_value(TABLE, key) else {
        return;
    };
    let palettes = palettes_of(editor);
    let (new_row, rewritten) = match tileset::apply_tile_plan(&draft.plan, &row, key, &palettes) {
        Ok(result) => result,
        Err(error) => {
            editor.toast(format!("Not imported: {error}"));
            return;
        }
    };
    let png = match formats::encode_png(&draft.image) {
        Ok(png) => png,
        Err(error) => {
            editor.toast(format!("Not imported: {error}"));
            return;
        }
    };
    let image = row.text("image").to_owned();
    let names = ImportRecord::of_files(&draft.export.files, TABLE, key).names();
    let mut edit = ContentEdit::new(format!(
        "Imported {names} into {key} again · {} tiles renumbered",
        draft.plan.renumbered
    ));
    edit.write(format!("assets/{image}"), Arc::from(png));
    edit.write_json(row_file(TABLE, key), &new_row);
    for (area, text) in rewritten {
        edit.write(
            format!("{}/{}", grid::folder(&area), PALETTE_FILE),
            Arc::from(text.into_bytes()),
        );
    }
    let mut record = ImportRecord::of_files(&draft.export.files, TABLE, key);
    record.mapping = settings_value(draft.settings);
    if editor.apply(edit) {
        editor.imports.remember(record);
        editor.views.tileset.reimports.remove(key);
        editor.views.tileset.views.remove(key);
    }
}

pub fn animation_dialog(
    editor: &mut Editor,
    ctx: &Context,
    mut dialog: AnimationDialog,
) -> Option<AnimationDialog> {
    let mut close = false;
    let mut animate = false;
    let width = 1100.0;
    let file = dialog.export.files.first().cloned().unwrap_or_default();
    let row = editor.row_value(TABLE, &dialog.key)?;
    let strip = dialog.export.strips.first().cloned();
    let columns_total = (dialog.tileset.width() / dialog.tile.max(1)).max(1);
    dialogs::card(ctx, "import-animation", width, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🗋").size(16.0).color(theme::MUTED));
            ui.label(
                theme::strong(format!("Import an animation onto tileset/{}", dialog.key))
                    .size(17.0),
            );
        });
        ui.add_space(4.0);
        let lengths: Vec<u32> = strip
            .as_ref()
            .map(|strip| {
                strip
                    .frames
                    .iter()
                    .map(|f| f.ms.unwrap_or(formats::DEFAULT_MS))
                    .collect()
            })
            .unwrap_or_default();
        let each = lengths
            .first()
            .copied()
            .filter(|ms| lengths.iter().all(|other| other == ms));
        ui.horizontal(|ui| {
            ui.label(theme::code(super::show_path(editor, &file)));
            ui.label(theme::muted(format!(
                "· {} · {} frames of {} × {} · {}",
                dialog.export.kind.sentence(),
                lengths.len(),
                dialog.export.frame.0,
                dialog.export.frame.1,
                match each {
                    Some(ms) => format!("{ms} ms each"),
                    None => format!("{} ms in all", lengths.iter().sum::<u32>()),
                }
            )));
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(600.0);
                if let Some(strip) = &strip {
                    ui.horizontal(|ui| {
                        for (index, frame) in strip.frames.iter().enumerate().take(4) {
                            ui.vertical(|ui| {
                                let texture = dialog.textures.of(ctx, &frame.hash, &frame.image);
                                let (rect, _) = ui.allocate_exact_size(Vec2::splat(140.0), Sense::hover());
                                ui.painter().rect_filled(rect, 3.0, theme::CANVAS);
                                let (fw, fh) = dialog.export.frame;
                                let scale = (rect.width() / fw.max(1) as f32).min(rect.height() / fh.max(1) as f32);
                                let shown = Rect::from_center_size(rect.center(), Vec2::new(fw as f32 * scale, fh as f32 * scale));
                                ui.painter().image(
                                    texture.id(),
                                    shown,
                                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                                ui.label(theme::mono(format!("frame {} · {} ms", index + 1, frame.ms.unwrap_or(formats::DEFAULT_MS))).size(11.0).color(theme::MUTED));
                            });
                        }
                        if strip.frames.len() > 4 {
                            ui.label(theme::faint(format!("… {} more", strip.frames.len() - 4)));
                        }
                    });
                }
                widgets::section(
                    ui,
                    &format!(
                        "Onto the block at column {}, row {} · picked on the tileset",
                        dialog.block.0,
                        dialog.block.1
                    ),
                );
                let (rect, response) = ui.allocate_exact_size(Vec2::new(432.0, 310.0), Sense::click());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, theme::CANVAS);
                let tileset_hash = formats::pixel_hash(&dialog.tileset);
                let texture = dialog.textures.of(ctx, &tileset_hash, &dialog.tileset);
                let (iw, ih) = dialog.tileset.dimensions();
                let scale = (rect.width() / iw.max(1) as f32).min(rect.height() / ih.max(1) as f32);
                let shown = Rect::from_min_size(rect.min, Vec2::new(iw as f32 * scale, ih as f32 * scale));
                painter.image(
                    texture.id(),
                    shown,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let tile = dialog.tile as f32 * scale;
                let stroke = egui::Stroke::new(1.0, Color32::from_white_alpha(20));
                let columns = iw / dialog.tile.max(1);
                let rows = ih / dialog.tile.max(1);
                for column in 0..=columns {
                    let x = shown.min.x + column as f32 * tile;
                    painter.line_segment([egui::pos2(x, shown.min.y), egui::pos2(x, shown.max.y)], stroke);
                }
                for row_index in 0..=rows {
                    let y = shown.min.y + row_index as f32 * tile;
                    painter.line_segment([egui::pos2(shown.min.x, y), egui::pos2(shown.max.x, y)], stroke);
                }
                let (bw, bh) = (
                    (dialog.export.frame.0 / dialog.tile.max(1)).max(1),
                    (dialog.export.frame.1 / dialog.tile.max(1)).max(1),
                );
                let block = Rect::from_min_size(
                    egui::pos2(
                        shown.min.x + dialog.block.0 as f32 * tile,
                        shown.min.y + dialog.block.1 as f32 * tile,
                    ),
                    Vec2::new(bw as f32 * tile, bh as f32 * tile),
                );
                painter.rect_stroke(block, 0.0, egui::Stroke::new(2.0, theme::WARN), egui::StrokeKind::Outside);
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && shown.contains(pos)
                {
                    let column = ((pos.x - shown.min.x) / tile) as u32;
                    let row_index = ((pos.y - shown.min.y) / tile) as u32;
                    dialog.block = (
                        column.min(columns.saturating_sub(bw)),
                        row_index.min(rows.saturating_sub(bh)),
                    );
                    replan(&mut dialog, &row);
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(width - 600.0 - 60.0);
                widgets::section(ui, "What the import does");
                match &dialog.plan {
                    Ok(plan) => {
                        let line = |ui: &mut Ui, ok: bool, text: String| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(if ok { theme::ok("✓") } else { theme::warn("⚠") });
                                ui.label(theme::muted(text).size(12.5));
                            });
                        };
                        line(
                            ui,
                            true,
                            format!(
                                "{} tiles change between frames: they animate, {} frames of {}.",
                                plan.animated(),
                                plan.lengths.len(),
                                match each {
                                    Some(ms) => format!("{ms} ms each"),
                                    None => format!("{} ms in all", plan.lengths.iter().sum::<u32>()),
                                }
                            ),
                        );
                        line(ui, true, format!("{} tiles never change: they stay still.", plan.still));
                        let frames_text = match plan.lengths.len() {
                            2 => "Frame 2 is".to_owned(),
                            n => format!("Frames 2 to {n} are"),
                        };
                        if plan.new_images.is_empty() {
                            let columns: Vec<String> = plan
                                .cells
                                .iter()
                                .flat_map(|cell| {
                                    cell.frames.iter().filter_map(move |frame| match frame {
                                        tileset::AnimFrameRef::Existing(index) => Some(
                                            (index % columns_total).saturating_sub(cell.column - plan.block.0),
                                        ),
                                        _ => None,
                                    })
                                })
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .map(|column| column.to_string())
                                .collect();
                            line(
                                ui,
                                true,
                                format!(
                                    "{frames_text} already in the tileset, found by their pixels at column{} {}. No tiles added.",
                                    widgets::plural(columns.len()),
                                    widgets::join_and(&columns)
                                ),
                            );
                        } else {
                            line(
                                ui,
                                true,
                                format!(
                                    "{} new tile{} go after the last tile, so nothing renumbers{}.",
                                    plan.new_images.len(),
                                    widgets::plural(plan.new_images.len()),
                                    match plan.reused.len() {
                                        0 => String::new(),
                                        n => format!("; {n} found by their pixels already"),
                                    }
                                ),
                            );
                        }
                        let redrawn = plan.cells.iter().filter(|cell| cell.redrawn).count();
                        if redrawn > 0 {
                            line(ui, false, format!("{redrawn} tiles of the block take the first frame's pixels."));
                        }
                        if let Some(fits) = &plan.fits {
                            line(ui, true, fits.clone());
                        }
                        for warning in &plan.warnings {
                            line(ui, false, warning.clone());
                        }
                        ui.add_space(6.0);
                    }
                    Err(error) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::warn("⚠"));
                            ui.label(theme::muted(error));
                        });
                    }
                }
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let count = dialog.plan.as_ref().map(AnimPlan::animated).unwrap_or(0);
            if ui
                .add_enabled(
                    dialog.plan.is_ok(),
                    egui::Button::new(
                        egui::RichText::new(format!("Animate {count} tiles, as one step"))
                            .color(Color32::WHITE),
                    )
                    .fill(theme::ACCENT),
                )
                .clicked()
            {
                animate = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            if let Some(error) = &dialog.error {
                ui.label(theme::muted(error));
            }
        });
    });
    if animate {
        match apply_animation(editor, &dialog, &row) {
            Ok(()) => return None,
            Err(error) => dialog.error = Some(error),
        }
    }
    if close {
        return None;
    }
    Some(dialog)
}

fn apply_animation(
    editor: &mut Editor,
    dialog: &AnimationDialog,
    row: &Value,
) -> Result<(), String> {
    let plan = dialog.plan.as_ref().map_err(|error| error.clone())?;
    let (png, new_row) = tileset::apply_anim_plan(plan, &dialog.tileset, row, &dialog.export)?;
    let image = row.text("image").to_owned();
    let names = ImportRecord::of_files(&dialog.export.files, TABLE, &dialog.key).names();
    let mut edit = ContentEdit::new(format!(
        "Animated {} tiles of {} from {names}",
        plan.animated(),
        dialog.key
    ));
    if let Some(png) = png {
        edit.write(format!("assets/{image}"), Arc::from(png));
    }
    edit.write_json(row_file(TABLE, &dialog.key), &new_row);
    editor.try_apply(edit)?;
    let mut record = ImportRecord::of_files(&dialog.export.files, TABLE, &dialog.key);
    record.mapping = json!({ "animation": { "column": dialog.block.0, "row": dialog.block.1 } });
    record.part = format!("animation {},{}", dialog.block.0, dialog.block.1);
    editor.imports.remember(record);
    editor.views.tileset.views.remove(&dialog.key);
    editor.select(RowSelection::parts(
        TABLE,
        &dialog.key,
        vec![ANIMATION_PART.to_owned()],
    ));
    editor.open_tab(EditorTab::Tileset {
        key: dialog.key.clone(),
    });
    Ok(())
}
