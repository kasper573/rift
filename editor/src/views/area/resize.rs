use engine::core::content::{ContentFile, table_of};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::GridSize;
use engine::systems::area::{self, AreaDef, AreaModule, MapSize};
use engine::systems::script::Place;
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::egui::{RichText, Vec2};
use ui::form;
use ui::theme;
use ui::words;

use super::grid::GridDraft;
use super::{nodes, pins, shapes};
use crate::app::Editor;
use crate::tabs::file_title;
use ui::input::InputField;

pub struct ResizeDialog {
    /// The area or blueprint bundle resized.
    pub bundle: String,
    pub width: u32,
    pub height: u32,
    pub anchor: (i8, i8),
    pub error: Option<String>,
}

pub fn resize_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: ResizeDialog,
) -> Option<ResizeDialog> {
    let mut outcome = None;
    let old = GridDraft::own(editor, &dialog.bundle)
        .map(|draft| (draft.size.width, draft.size.height))
        .unwrap_or((dialog.width, dialog.height));
    let smallest = match table_of(&dialog.bundle) {
        Some(AreaDef::TABLE) => area::MIN_SIDE,
        _ => 1,
    };
    dialog::card(ctx, "resize", 480.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(
            RichText::new(format!("Resize {}", file_title(&dialog.bundle)))
                .strong()
                .size(18.0),
        );
        ui.add_space(8.0);
        for (name, value, was) in [
            ("width", &mut dialog.width, old.0),
            ("height", &mut dialog.height, old.1),
        ] {
            form::kv(ui, name, |ui| {
                ui.scope(|ui| {
                    let id = ui.id().with((name, "resize"));
                    let mut draft = ui
                        .ctx()
                        .data(|d| d.get_temp::<String>(id))
                        .unwrap_or_else(|| value.to_string());
                    let was = format!("was {was}");
                    let response = InputField::new(&mut draft).unit(&was).width(120.0).show(ui);
                    if response.changed() {
                        ui.ctx().data_mut(|d| d.insert_temp(id, draft.clone()));
                        if let Ok(parsed) = draft.trim().parse::<u32>() {
                            *value = parsed.clamp(smallest, area::MAX_SIDE);
                        }
                    }
                    if response.lost_focus() {
                        ui.ctx().data_mut(|d| d.remove_temp::<String>(id));
                    }
                });
            });
        }
        form::kv(ui, "anchor", |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                for row in [-1i8, 0, 1] {
                    ui.horizontal(|ui| {
                        for column in [-1i8, 0, 1] {
                            let picked = dialog.anchor == (column, row);
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                if picked { theme::ACCENT } else { theme::INPUT },
                            );
                            ui.painter().rect_stroke(
                                rect,
                                3.0,
                                egui::Stroke::new(
                                    1.0,
                                    if picked { theme::ACCENT } else { theme::BORDER },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::RadioButton,
                                    true,
                                    picked,
                                    anchor_name(column, row),
                                )
                            });
                            if response.clicked() {
                                dialog.anchor = (column, row);
                            }
                        }
                    });
                }
            });
        });
        ui.add_space(6.0);
        ui.label(
            theme::muted(resize_description(
                old,
                (dialog.width, dialog.height),
                dialog.anchor,
            ))
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
        Some(true) => match resize_area(editor, &dialog) {
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

pub(super) fn resize_description(old: (u32, u32), new: (u32, u32), anchor: (i8, i8)) -> String {
    let mut parts = Vec::new();
    let side = |delta: i32, axis_anchor: i8, unit: &str, sides: (&str, &str)| -> Option<String> {
        if delta == 0 {
            return None;
        }
        let n = delta.unsigned_abs();
        let where_ = match axis_anchor {
            -1 => format!("on the {}", sides.1),
            1 => format!("on the {}", sides.0),
            _ => "on both sides".to_owned(),
        };
        Some(if delta > 0 {
            format!("Adds {n} empty {unit}{} {where_}", words::plural(n))
        } else {
            format!("Clips {n} {unit}{} {where_}", words::plural(n))
        })
    };
    if let Some(text) = side(
        new.0 as i32 - old.0 as i32,
        anchor.0,
        "column",
        ("left", "right"),
    ) {
        parts.push(text);
    }
    if let Some(text) = side(
        new.1 as i32 - old.1 as i32,
        anchor.1,
        "row",
        ("top", "bottom"),
    ) {
        parts.push(text);
    }
    if parts.is_empty() {
        return "The size stays as it is.".to_owned();
    }
    let (dx, dy) = (
        anchor_shift(anchor.0, old.0, new.0),
        anchor_shift(anchor.1, old.1, new.1),
    );
    let shifting = if dx == 0 && dy == 0 {
        "With this anchor nothing shifts.".to_owned()
    } else {
        format!(
            "With this anchor everything shifts by {dx}, {dy}: positions, and every place leading in."
        )
    };
    format!(
        "{} of every ground layer and the walk grid. {shifting}",
        parts.join(" and ")
    )
}

fn anchor_shift(anchor: i8, old: u32, new: u32) -> i32 {
    let delta = new as i32 - old as i32;
    match anchor {
        -1 => 0,
        0 => delta / 2,
        _ => delta,
    }
}

pub fn resize_area(editor: &mut Editor, dialog: &ResizeDialog) -> Result<(), String> {
    let bundle = &dialog.bundle;
    let draft = GridDraft::own(editor, bundle).ok_or("the bundle is gone")?;
    let old = draft.size;
    let new = GridSize::new(dialog.width, dialog.height);
    let shift = (
        anchor_shift(dialog.anchor.0, old.width, new.width),
        anchor_shift(dialog.anchor.1, old.height, new.height),
    );
    let (resized, clipped) = draft.resized(new, shift);
    let (dx, dy) = (shift.0 as f32, shift.1 as f32);
    let bounds = Rect::new(
        Pos::new(-0.5, -0.5),
        Size::new(dialog.width as f32, dialog.height as f32),
    );
    let in_area = table_of(bundle) == Some(AreaDef::TABLE);
    let reader = editor.reader().ok_or("no folder is open")?;
    let mut outside = Vec::new();
    let edit = nodes::blueprint_edit(editor, bundle, |blueprint| {
        blueprint.map.size = MapSize {
            width: dialog.width,
            height: dialog.height,
        };
        for module in nodes::modules(&blueprint.scene.nodes) {
            let node = nodes::module_node_mut(&mut blueprint.scene.nodes, &module.pointer)?;
            let module = &mut node.module;
            shapes::shift(module, dx, dy);
            let keeps_out = matches!(
                module,
                AreaModule::SafeZone(_) | AreaModule::Soundscape(_) | AreaModule::Population(_)
            );
            if let Some(b) = shapes::bounds(module, None)
                && in_area
                && !keeps_out
                && !bounds.contains_rect(&b)
            {
                outside.push(shapes::label(module, reader.as_ref()));
            }
        }
        resized.apply_to(blueprint);
        Some(format!(
            "Resize {} to {} × {} ({clipped} cells clipped)",
            file_title(bundle),
            dialog.width,
            dialog.height
        ))
    });
    if !outside.is_empty() {
        return Err(format!(
            "{} left outside the map: {}",
            outside.len(),
            outside.join(", ")
        ));
    }
    let mut edit = edit.ok_or("the bundle doesn't read")?;
    if dx != 0.0 || dy != 0.0 {
        let project = editor.project.as_ref().ok_or("no folder is open")?;
        for arrival in pins::arrivals(editor, bundle) {
            edit.update_at(
                project,
                &arrival.file,
                &arrival.pointer,
                |place: &mut Place| {
                    place.at = Pos::new(arrival.at.x + dx, arrival.at.y + dy);
                },
            )?;
        }
    }
    editor.try_apply(edit)
}

/// The anchor `(column, row)`, -1 to 1 each, as a compass reads it out: "top left", "center".
fn anchor_name(column: i8, row: i8) -> String {
    let side = match column {
        -1 => "left",
        1 => "right",
        _ => "",
    };
    let edge = match row {
        -1 => "top",
        1 => "bottom",
        _ => "",
    };
    match (edge, side) {
        ("", "") => "center".to_owned(),
        ("", side) | (side, "") => side.to_owned(),
        (edge, side) => format!("{edge} {side}"),
    }
}
