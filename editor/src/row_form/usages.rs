use crate::widgets::Badge;
use engine::core::content::FileSize;
use engine::core::content::NOTE_EXTENSION;
use ui::egui;
use ui::egui::{Align, Layout, Sense, Ui};
use ui::face;
use ui::form;
use ui::theme;

use super::widgets;
use crate::app::Editor;
use crate::project::files::content_holding;
use crate::tabs::file_title;
use crate::tabs::shown_path;

pub(crate) fn show_usages_grouped(editor: &mut Editor, ui: &mut Ui, key: &str, titled: bool) {
    let Some(project) = &editor.project else {
        return;
    };
    let mut groups: Vec<FieldUsages> = Vec::new();
    let mut total = 0;
    for usage in project.usages.of_content(key) {
        total += 1;
        let field = crate::rows::field_of(project, &usage.file, &usage.pointer);
        match groups
            .iter_mut()
            .find(|group| group.file == usage.file && group.field == field)
        {
            Some(group) => group.count += 1,
            None => groups.push(FieldUsages {
                file: usage.file.clone(),
                field,
                count: 1,
            }),
        }
    }
    let heading = if titled {
        format!("Used by · {}", file_title(key))
    } else {
        "Used by".to_owned()
    };
    form::section_count(ui, &heading, total);
    if groups.is_empty() {
        form::hint(ui, "nothing names this file");
    }
    let mut open = None;
    for group in &groups {
        let row = content_holding(&group.file);
        let name = row.map_or_else(|| group.file.clone(), str::to_owned);
        let response = ui
            .horizontal(|ui| {
                let table = row.and_then(engine::core::content::table_of);
                if group.field.is_empty() {
                    widgets::table_mark(ui, table.unwrap_or_default());
                } else {
                    widgets::badge(ui, Badge::of_tag(&group.field, table));
                }
                if !group.field.is_empty() && group.field != name {
                    ui.label(face::mono(&group.field).size(13.0));
                    ui.label(theme::faint("·"));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if group.count > 1 {
                        ui.label(
                            theme::muted(format!("× {}", group.count))
                                .monospace()
                                .size(12.0),
                        );
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(face::mono(name).size(13.0).color(theme::CODE))
                                .truncate(),
                        );
                    });
                });
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            open = row.map(str::to_owned);
        }
    }
    if let Some(path) = open {
        editor.open_path(&path);
    }
}

pub(crate) fn show_asset(editor: &mut Editor, ui: &mut Ui, path: &str) {
    ui.label(face::mono(path.to_owned()).color(theme::CODE));
    if path.ends_with(&format!(".{NOTE_EXTENSION}")) {
        show_note(editor, ui, path);
        return;
    }
    let ctx = ui.ctx().clone();
    let (thumb, size) = {
        let Editor { project, form, .. } = editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        let bytes = project
            .files
            .get(path)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let thumb = if path.to_lowercase().ends_with(".png") {
            form.thumbnails.image(&ctx, &project.files, path)
        } else {
            None
        };
        (thumb, bytes)
    };
    ui.label(theme::muted(FileSize(size).to_string()));
    if let Some(thumb) = thumb {
        let scale = ((ui.available_width() - 8.0) / thumb.size[0].max(1) as f32).min(4.0);
        ui.add(egui::Image::from_texture((
            thumb.texture.id(),
            egui::vec2(thumb.size[0] as f32 * scale, thumb.size[1] as f32 * scale),
        )));
        ui.label(theme::muted(format!(
            "{} × {} px",
            thumb.size[0], thumb.size[1]
        )));
    }
}

/// A note reads here as it is; it is written in an editor of the user's choosing.
fn show_note(editor: &Editor, ui: &mut Ui, path: &str) {
    let Some(project) = &editor.project else {
        return;
    };
    if ui.button("Open in your editor").clicked() {
        let _ = opener::open(project.root.join(path));
    }
    ui.add_space(6.0);
    let text = project.files.text(path).unwrap_or_default();
    ui.add(egui::Label::new(face::mono(text).color(theme::TEXT)).wrap());
}

pub(crate) fn show_asset_usages(editor: &mut Editor, ui: &mut Ui, path: &str) {
    let usages: Vec<(String, String)> = editor
        .project
        .as_ref()
        .map(|project| {
            project
                .usages
                .of_media(path)
                .iter()
                .map(|u| (u.file.clone(), u.pointer.clone()))
                .collect()
        })
        .unwrap_or_default();
    form::section_count(ui, "Used by", usages.len());
    if usages.is_empty() {
        form::hint(ui, "nothing names this file");
    }
    let mut open = None;
    for (file, pointer) in &usages {
        ui.horizontal(|ui| {
            if ui.link(face::mono(shown_path(file))).clicked() {
                open = Some(file.clone());
            }
            ui.label(theme::muted(pointer).monospace().size(12.0));
        });
    }
    if let Some(file) = open
        && let Some(path) = content_holding(&file)
    {
        editor.open_path(path);
    }
}

/// The usages one file makes of a row from the same field.
pub struct FieldUsages {
    file: String,
    field: String,
    count: usize,
}
