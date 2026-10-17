//! The vfx views: a vfx row beside its preview, and a shader row beside its code and preview.

mod params;
mod preview;
mod shader_code;

use std::sync::Arc;

use egui::{Color32, FontId, Ui, Vec2 as EVec2};
use engine::core::time::Seconds;
use engine::core::vfx::{ParamFill, VfxShaderDef, shader_file};

pub use params::{ParamSpec, param_fill, params_of, shader_params};
pub use preview::{Target, VfxViews, sync};

use self::preview::{Layer, layers_of, preview};
use crate::app::Editor;
use crate::drafts::{clear_draft, draft_or, set_draft};
use crate::project::{ContentEdit, row_file};
use crate::ui::{inspector, theme, widgets};

pub const TABLE: &str = "vfx";
pub const SHADER_TABLE: &str = "vfx_shader";

const FORM_WIDTH: f32 = 440.0;
const CODE_FOOTER: f32 = 40.0;
const COLUMN_GAP: f32 = 16.0;
const MIN_CODE_WIDTH: f32 = 260.0;
const MIN_PREVIEW_WIDTH: f32 = 220.0;
const HEADING_HEIGHT: f32 = 24.0;
const MIN_TITLE_WIDTH: f32 = 60.0;

pub fn show_vfx(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = editor.row_value(TABLE, key) else {
        ui.label(theme::muted(format!("{} is gone", row_file(TABLE, key))));
        return;
    };
    let layers = layers_of(&row);
    let specs = editor
        .project
        .as_ref()
        .map(|project| params_of(&project.files, TABLE, key))
        .unwrap_or_default();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(FORM_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt(("vfx-form", key))
                .auto_shrink([false, false])
                .show(ui, |ui| inspector::show_row(editor, ui, TABLE, key));
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            widgets::section(ui, "preview");
            preview(editor, ui, &format!("{TABLE}/{key}"), layers, &specs, &[]);
        });
    });
}

pub fn show_shader(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = editor.row_value(SHADER_TABLE, key) else {
        ui.label(theme::muted(format!(
            "{} is gone",
            row_file(SHADER_TABLE, key)
        )));
        return;
    };
    let def: Option<VfxShaderDef> = serde_json::from_value(row.clone()).ok();
    let file = shader_file(key);
    let current = editor
        .project
        .as_ref()
        .and_then(|project| project.files.text(&file))
        .unwrap_or_default();
    let code_id = egui::Id::new(("shader-code", key));
    let draft = draft_or(ui, code_id, current.clone());
    let room = ui.available_width() - 2.0 * COLUMN_GAP;
    let preview_width =
        (room - FORM_WIDTH - MIN_CODE_WIDTH).clamp(MIN_PREVIEW_WIDTH, preview::PREVIEW[0] as f32);
    let code_width = (room - FORM_WIDTH - preview_width).max(MIN_CODE_WIDTH);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(FORM_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt(("shader-form", key))
                .auto_shrink([false, false])
                .show(ui, |ui| inspector::show_row(editor, ui, SHADER_TABLE, key));
        });
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(code_width);
            match def.as_ref() {
                Some(def) => {
                    code_pane(
                        editor,
                        ui,
                        code_id,
                        def,
                        key,
                        &file,
                        &current,
                        draft.clone(),
                    );
                }
                None => {
                    widgets::section(ui, &format!("code · {file}"));
                    ui.label(theme::warn("the row does not parse as a shader yet"));
                }
            }
        });
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(preview_width);
            widgets::section(ui, "preview");
            let Some(def) = def.as_ref() else {
                return;
            };
            let layer = Layer::Shader {
                shader: key.to_owned(),
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
            let drafts = [(key.to_owned(), draft)];
            preview(
                editor,
                ui,
                &format!("{SHADER_TABLE}/{key}"),
                vec![layer],
                &specs,
                &drafts,
            );
        });
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "one shader's code as it stands, read once"
)]
fn code_pane(
    editor: &mut Editor,
    ui: &mut Ui,
    id: egui::Id,
    def: &VfxShaderDef,
    key: &str,
    file: &str,
    current: &str,
    mut draft: String,
) {
    let heading_slot = ui.available_rect_before_wrap();
    ui.add_space(HEADING_HEIGHT);
    let height = ui.available_height() - CODE_FOOTER;
    let (response, verdict) = shader_code::show(ui, id, &mut draft, def, height);
    if response.changed() {
        set_draft(ui, id, &draft);
    }
    let status = match &verdict {
        Ok(()) => theme::ok("✓ compiles"),
        Err(error) => {
            let place = error
                .line
                .map(|(line, column)| format!("line {line}:{column} · "))
                .unwrap_or_default();
            theme::warn(format!("✗ {place}{}", error.message))
        }
    };
    let heading = egui::Rect::from_min_size(
        heading_slot.min,
        EVec2::new(heading_slot.width(), HEADING_HEIGHT),
    );
    ui.scope_builder(
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
            ui.scope(|ui| {
                ui.set_max_width(title_room.max(MIN_TITLE_WIDTH));
                ui.add(
                    egui::Label::new(widgets::section_heading(&format!("code · {file} · ")))
                        .truncate(),
                );
            });
            ui.add(egui::Label::new(status).truncate());
        },
    );
    let dirty = draft != current;
    if !dirty {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let apply = widgets::primary(ui, "Apply").clicked();
        if ui.button("Revert").clicked() {
            clear_draft(ui, id);
        }
        if apply || (response.lost_focus() && verdict.is_ok()) {
            let mut edit = ContentEdit::new(format!("Edit shader {key}")).merging();
            edit.write(file.to_owned(), Arc::from(draft.as_bytes()));
            if editor.apply(edit) {
                clear_draft(ui, id);
            }
        }
    });
}
