use egui::{Color32, Sense, Ui, Vec2};
use serde_json::Value;

use super::{ModelStrip, strip::DraggedFrame, strip_of};
use crate::app::Editor;
use crate::ui::{theme, widgets};

const CELL: f32 = 76.0;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &Value) {
    let ctx = ui.ctx().clone();
    let sheet = super::sheet_info(editor, &ctx, row);
    let view = editor.views.model.view(key);
    let (strip, pick) = (view.strip, view.sheet_pick);
    let current = strip_of(row, strip);
    let used: Vec<u32> = current.iter().map(|frame| frame.frame).collect();
    let starts: Vec<(u32, ModelStrip)> = ModelStrip::all()
        .filter_map(|strip| Some((strip_of(row, strip).first()?.frame, strip)))
        .collect();
    let strips_starting_in_row = |frame: u32| -> String {
        let columns = sheet.columns.max(1);
        let line = frame / columns;
        let first = line * columns;
        let last = first + columns;
        starts
            .iter()
            .filter(|(start, _)| *start >= first && *start < last)
            .map(|(_, strip)| format!("{} {}", strip.action.as_ref(), strip.direction.as_ref()))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        widgets::chip_colored(ui, &sheet.image, theme::ACCENT_SOFT, theme::TEXT);
        if let Some(first) = current.first() {
            let line = first.frame / sheet.columns.max(1);
            widgets::chip(
                ui,
                &format!("row {} · {}", line + 1, strips_starting_in_row(first.frame)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            widgets::hint(
                ui,
                "drag frames into the timeline, or select and press Insert",
            );
        });
    });
    let mut picked = None;
    egui::ScrollArea::both()
        .id_salt(("sheet", key))
        .show(ui, |ui| {
            ui.add_space(4.0);
            for line in 0..sheet.rows {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for column in 0..sheet.columns {
                        let frame = line * sheet.columns + column;
                        let is_used = used.contains(&frame);
                        let is_picked = pick == Some(frame);
                        let response = ui
                            .dnd_drag_source(
                                ui.id().with(("sheet-frame", key, frame)),
                                DraggedFrame(frame),
                                |ui| {
                                    let (rect, response) =
                                        ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
                                    let painter = ui.painter_at(rect);
                                    painter.rect(
                                        rect,
                                        2.0,
                                        if is_used {
                                            theme::ACCENT_SOFT
                                        } else {
                                            theme::INPUT
                                        },
                                        egui::Stroke::new(
                                            if is_picked { 2.0 } else { 1.0 },
                                            if is_picked {
                                                Color32::WHITE
                                            } else {
                                                theme::BORDER
                                            },
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                    sheet.draw(&painter, rect.shrink(4.0), frame, Color32::WHITE);
                                    response
                                },
                            )
                            .inner;
                        if response.clicked() {
                            picked = Some(frame);
                        }
                        response.on_hover_text(format!("sheet frame {frame}"));
                    }
                    let label = strips_starting_in_row(line * sheet.columns);
                    if !label.is_empty() {
                        ui.add_space(8.0);
                        ui.label(theme::faint(format!("row {} · {label}", line + 1)).size(11.5));
                    }
                });
            }
        });
    if let Some(frame) = picked {
        let view = editor.views.model.view_mut(key);
        view.sheet_pick = Some(frame);
    }
}
