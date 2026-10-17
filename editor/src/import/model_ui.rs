use std::collections::{BTreeMap, HashMap};

use egui::{Color32, Context, Rect, Sense, TextureHandle, Ui, Vec2};
use image::RgbaImage;
use serde_json::Value;

use super::formats::{self, AnimationExport, ExportFormat};
use super::model::{
    self, CuePlacement, FrameCue, MappingNote, ModelImportPlan, StripMapping, StripPlan,
};
use super::records::{DueReimport, ImportRecord};
use crate::app::Editor;
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::tabs::EditorTab;
use crate::ui::widgets;
use crate::ui::{dialogs, theme};
use crate::views::model::{StripFrame, TABLE};
use crate::{rows, schema};

const FRAME_CARD: f32 = 48.0;

#[derive(Default)]
pub struct TextureCache {
    map: HashMap<String, TextureHandle>,
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

pub struct ModelDialog {
    pub key: String,
    pub new_row: bool,
    pub export: AnimationExport,
    pub mapping: StripMapping,
    pub notes: BTreeMap<String, MappingNote>,
    pub asked: BTreeMap<String, String>,
    pub grid: (String, String),
    pub plan: ModelImportPlan,
    pub selected: Option<String>,
    pub textures: TextureCache,
    pub record: Option<ImportRecord>,
    pub sheet: Option<RgbaImage>,
    pub error: Option<String>,
}

pub struct ModelReimport {
    pub export: AnimationExport,
    pub mapping: StripMapping,
    pub plan: ModelImportPlan,
    pub auto: bool,
    pub while_closed: bool,
    pub textures: TextureCache,
    pub sheet: Option<RgbaImage>,
    pub old_frame: (u32, u32),
}

fn frame_size_of(row: &Value) -> (u32, u32) {
    (
        row["sprites"]["frame_size"]["width"].as_u64().unwrap_or(0) as u32,
        row["sprites"]["frame_size"]["height"].as_u64().unwrap_or(0) as u32,
    )
}

fn sheet_of(editor: &Editor, row: &Value) -> Option<RgbaImage> {
    let image = row["sprites"]["image"].as_str()?;
    let bytes = editor
        .project
        .as_ref()?
        .files
        .get(&format!("assets/{image}"))?;
    formats::decode_image_bytes(bytes).ok()
}

fn asked_ms(asked: &BTreeMap<String, String>) -> BTreeMap<String, u32> {
    asked
        .iter()
        .filter_map(|(tag, text)| Some((tag.clone(), text.trim().parse().ok()?)))
        .collect()
}

pub fn land(
    editor: &mut Editor,
    key: &str,
    export: AnimationExport,
    again: Option<&DueReimport>,
) -> Option<super::ImportDialog> {
    let row = editor.row_value(TABLE, key);
    let record = editor.imports.record_for(TABLE, key).cloned();
    let sheet = row.as_ref().and_then(|row| sheet_of(editor, row));
    if let (Some(due), Some(record), Some(row)) = (again, &record, &row) {
        let mapping = model::recorded_mapping(record, &export);
        let plan = model::plan(
            Some(row),
            sheet.as_ref(),
            &export,
            &mapping,
            &BTreeMap::new(),
            Some(record),
        );
        let auto = plan.decided() && !due.while_closed;
        let draft = ModelReimport {
            export,
            mapping,
            plan,
            auto,
            while_closed: due.while_closed,
            textures: TextureCache::default(),
            sheet,
            old_frame: frame_size_of(row),
        };
        if !auto {
            editor.open_tab(EditorTab::Model {
                key: key.to_owned(),
            });
            editor.select(RowSelection::parts(
                TABLE,
                key,
                vec![super::IMPORT_PART.to_owned()],
            ));
            editor.toast(format!(
                "{} changed{} · {} to decide in model/{key}",
                record.names(),
                if due.while_closed {
                    " while the editor was closed"
                } else {
                    ""
                },
                to_decide(&draft.plan)
            ));
        }
        editor.views.model.reimports.insert(key.to_owned(), draft);
        return None;
    }
    Some(super::ImportDialog::Model(Box::new(new_dialog(
        key.to_owned(),
        row.is_none(),
        export,
        row,
        sheet,
        record,
    ))))
}

pub fn land_new(editor: &mut Editor, export: AnimationExport) -> Option<super::ImportDialog> {
    let wanted = export
        .files
        .first()
        .map(|file| formats::stem(file).replace(['-', ' '], "_").to_lowercase())
        .unwrap_or_else(|| "new_model".to_owned());
    let key = editor
        .project
        .as_ref()
        .map(|project| rows::free_key(&project.files, TABLE, &wanted))
        .unwrap_or(wanted);
    Some(super::ImportDialog::Model(Box::new(new_dialog(
        key, true, export, None, None, None,
    ))))
}

fn new_dialog(
    key: String,
    new_row: bool,
    export: AnimationExport,
    row: Option<Value>,
    sheet: Option<RgbaImage>,
    record: Option<ImportRecord>,
) -> ModelDialog {
    let (mut mapping, notes) = model::default_mapping(&export);
    if let Some(record) = &record {
        let recorded = model::recorded_mapping(record, &export);
        for (tag, target) in recorded {
            if target.is_some() {
                mapping.insert(tag, target);
            }
        }
    }
    let asked = export
        .strips
        .iter()
        .map(|strip| (strip.tag.clone(), formats::DEFAULT_MS.to_string()))
        .collect();
    let grid = match export.kind {
        ExportFormat::Grid => (String::new(), String::new()),
        _ => (export.frame.0.to_string(), export.frame.1.to_string()),
    };
    let selected = export.strips.first().map(|strip| strip.tag.clone());
    let mut dialog = ModelDialog {
        key,
        new_row,
        export,
        mapping,
        notes,
        asked,
        grid,
        plan: ModelImportPlan {
            strips: Vec::new(),
            missing: Vec::new(),
            frame: (0, 0),
            trimmed: false,
            keeps_hitbox: false,
            busts: 0,
            cues_total: 0,
        },
        selected,
        textures: TextureCache::default(),
        record,
        sheet,
        error: None,
    };
    dialog.plan = model::plan(
        row.as_ref(),
        dialog.sheet.as_ref(),
        &dialog.export,
        &dialog.mapping,
        &asked_ms(&dialog.asked),
        dialog.record.as_ref(),
    );
    dialog
}

fn recompute(editor: &Editor, dialog: &mut ModelDialog) {
    let row = (!dialog.new_row)
        .then(|| editor.row_value(TABLE, &dialog.key))
        .flatten();
    let choices: BTreeMap<(String, usize, FrameCue), CuePlacement> = dialog
        .plan
        .strips
        .iter()
        .flat_map(|strip| {
            strip.cues.iter().filter_map(|cue| {
                cue.choice
                    .map(|choice| ((strip.name.clone(), cue.from, cue.cue.clone()), choice))
            })
        })
        .collect();
    let kept: BTreeMap<String, bool> = dialog
        .plan
        .strips
        .iter()
        .filter_map(|strip| {
            strip
                .retimed
                .as_ref()
                .map(|r| (strip.name.clone(), r.keep_here))
        })
        .collect();
    dialog.plan = model::plan(
        row.as_ref(),
        dialog.sheet.as_ref(),
        &dialog.export,
        &dialog.mapping,
        &asked_ms(&dialog.asked),
        dialog.record.as_ref(),
    );
    for strip in &mut dialog.plan.strips {
        for cue in &mut strip.cues {
            if let Some(choice) = choices.get(&(strip.name.clone(), cue.from, cue.cue.clone())) {
                cue.choice = Some(*choice);
            }
        }
        if let (Some(retimed), Some(keep)) = (&mut strip.retimed, kept.get(&strip.name)) {
            retimed.keep_here = *keep;
        }
    }
}

fn to_decide(plan: &ModelImportPlan) -> String {
    let mut parts = Vec::new();
    match plan.needs_place() {
        0 => {}
        1 => parts.push("1 cue to place".to_owned()),
        n => parts.push(format!("{n} cues to place")),
    }
    match plan.retimed() {
        0 => {}
        1 => parts.push("1 strip retimed here".to_owned()),
        n => parts.push(format!("{n} strips retimed here")),
    }
    if !plan.missing.is_empty() {
        parts.push(format!("{} strips without a tag", plan.missing.len()));
    }
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(" · ")
    }
}

