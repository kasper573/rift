use std::collections::BTreeMap;

use egui::{Color32, Rect as ERect, RichText, Sense, Ui, Vec2};
use engine::core::math::Direction;
use engine::systems::actor::Action;
use serde_json::Value;
use strum::VariantArray;

use super::{ModelPage, ModelStrip, StripFrame, action_timing, play_rule, shifted, strip_of};
use crate::app::Editor;
use crate::import;
use crate::ui::{theme, widgets};

const CELL: f32 = 58.0;
const LABEL_W: f32 = 120.0;
const PREVIEW: Vec2 = Vec2::new(180.0, 350.0);

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &Value) {
    let ctx = ui.ctx().clone();
    let sheet = super::sheet_info(editor, &ctx, row);
    let timings: BTreeMap<Action, super::ActionTiming> = Action::VARIANTS
        .iter()
        .map(|&action| (action, action_timing(row, action)))
        .collect();
    let strip = editor.views.model.view(key).strip;
    keys(editor, ui, key);
    let moved = editor.views.model.view(key).strip;
    if moved != strip {
        super::select_strip(editor, key, moved);
    }
    let strip = moved;
    let frames = strip_of(row, strip);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let tick = {
        let view = editor.views.model.view_mut(key);
        super::preview::keys(ui, &mut view.preview, &frames);
        super::preview::tick(&mut view.preview, dt, &frames, strip.action)
    };
    super::preview::fire(editor, key, &frames, &tick.passed);
    ui.ctx().request_repaint();
    let mut open = None;
    let mut pick = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LABEL_W + Direction::VARIANTS.len() as f32 * (CELL + 8.0));
            header(ui);
            let mut strips = 0;
            let mut frames_total = 0;
            for &action in Action::VARIANTS {
                let timing = &timings[&action];
                ui.horizontal(|ui| {
                    let first = strip_of(row, ModelStrip { action, ..ModelStrip::default() });
                    let total = super::total_ms(&first);
                    let (column, _) =
                        ui.allocate_exact_size(Vec2::new(LABEL_W, CELL + 22.0), Sense::hover());
                    let mut column = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(column)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    column.add_space(6.0);
                    column.label(theme::mono(action.as_ref()).color(theme::CODE).size(13.0));
                    column.label(theme::muted(format!("{} · {total} ms", first.len())).size(12.0));
                    column.label(theme::faint(play_rule(action)).size(11.5));
                    for &direction in Direction::VARIANTS {
                        let this = ModelStrip { action, direction };
                        let frames = strip_of(row, this);
                        strips += 1;
                        frames_total += frames.len();
                        let badged = timing.out_of_step(direction);
                        let response = cell(ui, &sheet, frames.first(), this == strip, badged);
                        if response.clicked() {
                            pick = Some(this);
                        }
                        if response.double_clicked() {
                            open = Some(this);
                        }
                        if badged {
                            response.on_hover_text(format!(
                                "{}: {} frames · {} ms · the other directions last {} ms over {} frames",
                                direction.as_ref(),
                                frames.len(),
                                super::total_ms(&frames),
                                timing.common.iter().sum::<u32>(),
                                timing.common.len()
                            ));
                        }
                    }
                });
            }
            ui.add_space(8.0);
            let out_of_step: usize = timings.values().map(super::ActionTiming::stragglers).sum();
            ui.horizontal(|ui| {
                if out_of_step == 0 {
                    ui.label(RichText::new("✓").color(theme::OK));
                    ui.label(theme::muted(format!(
                        "{strips} strips · {frames_total} frames · all match"
                    )));
                } else {
                    ui.label(RichText::new("!").color(theme::WARN).strong());
                    ui.label(theme::muted(format!(
                        "{strips} strips · {frames_total} frames · {out_of_step} out of step with their action"
                    )));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🗋 Import onto this model…").clicked() {
                        import::start_for_model(editor, key);
                    }
                });
            });
            if let Some(record) = editor.imports.record_for(super::TABLE, key).cloned() {
                ui.horizontal(|ui| {
                    let export = import::show_path(editor, &record.export);
                    if record.missing() {
                        ui.label(theme::warn("⚠"));
                        ui.label(theme::muted(format!(
                            "imported from {export}, which is missing now"
                        )));
                    } else {
                        ui.label(theme::faint(format!(
                            "imported from {export} · watched for changes"
                        )));
                    }
                });
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(PREVIEW.x);
            ui.label(
                theme::mono(strip.name())
                    .color(theme::CODE),
            );
            let frame = frames.get(tick.index).map(|frame| frame.frame);
            super::preview::draw(
                editor,
                ui,
                key,
                &sheet,
                row,
                frame,
                super::preview::PreviewFit {
                    size: PREVIEW,
                    zoom: 3.0,
                },
            );
            let view = editor.views.model.view_mut(key);
            super::preview::controls(ui, &mut view.preview, tick.index, frames.len(), false);
        });
    });
    if let Some(picked) = pick {
        let view = editor.views.model.view_mut(key);
        view.strip = picked;
        view.frames.clear();
        super::preview::restart(&mut view.preview);
        super::select_strip(editor, key, picked);
    }
    if let Some(opened) = open {
        open_strip(editor, key, opened);
    }
    let used = super::users(editor, key).len();
    editor
        .status
        .left
        .push(format!("model/{key} · {}", strip.name()));
    editor.status.left.push(
        "Space plays · , and . step frames · arrows move in the matrix · Enter opens the strip"
            .to_owned(),
    );
    editor.status.right.push(format!(
        "{used} {} use this model",
        if used == 1 { "row" } else { "rows" }
    ));
}

