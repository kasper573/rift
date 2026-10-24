use std::collections::BTreeSet;
use std::sync::Arc;
use ui::dialog;
use ui::egui;
use ui::form;
use ui::panel;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::assets::ImageRef;
use engine::core::content::files::NAME_RULE;
use engine::core::content::{ContentFile, valid_name};
use engine::systems::area::tileset::TilesetDef;
use image::RgbaImage;
use ui::controls;
use ui::egui::{Color32, Context, Rect, Sense, TextureHandle, Ui, Vec2};
use ui::glyph::{self, Mark};

use super::formats::{self, AnimationExport, ExportFormat};
use super::records::{DueReimport, ImportMapping, ImportRecord};
use super::tileset::{self, AnimPlan, TileBlock, TileGridCut, TileGridSettings, TilePlan};
use crate::app::Editor;
use crate::parts;
use crate::project::{ContentEdit, RowSelection};
use crate::shell::content_tree;
use crate::shell::widgets;
use crate::tabs::EditorTab;
use crate::tabs::{file_title, shown_path};
use crate::views::area;
use crate::views::area as area_view;
use crate::views::area::grid::BundlePalette;
use crate::views::tileset::{ANIMATION_PART, reset_view, tile_size};
use crate::{rows, thumbs};
use ui::input::InputField;