pub fn dialog(editor: &mut Editor, ctx: &Context, mut dialog: ModelDialog) -> Option<ModelDialog> {
    let mut close = false;
    let mut import = false;
    let mut changed = false;
    let width = 1130.0;
    dialogs::card(ctx, "import-model", width, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🗋").size(16.0).color(theme::MUTED));
            ui.label(
                theme::strong(if dialog.new_row {
                    "Import a model".to_owned()
                } else {
                    format!("Import into model/{}", dialog.key)
                })
                .size(17.0),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (index, file) in dialog.export.files.iter().enumerate() {
                if index > 0 {
                    ui.label(theme::muted("+"));
                }
                ui.label(theme::code(super::show_path(editor, file)));
                if index == 1 && dialog.export.files.len() > 2 {
                    ui.label(theme::muted(format!(
                        "… {} files",
                        dialog.export.files.len()
                    )));
                    break;
                }
            }
            ui.label(theme::muted(format!(
                "· {} frames of {} × {}{} · {} {}",
                dialog.export.frames(),
                dialog.export.frame.0,
                dialog.export.frame.1,
                if dialog.export.trimmed {
                    ", trimmed"
                } else {
                    ""
                },
                dialog.export.strips.len(),
                match dialog.export.kind {
                    ExportFormat::Grid => "rows",
                    ExportFormat::Sequence => "sequences",
                    _ => "tags",
                }
            )));
            if dialog.new_row {
                ui.add_space(12.0);
                ui.label(theme::muted("into"));
                ui.label(theme::mono("model/").color(theme::CODE));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut dialog.key)
                        .desired_width(160.0)
                        .font(egui::TextStyle::Monospace),
                );
                if response.changed() {
                    dialog.error = None;
                }
                ui.label(theme::faint("a new row"));
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (kind, label) in [
                (ExportFormat::SheetJson, "sheet + JSON"),
                (ExportFormat::Grid, "grid"),
                (ExportFormat::Sequence, "sequence"),
                (ExportFormat::Gif, "GIF · APNG"),
            ] {
                let on = dialog.export.kind == kind
                    || (kind == ExportFormat::Gif && dialog.export.kind == ExportFormat::Apng);
                widgets::mode_chip(ui, label, on);
            }
            ui.label(theme::faint("picked from the files dropped"));
            if dialog.export.kind == ExportFormat::Grid {
                ui.add_space(12.0);
                ui.label(theme::muted("frame"));
                let w = ui.add(egui::TextEdit::singleline(&mut dialog.grid.0).desired_width(40.0));
                ui.label(theme::muted("×"));
                let h = ui.add(egui::TextEdit::singleline(&mut dialog.grid.1).desired_width(40.0));
                ui.label(theme::muted("px"));
                if w.changed() || h.changed() {
                    if let (Ok(w), Ok(h)) =
                        (dialog.grid.0.trim().parse(), dialog.grid.1.trim().parse())
                    {
                        match dialog.export.cut_grid(w, h) {
                            Ok(()) => {
                                let (mapping, notes) = model::default_mapping(&dialog.export);
                                dialog.mapping = mapping;
                                dialog.notes = notes;
                                dialog.asked = dialog
                                    .export
                                    .strips
                                    .iter()
                                    .map(|strip| {
                                        (strip.tag.clone(), formats::DEFAULT_MS.to_string())
                                    })
                                    .collect();
                                dialog.selected =
                                    dialog.export.strips.first().map(|s| s.tag.clone());
                                dialog.error = None;
                            }
                            Err(error) => dialog.error = Some(error),
                        }
                    }
                    changed = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(width - 330.0, 430.0), |ui| {
                ui.set_width(width - 330.0);
                egui::ScrollArea::vertical()
                    .id_salt("import-model-table")
                    .max_height(430.0)
                    .show(ui, |ui| {
                        changed |= mapping_table(ui, &mut dialog);
                    });
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.set_width(300.0);
                changed |= preview(ctx, ui, &mut dialog);
                ui.add_space(8.0);
                checklist(ui, &dialog);
            });
        });
        ui.add_space(8.0);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ready = dialog.plan.ready();
            let key_problem = dialog
                .new_row
                .then(|| {
                    editor.project.as_ref().and_then(|project| {
                        rows::key_problem(&project.files, TABLE, dialog.key.trim())
                    })
                })
                .flatten();
            let enabled = ready.is_ok() && key_problem.is_none() && dialog.error.is_none();
            if ui
                .add_enabled(enabled, egui::Button::new("Import, as one step"))
                .clicked()
            {
                import = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            let status = dialog
                .error
                .clone()
                .or(key_problem)
                .or_else(|| ready.err())
                .unwrap_or_else(|| {
                    if dialog.export.timed() {
                        "plays the tag with its lengths".to_owned()
                    } else {
                        "grids and sequences carry no lengths, so each strip asks one".to_owned()
                    }
                });
            ui.label(theme::muted(status));
        });
    });
    if changed {
        recompute(editor, &mut dialog);
    }
    if import {
        match apply_dialog(editor, &dialog) {
            Ok(()) => return None,
            Err(error) => dialog.error = Some(error),
        }
    }
    if close {
        return None;
    }
    Some(dialog)
}

