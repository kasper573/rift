use ui::controls;
use ui::egui;
use ui::egui::{Align, Layout, RichText, Ui};
use ui::panel;
use ui::theme;
use ui::words;

use crate::app::Editor;
use crate::play;

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    moved_outside(editor, ui);
    crate::views::area::problems::output(editor, ui);
    ui.horizontal(|ui| {
        ui.label(theme::faint(format!("{} lines", editor.output.lines.len())).size(12.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Clear").clicked() {
                editor.output.lines.clear();
            }
            if ui.small_button("Save to file…").clicked() {
                play::save_output(editor);
            }
            if ui.small_button("Copy").clicked() {
                let text = editor.output.text();
                ui.ctx().copy_text(text);
            }
        });
    });
    let lines = &editor.output.lines;
    egui::ScrollArea::both()
        .stick_to_bottom(true)
        .show_rows(ui, ROW, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                let bad = alarming(&line.text);
                ui.horizontal(|ui| {
                    ui.set_height(ROW);
                    ui.add_sized(
                        egui::vec2(104.0, ROW),
                        egui::Label::new(
                            RichText::new(format!("{}:", line.tag))
                                .monospace()
                                .size(12.5)
                                .color(if bad { theme::DANGER } else { theme::MUTED }),
                        ),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&line.text)
                                .monospace()
                                .size(12.5)
                                .color(if bad { theme::DANGER } else { theme::TEXT }),
                        )
                        .extend()
                        .selectable(true),
                    );
                });
            }
        });
}

const ROW: f32 = 18.0;

/// Files moved on disk away from what names them, each found again by its name, and the one
/// button that points every reference there.
fn moved_outside(editor: &mut Editor, ui: &mut Ui) {
    let Some((moved, summary)) = editor.project.as_ref().and_then(|project| {
        let moved = project.moved_outside.clone();
        let summary = crate::moves::moved_outside_summary(&moved, &project.files.index());
        (!moved.is_empty()).then_some((moved, summary))
    }) else {
        return;
    };
    let mut point = false;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::warn(summary).size(13.0));
        for step in moved.iter().take(SHOWN) {
            ui.label(
                theme::mono(format!("{}  →  {}", step.from, step.to))
                    .size(12.5)
                    .color(theme::CODE),
            );
        }
        if moved.len() > SHOWN {
            ui.label(theme::muted(format!("and {} more", moved.len() - SHOWN)).size(12.5));
        }
        let label = if moved.len() == 1 {
            "Point it there"
        } else {
            "Point them there"
        };
        point = controls::primary(ui, label).clicked();
    });
    ui.add_space(4.0);
    if point {
        match crate::moves::repoint(editor, &moved) {
            Ok(count) => editor.toast(format!(
                "{count} reference{} now name where their files are; Ctrl Z takes it back",
                words::plural(count)
            )),
            Err(error) => editor.toast(error),
        }
    }
}

const SHOWN: usize = 3;

fn alarming(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("error")
        || lower.contains("panicked")
        || lower.contains("refused")
        || lower.contains("failed")
        || lower.contains("stopped:")
        || (lower.contains("exited with code")
            && !lower.ends_with("code 0)")
            && !lower.ends_with("code 0"))
}
