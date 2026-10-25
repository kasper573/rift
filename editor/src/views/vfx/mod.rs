//! The vfx views: a vfx row beside its preview, and a shader row beside its code and preview.

mod params;
mod preview;
mod shader_code;

pub use params::{ParamSpec, param_fill, params_of, shader_params};
pub use preview::VfxViews;

use std::sync::Arc;
use ui::egui;
use ui::form;
use ui::sheet;
use ui::theme;

use engine::core::content::{ContentFile, ContentScope, ErrorPlace, fields_file, rescoped};
use engine::core::time::Seconds;
use engine::core::vfx::{ParamFill, VfxDef, VfxLayer, VfxShaderDef, shader_file};
use ui::egui::{Color32, FontId, Ui, Vec2 as EVec2};

use self::preview::preview;
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::shell::{row_form, widgets};
use crate::tabs::file_title;

const FORM_WIDTH: f32 = 440.0;
const COLUMN_GAP: f32 = 16.0;
const MIN_CODE_WIDTH: f32 = 260.0;
const HEADING_HEIGHT: f32 = 24.0;
const MIN_TITLE_WIDTH: f32 = 60.0;

pub fn show_vfx(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(project) = &editor.project else {
        return;
    };
    if !project.files.contains(&fields_file(key)) {
        ui.label(theme::muted(format!("{} is gone", fields_file(key))));
        return;
    }
    let vfx = project.read::<VfxDef>(key).ok();
    let specs = params_of(project, VfxDef::TABLE, key);
    let reader = project.reader();
    let (layers, inside) = match (editor.loaded(), vfx) {
        (Some(content), Some(vfx)) => {
            let loaded = ContentScope::of(&content);
            let layers = rescoped(&vfx.layers, reader.scope(), &loaded).unwrap_or_default();
            let inside = vfx
                .on_actors_inside
                .map(|inner| match reader.path_of(inner) {
                    Some(inner) if inner == key => layers.clone(),
                    inner => inner
                        .and_then(|inner| content.by_path::<VfxDef>(inner))
                        .map_or_else(Vec::new, |id| id.get(&content).layers.clone()),
                });
            (layers, inside)
        }
        _ => (Vec::new(), None),
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(FORM_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt(("vfx-form", key))
                .auto_shrink([false, false])
                .show(ui, |ui| row_form::show_row(editor, ui, VfxDef::TABLE, key));
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            sheet::section(ui, "preview");
            preview(
                editor,
                ui,
                &format!("{}/{key}", VfxDef::TABLE),
                layers,
                inside,
                &specs,
            );
        });
    });
}

pub fn show_shader(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(read) = editor.read::<VfxShaderDef>(key) else {
        return;
    };
    let def = read.as_ref().ok();
    let file = shader_file(key);
    let current = editor
        .project
        .as_ref()
        .and_then(|project| project.files.text(&file))
        .unwrap_or_default();
    let room = ui.available_width() - 2.0 * COLUMN_GAP;
    let preview_width = crate::stage::preview_width(room - FORM_WIDTH - MIN_CODE_WIDTH);
    let code_width = (room - FORM_WIDTH - preview_width).max(MIN_CODE_WIDTH);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(FORM_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt(("shader-form", key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    row_form::show_row(editor, ui, VfxShaderDef::TABLE, key)
                });
        });
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(code_width);
            match &read {
                Ok(def) => code_pane(editor, ui, def, key, &file, &current),
                Err(errors) => {
                    sheet::section(ui, &format!("code · {file}"));
                    widgets::unreadable(ui, key, errors);
                }
            }
        });
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(preview_width);
            sheet::section(ui, "preview");
            let Some((def, shader)) = def.zip(
                editor
                    .loaded()
                    .and_then(|content| content.by_path::<VfxShaderDef>(key)),
            ) else {
                return;
            };
            let layer = VfxLayer::Shader {
                shader,
                params: def
                    .params
                    .iter()
                    .map(|param| {
                        let fill = ParamFill::Param {
                            param: param.name.clone(),
                        };
                        (param.name.clone(), fill)
                    })
                    .collect(),
                duration: Seconds(1.0),
                delay: Seconds(0.0),
            };
            let specs: Vec<ParamSpec> = def.params.iter().map(ParamSpec::from).collect();
            preview(
                editor,
                ui,
                &format!("{}/{key}", VfxShaderDef::TABLE),
                vec![layer],
                None,
                &specs,
            );
        });
    });
}

/// The shader's code, edited in place: every change is a step in the editor's history, merged with
/// the last, so the preview follows the typing and undo takes it back.
fn code_pane(
    editor: &mut Editor,
    ui: &mut Ui,
    def: &VfxShaderDef,
    key: &str,
    file: &str,
    current: &str,
) {
    let heading_slot = ui.available_rect_before_wrap();
    ui.add_space(HEADING_HEIGHT);
    let height = ui.available_height();
    let mut text = current.to_owned();
    let id = egui::Id::new(("shader-code", key));
    let (response, verdict) = shader_code::show(ui, id, &mut text, def, height);
    if response.changed() {
        let mut edit = ContentEdit::new(format!("Edit the code of {}", file_title(key))).merging();
        edit.write(file.to_owned(), Arc::from(text.as_bytes()));
        editor.apply(edit);
    }
    let status = match &verdict {
        Ok(()) => theme::ok("compiles"),
        Err(error) => {
            let place = match error.place {
                ErrorPlace::Text { line, column } => format!("line {line}:{column} · "),
                _ => String::new(),
            };
            theme::warn(format!("{place}{}", error.message))
        }
    };
    let heading = egui::Rect::from_min_size(
        heading_slot.min,
        EVec2::new(heading_slot.width(), HEADING_HEIGHT),
    );
    let title = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(heading)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let status = status.size(11.0);
            let status_width = ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        status.text().to_owned(),
                        FontId::proportional(11.0),
                        Color32::WHITE,
                    )
                    .size()
                    .x
            });
            let title_room = ui.available_width() - status_width - ui.spacing().item_spacing.x;
            let title = ui.scope(|ui| {
                ui.set_max_width(title_room.max(MIN_TITLE_WIDTH));
                ui.add(
                    egui::Label::new(form::section_heading(&format!("code · {file} · ")))
                        .truncate(),
                )
            });
            ui.add(egui::Label::new(status).truncate());
            title.inner
        },
    );
    response.labelled_by(title.inner.id);
}