fn mapping_table(ui: &mut Ui, dialog: &mut ModelDialog) -> bool {
    let mut changed = false;
    let names = model::strip_names();
    let plain: Vec<String> = dialog
        .export
        .strips
        .iter()
        .filter(|strip| {
            matches!(
                dialog.notes.get(&strip.tag),
                Some(MappingNote::ByName | MappingNote::ByOrder)
            ) && dialog.mapping.get(&strip.tag).is_some_and(Option::is_some)
                && !has_issue(&dialog.plan, &dialog.mapping, &strip.tag)
        })
        .map(|strip| strip.tag.clone())
        .collect();
    let folded: Vec<String> = plain.iter().skip(2).cloned().collect();
    egui::Grid::new("import-mapping")
        .num_columns(4)
        .spacing(Vec2::new(14.0, 6.0))
        .min_col_width(60.0)
        .show(ui, |ui| {
            for header in ["TAG", "STRIP", "FRAMES", ""] {
                ui.label(theme::faint(header).size(11.0));
            }
            ui.end_row();
            let mut shown_fold = false;
            let tags: Vec<String> = dialog.export.strips.iter().map(|s| s.tag.clone()).collect();
            for tag in &tags {
                if folded.contains(tag) {
                    if !shown_fold {
                        shown_fold = true;
                        ui.label(theme::muted(format!("… {} more", folded.len())));
                        ui.label("");
                        ui.label("");
                        ui.horizontal(|ui| {
                            ui.label(theme::ok("✓"));
                            ui.label(theme::muted("by name"));
                        });
                        ui.end_row();
                    }
                    continue;
                }
                let count = dialog
                    .export
                    .strip(tag)
                    .map(|s| s.frames.len())
                    .unwrap_or(0);
                let selected = dialog.selected.as_deref() == Some(tag);
                let label = ui.add(
                    egui::Label::new(theme::mono(tag).color(if selected {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }))
                    .sense(Sense::click()),
                );
                if label.clicked() {
                    dialog.selected = Some(tag.clone());
                }
                let current = dialog.mapping.get(tag).cloned().flatten();
                let mut picked: Option<Option<String>> = None;
                widgets::combo_box(("import-strip", tag))
                    .width(150.0)
                    .selected_text(
                        current
                            .as_deref()
                            .map(model::pretty)
                            .unwrap_or_else(|| "skip".to_owned()),
                    )
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), "skip").clicked() {
                            picked = Some(None);
                        }
                        for name in &names {
                            if ui
                                .selectable_label(
                                    current.as_deref() == Some(name),
                                    model::pretty(name),
                                )
                                .clicked()
                            {
                                picked = Some(Some(name.clone()));
                            }
                        }
                    });
                if let Some(picked) = picked {
                    dialog.mapping.insert(tag.clone(), picked);
                    dialog.notes.insert(tag.clone(), MappingNote::Picked);
                    changed = true;
                }
                ui.label(theme::mono(count.to_string()).color(theme::MUTED));
                ui.vertical(|ui| {
                    changed |= note_cell(ui, dialog, tag);
                });
                ui.end_row();
            }
            let missing = dialog.plan.missing.clone();
            for name in missing {
                ui.label(theme::mono("no tag").color(theme::DANGER));
                ui.label(theme::mono(model::pretty(&name)));
                ui.label("");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(theme::warn("⚠"));
                        ui.label(theme::muted("every model needs this strip"));
                    });
                    let mut picked = None;
                    widgets::combo_box(("import-pick-tag", &name))
                        .width(120.0)
                        .selected_text("pick a tag")
                        .show_ui(ui, |ui| {
                            for strip in &dialog.export.strips {
                                if ui.selectable_label(false, &strip.tag).clicked() {
                                    picked = Some(strip.tag.clone());
                                }
                            }
                        });
                    if let Some(tag) = picked {
                        dialog.mapping.insert(tag.clone(), Some(name.clone()));
                        dialog.notes.insert(tag, MappingNote::Picked);
                        changed = true;
                    }
                });
                ui.end_row();
            }
        });
    changed
}

