use engine::core::content::{ContentFile, table_of};
use engine::systems::area::AreaDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::terrain::{self, TerrainSize};
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::egui::RichText;
use ui::form;
use ui::theme;

use super::nodes::{self, SceneBundle};
use crate::app::Editor;
use crate::tabs::file_title;
use ui::input::InputField;

/// Resizes an area or a blueprint from its top-left corner: the ground reaches further, or is cut,
/// at the right and the bottom, so nothing on it moves.
pub struct ResizeDialog {
    pub bundle: String,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

impl ResizeDialog {
    pub fn for_bundle(bundle: &str, size: TerrainSize) -> ResizeDialog {
        ResizeDialog {
            bundle: bundle.to_owned(),
            width: size.width,
            height: size.height,
            error: None,
        }
    }
}

pub fn resize_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: ResizeDialog,
) -> Option<ResizeDialog> {
    let mut outcome = None;
    let old = nodes::bundle_of(editor, &dialog.bundle).map(|bundle| bundle.size());
    let smallest = match table_of(&dialog.bundle) {
        Some(AreaDef::TABLE) => terrain::MIN_SIDE,
        _ => 1,
    };
    dialog::card(ctx, "resize", 420.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(
            RichText::new(format!("Resize {}", file_title(&dialog.bundle)))
                .strong()
                .size(18.0),
        );
        ui.add_space(8.0);
        for (name, value, was) in [
            ("width", &mut dialog.width, old.map(|size| size.width)),
            ("height", &mut dialog.height, old.map(|size| size.height)),
        ] {
            form::kv(ui, name, |ui| {
                let id = ui.id().with((name, "resize"));
                let mut draft = ui
                    .ctx()
                    .data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| value.to_string());
                let was = was.map_or("m".to_owned(), |was| format!("m · was {was}"));
                let response = InputField::new(&mut draft).unit(&was).width(120.0).show(ui);
                if response.changed() {
                    ui.ctx().data_mut(|d| d.insert_temp(id, draft.clone()));
                    if let Ok(parsed) = draft.trim().parse::<u32>() {
                        *value = parsed.clamp(smallest, terrain::MAX_SIDE);
                    }
                }
                if response.lost_focus() {
                    ui.ctx().data_mut(|d| d.remove_temp::<String>(id));
                }
            });
        }
        ui.add_space(6.0);
        ui.label(
            theme::muted(
                "The ground grows or is cut at the right and the bottom; nothing on it moves.",
            )
            .size(13.0),
        );
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if controls::primary(ui, "Resize").clicked() || pressed.confirm {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() || pressed.cancel {
                outcome = Some(false);
            }
        });
    });
    match outcome {
        Some(true) => match resize(editor, &dialog) {
            Ok(()) => None,
            Err(error) => {
                dialog.error = Some(error);
                Some(dialog)
            }
        },
        Some(false) => None,
        None => Some(dialog),
    }
}

fn resize(editor: &mut Editor, dialog: &ResizeDialog) -> Result<(), String> {
    let size = TerrainSize {
        width: dialog.width,
        height: dialog.height,
    };
    let bundle = nodes::bundle_of(editor, &dialog.bundle)
        .ok_or_else(|| format!("{} doesn't read", file_title(&dialog.bundle)))?;
    let label = format!(
        "Resize {} to {} × {}",
        file_title(&dialog.bundle),
        size.width,
        size.height
    );
    let written = match bundle {
        SceneBundle::Area(area) => {
            editor.write_content(&label, &dialog.bundle, &area.resized(size))
        }
        SceneBundle::Blueprint(blueprint) => {
            let mut blueprint = (*blueprint).clone();
            blueprint.size = size;
            editor.write_content(&label, &dialog.bundle, &blueprint)
        }
    };
    if written {
        Ok(())
    } else {
        Err(editor
            .last_refusal
            .as_ref()
            .map(|refusal| refusal.sentence())
            .unwrap_or_else(|| "the resize was refused".to_owned()))
    }
}

/// The table a dialog resizes, for the step that names it.
pub fn table_resized(dialog: &ResizeDialog) -> &'static str {
    match table_of(&dialog.bundle) {
        Some(BlueprintDef::TABLE) => BlueprintDef::TABLE,
        _ => AreaDef::TABLE,
    }
}