pub struct TilesetDialog {
    /// The tileset's path when importing onto one, or the name of the one to make.
    pub key: String,
    /// The folder a new tileset is made in.
    pub folder: String,
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

const NEW_TILE_PX: u32 = 16;

#[derive(Default)]
pub struct TextureCache {
    map: std::collections::HashMap<String, TextureHandle>,
}

impl TextureCache {
    pub fn of(&mut self, ctx: &Context, hash: &str, image: &RgbaImage) -> TextureHandle {
        self.map
            .entry(hash.to_owned())
            .or_insert_with(|| {
                let size = [image.width() as usize, image.height() as usize];
                ctx.load_texture(
                    format!("import-{hash}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                    egui::TextureOptions::NEAREST,
                )
            })
            .clone()
    }
}

fn image_of(editor: &Editor, tileset: &str) -> Option<RgbaImage> {
    super::asset_image(editor, &parts::tileset_image(tileset))
}

fn palettes_of(editor: &Editor) -> Result<Vec<BundlePalette>, String> {
    editor
        .project
        .as_ref()
        .map_or(Ok(Vec::new()), area::grid::palettes)
}

/// The tileset at `key` as its files read now.
fn tileset_of(editor: &Editor, key: &str) -> Option<Arc<TilesetDef>> {
    editor.read::<TilesetDef>(key)?.ok()
}

pub fn land_new(editor: &mut Editor, export: AnimationExport) -> Option<super::ImportDialog> {
    let name = super::suggested_name(&export, TilesetDef::TABLE);
    let mut dialog = new_dialog(name, false, export, NEW_TILE_PX);
    dialog.folder = content_tree::current_folder(editor);
    Some(super::ImportDialog::Tileset(Box::new(dialog)))
}

fn new_dialog(key: String, existing: bool, export: AnimationExport, tile: u32) -> TilesetDialog {
    let mut dialog = TilesetDialog {
        key,
        folder: String::new(),
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
    if super::refuses_unread::<TilesetDef>(editor, key) {
        return None;
    }
    let Some(row) = tileset_of(editor, key) else {
        return land_new(editor, export);
    };
    let Some(old) = image_of(editor, key) else {
        let tile = tile_size(&row).unwrap_or(NEW_TILE_PX);
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
    let settings = match editor
        .imports
        .record_for(TilesetDef::TABLE, key)
        .map(|record| &record.mapping)
    {
        Some(ImportMapping::Grid { grid }) => Ok(*grid),
        _ => tile_size(&row).map(|tile| TileGridSettings {
            tile,
            ..TileGridSettings::default()
        }),
    };
    let sheet = export.sheet.as_ref()?;
    let planned = settings.and_then(|settings| {
        let image = tileset::cut_grid(sheet, settings)?.image;
        let plan = tileset::tile_plan(&old, &row, &image, key, &palettes_of(editor)?)?;
        Ok((settings, image, plan))
    });
    let (settings, image, plan) = match planned {
        Ok(planned) => planned,
        Err(error) => {
            editor.toast(format!("Not imported onto {key}: {error}"));
            return None;
        }
    };
    let draft = TilesetReimport {
        export,
        settings,
        image,
        old,
        plan,
        textures: TextureCache::default(),
    };
    editor.open_tab(EditorTab::Tileset {
        key: key.to_owned(),
    });
    editor.select(RowSelection::parts(
        TilesetDef::TABLE,
        key,
        vec![super::IMPORT_PART.to_owned()],
    ));
    editor.views.tileset.reimports.insert(key.to_owned(), draft);
    None
}

pub fn land_animation(
    editor: &mut Editor,
    key: &str,
    mut export: AnimationExport,
    again: Option<&DueReimport>,
) -> Option<super::ImportDialog> {
    let row = tileset_of(editor, key)?;
    let Some(tileset_image) = image_of(editor, key) else {
        editor.toast(format!("{key} has no image yet; import its grid first"));
        return None;
    };
    let tile = tile_size(&row).and_then(|tile| {
        if export.kind == ExportFormat::Grid {
            export.cut_grid(tile, tile)?;
        }
        Ok(tile)
    });
    let tile = match tile {
        Ok(tile) => tile,
        Err(error) => {
            editor.toast(format!("Not imported: {error}"));
            return None;
        }
    };
    let block = again
        .and_then(|due| due.block)
        .map(|block| (block.column, block.row))
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

fn replan(dialog: &mut AnimationDialog, row: &TilesetDef) {
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
    dialog::card(ctx, "import-tileset", width, |ui| {
        ui.horizontal(|ui| {
            glyph::show(ui, Mark::File, 16.0, theme::MUTED);
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
                painter.image(texture.id(), shown, thumbs::FULL_UV, Color32::WHITE);
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
                    if dialog.existing {
                        ui.label(theme::mono(&dialog.key));
                        ui.label(theme::faint("this tileset, no image yet"));
                    } else {
                        if !dialog.folder.is_empty() {
                            ui.label(theme::mono(format!("{}/", dialog.folder)).color(theme::CODE));
                        }
                        InputField::new(&mut dialog.key).width(160.0).show(ui);
                        ui.label(theme::mono(".tileset").color(theme::CODE));
                    }
                });
                sheet::section(ui, "Grid");
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
                        changed |= InputField::new(text)
                            .unit("px")
                            .width(170.0)
                            .show(ui)
                            .changed();
                    });
                }
                if changed {
                    recut(&mut dialog);
                }
                sheet::section(ui, "Checks");
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
                                glyph::show(ui, Mark::Check, 14.0, theme::OK);
                                ui.label(theme::muted(text));
                            });
                        }
                    }
                    Err(error) => {
                        ui.horizontal_wrapped(|ui| {
                            glyph::show(ui, Mark::Warning, 14.0, theme::WARN);
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
            let problem =
                (!dialog.existing && !valid_name(&key)).then(|| format!("a name is {NAME_RULE}"));
            let enabled = dialog.cut.is_ok() && problem.is_none();
            if ui
                .add_enabled_ui(enabled, |ui| {
                    controls::primary(ui, format!("Create {key}.tileset"))
                })
                .inner
                .clicked()
            {
                create = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            let status = dialog.error.clone().or(problem).unwrap_or_else(|| {
                "writes the tileset with its image; the export stays where it is".to_owned()
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
    let row = tileset_of(editor, &dialog.key)?;
    match apply_animation(editor, &dialog, &row) {
        Ok(()) => None,
        Err(error) => {
            dialog.error = Some(error);
            Some(dialog)
        }
    }
}

fn create_tileset(editor: &mut Editor, dialog: &TilesetDialog) -> Result<(), String> {
    let cut = dialog.cut.as_ref().map_err(|error| error.clone())?;
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let (key, mut row) = if dialog.existing {
        let key = dialog.key.clone();
        let row = tileset_of(editor, &key).ok_or("the tileset is gone")?;
        (key, (*row).clone())
    } else {
        let name = dialog.key.trim();
        if !valid_name(name) {
            return Err(format!("a name is {NAME_RULE}"));
        }
        let key = rows::new_path(&project.files, &dialog.folder, TilesetDef::TABLE, name);
        let row = TilesetDef {
            image: ImageRef::new(parts::tileset_image(&key)),
            tile: cut.settings.tile,
            terrains: Default::default(),
            tiles: Default::default(),
        };
        (key, row)
    };
    row.tile = cut.settings.tile;
    let png = formats::encode_png(&cut.grid.image)?;
    let names = formats::export_name(&dialog.export.files);
    let mut edit = ContentEdit::new(format!("Imported {names} as {key}"));
    edit.write(parts::tileset_image(&key), Arc::from(png));
    edit.write_content(project, &key, &row)?;
    editor.try_apply(edit)?;
    let record = ImportRecord::new(
        &dialog.export.files,
        TilesetDef::TABLE,
        &key,
        ImportMapping::Grid { grid: cut.settings },
    );
    editor.imports.remember(record);
    editor.select(RowSelection::row(TilesetDef::TABLE, &key));
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
                sheet::section(ui, &title);
                let (rect, _) = ui.allocate_exact_size(panel, Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, theme::CANVAS);
                let (iw, ih) = image.dimensions();
                let scale = (rect.width() / iw.max(1) as f32).min(rect.height() / ih.max(1) as f32);
                let shown =
                    Rect::from_min_size(rect.min, Vec2::new(iw as f32 * scale, ih as f32 * scale));
                painter.image(texture.id(), shown, thumbs::FULL_UV, Color32::WHITE);
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
        sheet::section(
            ui,
            &format!(
                "{} palette · {} entr{} rewritten",
                file_title(&rewrite.area),
                rewrite.entries,
                if rewrite.entries == 1 { "y" } else { "ies" }
            ),
        );
        let text = match rewrite.first {
            Some((old, new)) => format!("- {key},{old},\n+ {key},{new},\n…"),
            None => "…".to_owned(),
        };
        panel::well().show(ui, |ui| {
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
        format!("{} · importing onto it", shown_path(key)),
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
    let names = formats::export_name(&draft.export.files);
    ui.label(
        theme::mono(format!("{} › import", shown_path(key)))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(6.0);
    panel::card(true).show(ui, |ui| {
        ui.vertical(|ui| {
            panel::title(ui, &format!("importing {names}"));
            for (mark, color, text, count) in [
                (
                    Mark::Check,
                    theme::OK,
                    "found by their pixels",
                    draft.plan.found,
                ),
                (
                    Mark::Reload,
                    theme::WARN,
                    "renumbered, longer rows",
                    draft.plan.renumbered,
                ),
                (
                    Mark::Plus,
                    theme::TEXT,
                    "new, no data yet",
                    draft.plan.new_tiles,
                ),
                (Mark::Cross, theme::MUTED, "gone", draft.plan.gone.len()),
            ]
            .into_iter()
            .chain((draft.plan.blank_data > 0).then_some((
                Mark::Dot,
                theme::FAINT,
                "transparent, their data carried nothing",
                draft.plan.blank_data,
            ))) {
                ui.horizontal(|ui| {
                    glyph::show(ui, mark, 13.0, color);
                    ui.label(theme::muted(text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::mono(count.to_string()));
                    });
                });
            }
        });
    });
    sheet::section(ui, "It rewrites");
    ui.horizontal(|ui| {
        widgets::badge(ui, "blueprint");
        ui.label(theme::mono(shown_path(key)));
        ui.label(theme::muted("tile data"));
    });
    for rewrite in &draft.plan.rewrites {
        ui.horizontal(|ui| {
            widgets::badge(ui, "area");
            ui.label(theme::mono(shown_path(&rewrite.area)));
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
        sheet::section(ui, "Gone but still painted");
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
                glyph::show(ui, Mark::Warning, 14.0, theme::WARN);
                ui.label(theme::muted(format!(
                    "tile {tile} is gone; painted in {where_}"
                )));
            });
            ui.horizontal(|ui| {
                ui.label(theme::muted("replace with"));
                let current = draft.plan.replacements.get(&tile).copied().flatten();
                let mut text = current.map(|t| t.to_string()).unwrap_or_default();
                let response = InputField::new(&mut text).hint("tile").width(70.0).show(ui);
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
            .add_enabled_ui(ready.is_ok(), |ui| {
                controls::primary(ui, "Import, as one step")
            })
            .inner
            .clicked()
        {
            apply = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if let Err(reason) = ready {
        form::hint(ui, &reason);
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
    let Some(row) = tileset_of(editor, key) else {
        return;
    };
    let applied = palettes_of(editor)
        .and_then(|palettes| tileset::apply_tile_plan(&draft.plan, &row, key, &palettes));
    let (new_row, rewritten) = match applied {
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
    let image = parts::tileset_image(key);
    let names = formats::export_name(&draft.export.files);
    let mut edit = ContentEdit::new(format!(
        "Imported {names} onto {key} · {} tiles renumbered",
        draft.plan.renumbered
    ));
    edit.write(image.to_owned(), Arc::from(png));
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let mut wrote = edit.write_content(project, key, &new_row).map(|_| ());
    for BundlePalette {
        bundle,
        rows: palette,
    } in rewritten
    {
        wrote = wrote.and_then(|()| {
            area_view::nodes::bundle_in(project, &bundle)
                .ok_or_else(|| format!("{} doesn't read", shown_path(&bundle)))?
                .write(&mut edit, project, &bundle, |blueprint| {
                    blueprint.scene.grids.palette = palette
                })
        });
    }
    if let Err(error) = wrote {
        editor.toast(format!("Not imported: {error}"));
        return;
    }
    let record = ImportRecord::new(
        &draft.export.files,
        TilesetDef::TABLE,
        key,
        ImportMapping::Grid {
            grid: draft.settings,
        },
    );
    if editor.apply(edit) {
        editor.imports.remember(record);
        editor.views.tileset.reimports.remove(key);
        reset_view(editor, key);
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
    let row = tileset_of(editor, &dialog.key)?;
    let strip = dialog.export.strips.first().cloned();
    let columns_total = (dialog.tileset.width() / dialog.tile.max(1)).max(1);
    dialog::card(ctx, "import-animation", width, |ui| {
        ui.horizontal(|ui| {
            glyph::show(ui, Mark::File, 16.0, theme::MUTED);
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
                                    thumbs::FULL_UV,
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
                sheet::section(
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
                    thumbs::FULL_UV,
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
                sheet::section(ui, "What the import does");
                match &dialog.plan {
                    Ok(plan) => {
                        let line = super::check_line;
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
                                    words::plural(columns.len()),
                                    words::join_and(&columns)
                                ),
                            );
                        } else {
                            line(
                                ui,
                                true,
                                format!(
                                    "{} new tile{} go after the last tile, so nothing renumbers{}.",
                                    plan.new_images.len(),
                                    words::plural(plan.new_images.len()),
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
                            glyph::show(ui, Mark::Warning, 14.0, theme::WARN);
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
                .add_enabled_ui(dialog.plan.is_ok(), |ui| {
                    controls::primary(ui, format!("Animate {count} tiles, as one step"))
                })
                .inner
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
    row: &TilesetDef,
) -> Result<(), String> {
    let plan = dialog.plan.as_ref().map_err(|error| error.clone())?;
    let (png, new_row) = tileset::apply_anim_plan(plan, &dialog.tileset, row, &dialog.export)?;
    let image = parts::tileset_image(&dialog.key);
    let names = formats::export_name(&dialog.export.files);
    let mut edit = ContentEdit::new(format!(
        "Animated {} tiles of {} from {names}",
        plan.animated(),
        dialog.key
    ));
    if let Some(png) = png {
        edit.write(image.to_owned(), Arc::from(png));
    }
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    edit.write_content(project, &dialog.key, &new_row)?;
    editor.try_apply(edit)?;
    let record = ImportRecord::new(
        &dialog.export.files,
        TilesetDef::TABLE,
        &dialog.key,
        ImportMapping::Animation {
            animation: TileBlock {
                column: dialog.block.0,
                row: dialog.block.1,
            },
        },
    );
    editor.imports.remember(record);
    reset_view(editor, &dialog.key);
    editor.select(RowSelection::parts(
        TilesetDef::TABLE,
        &dialog.key,
        vec![ANIMATION_PART.to_owned()],
    ));
    editor.open_tab(EditorTab::Tileset {
        key: dialog.key.clone(),
    });
    Ok(())
}