fn has_issue(plan: &ModelImportPlan, mapping: &StripMapping, tag: &str) -> bool {
    let Some(Some(target)) = mapping.get(tag) else {
        return false;
    };
    plan.strips
        .iter()
        .find(|strip| strip.name == *target)
        .is_some_and(|strip| strip.needs_place() > 0 || strip.retimed.is_some())
}

fn note_cell(ui: &mut Ui, dialog: &mut ModelDialog, tag: &str) -> bool {
    let mut changed = false;
    let target = dialog.mapping.get(tag).cloned().flatten();
    let Some(target) = target else {
        let note = dialog
            .notes
            .get(tag)
            .map(MappingNote::text)
            .unwrap_or_else(|| "skipped".to_owned());
        ui.label(theme::faint(note));
        return false;
    };
    let mapped_twice = dialog
        .mapping
        .iter()
        .filter(|(other, t)| **other != tag && t.as_deref() == Some(target.as_str()))
        .count()
        > 0;
    if mapped_twice {
        ui.horizontal(|ui| {
            ui.label(theme::warn("⚠"));
            ui.label(theme::muted(format!(
                "another tag already lands on {}; the first wins",
                model::pretty(&target)
            )));
        });
    }
    let Some(strip) = dialog.plan.strip_mut(&target) else {
        return false;
    };
    let count = strip.after_count();
    let was = strip.before.len();
    let mut any = false;
    for cue in &mut strip.cues {
        if cue.settled() {
            continue;
        }
        any = true;
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::warn("⚠"));
            ui.label(theme::muted(format!(
                "was {was} frames; the {} cue on frame {} has no frame now",
                cue.cue.label(),
                cue.from + 1
            )));
        });
        ui.horizontal(|ui| {
            ui.label(theme::muted("put it on"));
            let offered = cue.offered(count);
            let mut picked = None;
            widgets::combo_box(("import-place", &target, cue.from, cue.cue.label()))
                .width(100.0)
                .selected_text(theme::mono(format!("frame {}", offered + 1)).color(theme::FAINT))
                .show_ui(ui, |ui| {
                    for frame in 0..count {
                        if ui
                            .selectable_label(false, format!("frame {}", frame + 1))
                            .clicked()
                        {
                            picked = Some(CuePlacement::Frame(frame));
                        }
                    }
                    if ui.selectable_label(false, "remove the cue").clicked() {
                        picked = Some(CuePlacement::Remove);
                    }
                });
            if let Some(picked) = picked {
                cue.choice = Some(picked);
                changed = true;
            }
        });
    }
    if let Some(retimed) = &mut strip.retimed {
        any = true;
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::warn("⚠"));
            ui.label(theme::muted("retimed here since the last import"));
        });
        ui.horizontal(|ui| {
            ui.label(theme::muted("keep"));
            let mut picked = None;
            widgets::combo_box(("import-keep", &target))
                .width(150.0)
                .selected_text(if retimed.keep_here {
                    "the lengths set here"
                } else {
                    "the export's"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(retimed.keep_here, "the lengths set here")
                        .clicked()
                    {
                        picked = Some(true);
                    }
                    if ui
                        .selectable_label(!retimed.keep_here, "the export's")
                        .clicked()
                    {
                        picked = Some(false);
                    }
                });
            if let Some(picked) = picked {
                retimed.keep_here = picked;
            }
        });
    }
    if !any && !mapped_twice {
        let note = dialog
            .notes
            .get(tag)
            .map(MappingNote::text)
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(theme::ok("✓"));
            ui.label(theme::muted(note));
        });
    }
    changed
}