pub fn open_strip(editor: &mut Editor, key: &str, strip: ModelStrip) {
    let view = editor.views.model.view_mut(key);
    view.strip = strip;
    view.page = ModelPage::Timing;
    view.frames.clear();
    view.cue = None;
    super::preview::restart(&mut view.preview);
    super::select_strip(editor, key, strip);
}

fn header(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W);
        for direction in Direction::VARIANTS {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL, 18.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                direction.as_ref(),
                egui::FontId::monospace(12.5),
                theme::MUTED,
            );
        }
    });
}

fn cell(
    ui: &mut Ui,
    sheet: &super::ModelSheet,
    first: Option<&StripFrame>,
    selected: bool,
    badged: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(CELL, CELL + 22.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::INPUT);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    if let Some(first) = first {
        sheet.draw(&painter, rect.shrink(6.0), first.frame, Color32::WHITE);
    }
    if badged {
        let badge =
            ERect::from_center_size(rect.right_top() + Vec2::new(-9.0, 9.0), Vec2::splat(12.0));
        painter.circle_filled(badge.center(), 6.0, theme::WARN);
        painter.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            "!",
            egui::FontId::proportional(10.0),
            Color32::BLACK,
        );
    }
    if selected || response.hovered() {
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                2.0,
                if selected {
                    theme::ACCENT
                } else {
                    theme::MUTED
                },
            ),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn keys(editor: &mut Editor, ui: &Ui, key: &str) {
    if !super::keyboard_free(ui) {
        return;
    }
    let (left, right, up, down, enter) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
        )
    });
    let view = editor.views.model.view_mut(key);
    let was = view.strip;
    let turn = i32::from(right) - i32::from(left);
    let step = i32::from(down) - i32::from(up);
    view.strip = ModelStrip {
        action: shifted(Action::VARIANTS, was.action, step as isize),
        direction: shifted(Direction::VARIANTS, was.direction, turn as isize),
    };
    if view.strip != was {
        view.frames.clear();
        super::preview::restart(&mut view.preview);
    }
    if enter {
        let strip = view.strip;
        open_strip(editor, key, strip);
    }
}

pub fn timeline(editor: &mut Editor, ui: &mut Ui, key: &str, row: &Value) {
    let ctx = ui.ctx().clone();
    let sheet = super::sheet_info(editor, &ctx, row);
    let view = editor.views.model.view(key);
    let (strip, selected) = (view.strip, view.frames.clone());
    let frames = strip_of(row, strip);
    let total = super::total_ms(&frames).max(1);
    ui.add_space(6.0);
    let visible = (ui.clip_rect().right() - ui.cursor().left()).max(200.0);
    let width = ui.available_width().min(visible) - 16.0 - 4.0 * (frames.len() + 1) as f32;
    let px_per_ms = (width / total as f32).min(1.2);
    let mut clicked = None;
    ui.horizontal_top(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        for (index, frame) in frames.iter().enumerate() {
            let w = (frame.ms as f32 * px_per_ms).max(28.0);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.allocate_ui(Vec2::new(w, 22.0), |ui| {
                    ui.set_min_height(22.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        for cue in frame.cues() {
                            let text = match cue {
                                super::FrameCueKind::Apex => "◆ apex".to_owned(),
                                super::FrameCueKind::Sound => {
                                    format!("♫ {}", frame.sound.clone().unwrap_or_default())
                                }
                                super::FrameCueKind::Step => "● step".to_owned(),
                            };
                            widgets::chip_colored(ui, &text, theme::ACCENT_SOFT, theme::TEXT);
                        }
                    });
                });
                let is_selected = selected.contains(&index);
                let response = frame_card(ui, &sheet, frame, index, w, is_selected);
                if response.clicked() {
                    clicked = Some(index);
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(theme::faint("0 ms").monospace().size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            ui.label(theme::faint(format!("{total} ms")).monospace().size(11.5));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space((ui.available_width() / 2.0 - 120.0).max(0.0));
                ui.label(theme::faint("Enter opens the strip in the animation editor").size(12.0));
            });
        });
    });
    if let Some(index) = clicked {
        let view = editor.views.model.view_mut(key);
        view.frames = vec![index];
        super::select_frames(editor, key, strip);
    }
}

fn frame_card(
    ui: &mut Ui,
    sheet: &super::ModelSheet,
    frame: &StripFrame,
    index: usize,
    width: f32,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 100.0), Sense::click());
    super::strip::frame_card(&ui.painter_at(rect), rect, sheet, frame, index, selected);
    response
}