fn preview(ctx: &Context, ui: &mut Ui, dialog: &mut ModelDialog) -> bool {
    let mut changed = false;
    let Some(tag) = dialog.selected.clone() else {
        return false;
    };
    let Some(strip) = dialog.export.strip(&tag).cloned() else {
        return false;
    };
    widgets::section(ui, &format!("{tag} · {} frames", strip.frames.len()));
    let per_row = 8;
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(288.0);
            for chunk in strip.frames.chunks(per_row) {
                ui.horizontal(|ui| {
                    for frame in chunk {
                        let texture = dialog.textures.of(ctx, &frame.hash, &frame.image);
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                        draw_fit(ui.painter(), rect, &texture, dialog.export.frame);
                    }
                });
            }
        });
    ui.horizontal(|ui| {
        if dialog.export.timed() {
            for frame in strip.frames.iter().take(per_row) {
                ui.allocate_ui(Vec2::new(32.0, 14.0), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            theme::mono(frame.ms.unwrap_or(0).to_string())
                                .size(10.5)
                                .color(theme::MUTED),
                        );
                    });
                });
            }
            if strip.frames.len() > per_row {
                ui.label(theme::faint("…"));
            }
        } else {
            let text = dialog
                .asked
                .entry(tag.clone())
                .or_insert_with(|| formats::DEFAULT_MS.to_string());
            let response = ui.add(egui::TextEdit::singleline(text).desired_width(50.0));
            ui.label(theme::muted("ms each · no lengths in the export"));
            if response.changed() {
                changed = true;
            }
        }
    });
    changed
}

fn checklist(ui: &mut Ui, dialog: &ModelDialog) {
    widgets::section(ui, "What the import does");
    let plan = &dialog.plan;
    let (fw, fh) = plan.frame;
    let line = |ui: &mut Ui, ok: bool, text: String| {
        ui.horizontal_wrapped(|ui| {
            if ok {
                ui.label(theme::ok("✓"));
            } else {
                ui.label(theme::warn("⚠"));
            }
            ui.label(theme::muted(text).size(12.5));
        });
    };
    if plan.trimmed {
        line(
            ui,
            true,
            format!(
                "Frames trimmed in the export go back to their place on the {fw} × {fh} canvas."
            ),
        );
    } else {
        line(
            ui,
            true,
            format!("Frames lie on a {fw} × {fh} grid, one per cell."),
        );
    }
    if dialog.new_row {
        line(
            ui,
            true,
            "A new row with the sheet, the hitbox and cues set after.".to_owned(),
        )
    } else {
        line(
            ui,
            true,
            "Pixels and frame lengths replace the current ones.".to_owned(),
        )
    }
    if !dialog.new_row {
        let kept = plan.cues_kept();
        line(
            ui,
            true,
            format!(
                "Keeps {}{} and {kept} of {} cues on their frames.",
                if plan.keeps_hitbox {
                    "the hitbox"
                } else {
                    "the row"
                },
                match plan.busts {
                    0 => String::new(),
                    _ => ", the bust".to_owned(),
                },
                plan.cues_total
            ),
        );
    }
    match plan.needs_place() {
        0 => {}
        1 => line(
            ui,
            false,
            "One cue needs the place picked on the left.".to_owned(),
        ),
        n => line(
            ui,
            false,
            format!("{n} cues need their place picked on the left."),
        ),
    }
    match plan.retimed() {
        0 => {}
        n => line(
            ui,
            false,
            format!(
                "{n} strip{} retimed here: pick which lengths win.",
                if n == 1 { " was" } else { "s were" }
            ),
        ),
    }
    match plan.missing.len() {
        0 => {}
        1 => line(
            ui,
            false,
            format!(
                "{} has no tag; every model needs all {} strips.",
                model::pretty(&plan.missing[0]),
                plan.strips.len()
            ),
        ),
        n => line(
            ui,
            false,
            format!(
                "{n} strips have no tag; every model needs all {}.",
                plan.strips.len()
            ),
        ),
    }
    for warning in &dialog.export.warnings {
        line(ui, false, warning.clone());
    }
}

fn draw_fit(painter: &egui::Painter, rect: Rect, texture: &TextureHandle, frame: (u32, u32)) {
    let (fw, fh) = (frame.0.max(1) as f32, frame.1.max(1) as f32);
    let scale = (rect.width() / fw).min(rect.height() / fh);
    let size = Vec2::new(fw * scale, fh * scale);
    let fitted = Rect::from_center_size(rect.center(), size);
    painter.image(
        texture.id(),
        fitted,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

pub fn apply_now(editor: &mut Editor, mut dialog: ModelDialog) -> Option<ModelDialog> {
    match apply_dialog(editor, &dialog) {
        Ok(()) => None,
        Err(error) => {
            dialog.error = Some(error);
            Some(dialog)
        }
    }
}

fn apply_dialog(editor: &mut Editor, dialog: &ModelDialog) -> Result<(), String> {
    let key = dialog.key.trim().to_owned();
    let row = if dialog.new_row {
        let project = editor.project.as_ref().ok_or("no folder is open")?;
        if let Some(problem) = rows::key_problem(&project.files, TABLE, &key) {
            return Err(problem);
        }
        let shape = editor
            .shapes
            .table(TABLE)
            .ok_or("the model table has no shape")?;
        Some(schema::template(&shape.shape, &shape.shape))
    } else {
        editor.row_value(TABLE, &key)
    };
    let built = model::build(&dialog.plan, &dialog.export, row.as_ref(), &key)?;
    let names = ImportRecord::of_files(&dialog.export.files, TABLE, &key).names();
    let label = if dialog.new_row {
        format!("Imported {names} as model/{key}")
    } else {
        format!("Imported {names} into {key}")
    };
    let record = model::record(&dialog.export, &key, &dialog.mapping, built.lengths.clone());
    write(editor, &key, built, label, record)?;
    editor.select(RowSelection::row(TABLE, &key));
    editor.open_tab(EditorTab::Model { key });
    Ok(())
}

fn write(
    editor: &mut Editor,
    key: &str,
    built: model::BuiltModel,
    label: String,
    record: ImportRecord,
) -> Result<(), String> {
    let mut edit = ContentEdit::new(label);
    edit.write(
        format!("assets/{}", built.image),
        std::sync::Arc::from(built.png),
    );
    edit.write_json(row_file(TABLE, key), &built.row);
    editor.try_apply(edit)?;
    editor.imports.remember(record);
    editor.views.model.reimports.remove(key);
    Ok(())
}

pub fn apply_settled(editor: &mut Editor) {
    let keys: Vec<String> = editor
        .views
        .model
        .reimports
        .iter()
        .filter(|(_, draft)| draft.auto && draft.plan.decided())
        .map(|(key, _)| key.clone())
        .collect();
    for key in keys {
        apply_reimport(editor, &key, true);
    }
}

pub fn apply_reimport(editor: &mut Editor, key: &str, auto: bool) {
    let Some(draft) = editor.views.model.reimports.get(key) else {
        return;
    };
    let row = editor.row_value(TABLE, key);
    let built = match model::build(&draft.plan, &draft.export, row.as_ref(), key) {
        Ok(built) => built,
        Err(error) => {
            editor.toast(format!("Not imported: {error}"));
            return;
        }
    };
    let names = ImportRecord::of_files(&draft.export.files, TABLE, key).names();
    let changed = draft.plan.changed().len();
    let label = if auto {
        format!("Imported {names} into {key} again · nothing to decide")
    } else {
        format!(
            "Imported {names} into {key} again · {changed} strip{}, cues followed",
            widgets::plural(changed)
        )
    };
    let record = model::record(&draft.export, key, &draft.mapping, built.lengths.clone());
    if let Err(error) = write(editor, key, built, label, record) {
        editor.toast(error);
    }
}

pub fn show_reimport(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let ctx = ui.ctx().clone();
    let Some(draft) = editor.views.model.reimports.get_mut(key) else {
        return;
    };
    let names = ImportRecord::of_files(&draft.export.files, TABLE, key).names();
    let changed: Vec<usize> = draft
        .plan
        .strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| {
            strip.changed_count() || strip.retimed.is_some() || strip.needs_place() > 0
        })
        .map(|(index, _)| index)
        .collect();
    egui::Frame::new()
        .fill(theme::INPUT)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::strong(format!("{names} changed")));
                ui.label(theme::muted(format!(
                    "imported on this machine{} · {} strip{} · {}",
                    if draft.while_closed {
                        ", while the editor was closed"
                    } else {
                        ""
                    },
                    changed.len(),
                    widgets::plural(changed.len()),
                    to_decide(&draft.plan)
                )));
            });
        });
    ui.add_space(10.0);
    egui::ScrollArea::vertical()
        .id_salt(("reimport", key))
        .show(ui, |ui| {
            if changed.is_empty() {
                ui.label(theme::muted(
                    "Every strip keeps its frame count: pixels redrawn, every cue on its frame.",
                ));
            }
            let sheet = draft.sheet.clone();
            let frames = Frames {
                export: &draft.export,
                sheet: &sheet,
                old_frame: draft.old_frame,
                frame: draft.plan.frame,
            };
            for index in changed {
                let strip = draft.plan.strips[index].clone();
                strip_section(&ctx, ui, &mut draft.textures, &frames, &strip);
                ui.add_space(14.0);
            }
        });
    editor.status.left = vec![
        format!("model/{key} · its exports changed"),
        "the game keeps the old frames until the import applies".to_owned(),
    ];
}

struct Frames<'a> {
    export: &'a AnimationExport,
    sheet: &'a Option<RgbaImage>,
    old_frame: (u32, u32),
    frame: (u32, u32),
}

fn strip_section(
    ctx: &Context,
    ui: &mut Ui,
    textures: &mut TextureCache,
    frames: &Frames,
    strip: &StripPlan,
) {
    let Frames {
        export,
        sheet,
        old_frame,
        frame,
    } = *frames;
    widgets::section(
        ui,
        &format!(
            "{} · {} → {} frames",
            model::pretty(&strip.name),
            strip.before.len(),
            strip.after_count()
        ),
    );
    let gap = 8.0;
    let height = 2.0 * (FRAME_CARD + 20.0) + 60.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let label_w = 60.0;
    let row_y = |row: usize| rect.min.y + 6.0 + row as f32 * (FRAME_CARD + 20.0 + 50.0);
    for (row, title) in [(0usize, "before"), (1, "after")] {
        painter.text(
            egui::pos2(rect.min.x, row_y(row) + FRAME_CARD / 2.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(12.5),
            theme::MUTED,
        );
    }
    let card_at = |row: usize, index: usize| {
        Rect::from_min_size(
            egui::pos2(
                rect.min.x + label_w + index as f32 * (FRAME_CARD + gap),
                row_y(row),
            ),
            Vec2::splat(FRAME_CARD),
        )
    };
    let (ofw, ofh) = old_frame;
    for (index, frame_data) in strip.before.iter().enumerate() {
        let card = card_at(0, index);
        painter.rect(
            card,
            3.0,
            theme::INPUT,
            egui::Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        if let Some(sheet) = sheet
            && ofw > 0
            && let Some(image) = frame_image(sheet, (ofw, ofh), frame_data.frame)
        {
            let hash = formats::pixel_hash(&image);
            let texture = textures.of(ctx, &hash, &image);
            draw_fit(&painter, card.shrink(2.0), &texture, (ofw, ofh));
        }
        let cues = cue_labels(frame_data);
        painter.text(
            egui::pos2(card.center().x, card.max.y + 10.0),
            egui::Align2::CENTER_CENTER,
            format!(
                "{}{}",
                index + 1,
                if cues.is_empty() {
                    String::new()
                } else {
                    format!(" {cues}")
                }
            ),
            egui::FontId::monospace(11.0),
            if cues.is_empty() {
                theme::FAINT
            } else {
                theme::SELECTION
            },
        );
    }
    let source = strip.tag.as_ref().and_then(|tag| export.strip(tag));
    let after = strip.after();
    for (index, frame_data) in after.iter().enumerate() {
        let card = card_at(1, index);
        let waiting = strip
            .cues
            .iter()
            .any(|cue| !cue.settled() && cue.offered(after.len()) == index);
        painter.rect(
            card,
            3.0,
            theme::INPUT,
            egui::Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        if let Some(source) = source
            && let Some(export_frame) = source.frames.get(index)
        {
            let texture = textures.of(ctx, &export_frame.hash, &export_frame.image);
            draw_fit(&painter, card.shrink(2.0), &texture, frame);
        }
        if waiting {
            widgets::dashed_rect(
                &painter,
                card,
                egui::Stroke::new(1.5, theme::DANGER),
                4.0,
                3.0,
            );
        }
        let cues = cue_labels(frame_data);
        let text = match (cues.is_empty(), waiting) {
            (true, true) => format!("{} ?", index + 1),
            (true, false) => format!("{}", index + 1),
            (false, _) => format!("{} {cues}", index + 1),
        };
        painter.text(
            egui::pos2(card.center().x, card.max.y + 10.0),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::monospace(11.0),
            if waiting {
                theme::DANGER
            } else if cues.is_empty() {
                theme::FAINT
            } else {
                theme::SELECTION
            },
        );
    }
    for cue in &strip.cues {
        let Some(to) = cue.lands() else {
            continue;
        };
        let from_card = card_at(0, cue.from);
        let to_card = card_at(1, to);
        let start = egui::pos2(from_card.center().x, from_card.max.y + 20.0);
        let end = egui::pos2(to_card.center().x, to_card.min.y - 4.0);
        painter.line_segment([start, end], egui::Stroke::new(1.5, theme::SELECTION));
        let dir = (end - start).normalized();
        let side = Vec2::new(-dir.y, dir.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                end,
                end - dir * 8.0 + side * 4.0,
                end - dir * 8.0 - side * 4.0,
            ],
            theme::SELECTION,
            egui::Stroke::NONE,
        ));
    }
}

fn frame_image(sheet: &RgbaImage, frame: (u32, u32), index: u32) -> Option<RgbaImage> {
    let (fw, fh) = frame;
    let columns = sheet.width() / fw.max(1);
    if columns == 0 {
        return None;
    }
    let x = (index % columns) * fw;
    let y = (index / columns) * fh;
    if x + fw > sheet.width() || y + fh > sheet.height() {
        return None;
    }
    Some(image::imageops::crop_imm(sheet, x, y, fw, fh).to_image())
}

fn cue_labels(frame: &StripFrame) -> String {
    let mut labels = Vec::new();
    if frame.apex {
        labels.push("apex".to_owned());
    }
    if frame.step {
        labels.push("step".to_owned());
    }
    if let Some(sound) = &frame.sound {
        labels.push(sound.clone());
    }
    labels.join(" ")
}

pub fn inspect_reimport(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let mut apply = false;
    let mut keep = false;
    let Some(draft) = editor.views.model.reimports.get_mut(key) else {
        return;
    };
    ui.label(
        theme::mono(format!("model/{key} › import"))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(6.0);
    for strip in &mut draft.plan.strips {
        if strip.cues.iter().all(|cue| !cue.by_pixels) && strip.needs_place() == 0 {
            continue;
        }
        let count = strip.after_count();
        widgets::card_frame(strip.needs_place() > 0).show(ui, |ui| {
            ui.vertical(|ui| {
                widgets::card_title(ui, &model::pretty(&strip.name));
                let mut followed: Vec<String> = Vec::new();
                for cue in &strip.cues {
                    if let Some(to) = cue.to
                        && cue.by_pixels
                        && to != cue.from
                    {
                        followed.push(format!("{} {} → {}", cue.cue.label(), cue.from + 1, to + 1));
                    } else if cue.to.is_some() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::ok("✓"));
                            ui.label(theme::muted(format!(
                                "{} on frame {} stays.",
                                cue.cue.label(),
                                cue.from + 1
                            )));
                        });
                    }
                }
                if !followed.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::ok("✓"));
                        ui.label(theme::muted(format!(
                            "{} follow{} their frame{}. Found by its pixels.",
                            followed.join(", "),
                            if followed.len() == 1 { "s" } else { "" },
                            widgets::plural(followed.len())
                        )));
                    });
                }
                for cue in &mut strip.cues {
                    if cue.to.is_some() {
                        continue;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::warn("⚠"));
                        ui.label(theme::muted(format!(
                            "{} on frame {}: its frame is gone.",
                            cue.cue.label(),
                            cue.from + 1
                        )));
                    });
                    ui.horizontal(|ui| {
                        ui.label(theme::muted("put it on"));
                        let offered = cue.offered(count);
                        let shown = match cue.choice {
                            Some(CuePlacement::Frame(frame)) => format!("frame {}", frame + 1),
                            Some(CuePlacement::Remove) => "remove the cue".to_owned(),
                            None => format!("frame {}", offered + 1),
                        };
                        let mut picked = None;
                        widgets::combo_box((
                            "reimport-place",
                            &strip.name,
                            cue.from,
                            cue.cue.label(),
                        ))
                        .width(ui.available_width().min(200.0))
                        .selected_text(theme::mono(shown).color(if cue.choice.is_some() {
                            theme::TEXT
                        } else {
                            theme::FAINT
                        }))
                        .show_ui(ui, |ui| {
                            for frame in 0..count {
                                if ui
                                    .selectable_label(
                                        cue.choice == Some(CuePlacement::Frame(frame)),
                                        format!("frame {}", frame + 1),
                                    )
                                    .clicked()
                                {
                                    picked = Some(CuePlacement::Frame(frame));
                                }
                            }
                            if ui
                                .selectable_label(
                                    cue.choice == Some(CuePlacement::Remove),
                                    "remove the cue",
                                )
                                .clicked()
                            {
                                picked = Some(CuePlacement::Remove);
                            }
                        });
                        if let Some(picked) = picked {
                            cue.choice = Some(picked);
                        }
                    });
                }
            });
        });
        if strip.needs_place() > 0 {
            let offered = strip
                .cues
                .iter()
                .find(|cue| !cue.settled())
                .map(|cue| cue.offered(count))
                .unwrap_or(0);
            let origin = strip.origins.get(offered).copied().flatten();
            widgets::hint(
                ui,
                &match origin {
                    Some(old) => format!(
                        "Frame {} is the old frame {}. Remove the cue is the other choice.",
                        offered + 1,
                        old + 1
                    ),
                    None => format!(
                        "Frame {} is new. Remove the cue is the other choice.",
                        offered + 1
                    ),
                },
            );
        }
        ui.add_space(6.0);
    }
    for strip in &mut draft.plan.strips {
        let Some(retimed) = &mut strip.retimed else {
            continue;
        };
        widgets::card_frame(false).show(ui, |ui| {
            ui.vertical(|ui| {
                widgets::card_header(
                    ui,
                    &format!("{} · lengths", model::pretty(&strip.name)),
                    "",
                    |_| {},
                );
                let differing: Vec<String> = retimed
                    .frames()
                    .into_iter()
                    .map(|frame| frame.to_string())
                    .collect();
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::warn("⚠"));
                    ui.label(theme::muted(format!(
                        "Frame{} {} {} retimed here since the last import.",
                        widgets::plural(differing.len()),
                        widgets::join_and(&differing),
                        if differing.len() == 1 { "was" } else { "were" }
                    )));
                });
                ui.horizontal(|ui| {
                    ui.label(theme::muted("keep"));
                    let mut picked = None;
                    widgets::combo_box(("reimport-keep", &strip.name))
                        .width(ui.available_width().min(200.0))
                        .selected_text(theme::mono(if retimed.keep_here {
                            "the lengths set here"
                        } else {
                            "the export's"
                        }))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(retimed.keep_here, "the lengths set here")
                                .clicked()
                            {
                                picked = Some(true);
                            }
                            if ui
                                .selectable_label(!retimed.keep_here, "the export's")
                                .clicked()
                            {
                                picked = Some(false);
                            }
                        });
                    if let Some(picked) = picked {
                        retimed.keep_here = picked;
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
    ui.add_space(4.0);
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
        if ui.button("Keep the old frames").clicked() {
            keep = true;
        }
    });
    if let Err(reason) = ready {
        widgets::hint(ui, &reason);
    }
    if apply {
        apply_reimport(editor, key, false);
    }
    if keep {
        editor.views.model.reimports.remove(key);
        if let Some(record) = editor.imports.record_for(TABLE, key).cloned() {
            let mut refreshed = ImportRecord::of_files(&record.files(), TABLE, key);
            refreshed.mapping = record.mapping;
            refreshed.strips = record.strips;
            editor.imports.remember(refreshed);
        }
        editor.toast(format!(
            "Kept the old frames of {key}; the export is remembered as it is now"
        ));
    }
}
