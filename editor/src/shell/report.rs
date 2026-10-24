use ui::controls;
use ui::dialog;
use ui::egui;
use ui::egui::{Context, RichText, Ui};
use ui::glyph::{self, Mark};
use ui::panel;
use ui::theme;

use crate::app::Editor;

pub enum ReportAction {
    None,
    LoadAgain,
    Close,
}

pub fn panel(ui: &mut Ui, folder: &str, text: &str, closable: bool) -> ReportAction {
    let mut action = ReportAction::None;
    panel::well().show(ui, |ui| {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.label(
                    theme::muted(format!("load report · {folder}"))
                        .monospace()
                        .size(12.5),
                );
            });
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 60.0)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for line in text.lines() {
                            ui.label(colored(line));
                        }
                    });
            });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if controls::primary(ui, "Copy report").clicked() {
            ui.ctx().copy_text(text.to_owned());
        }
        if ui.button("Save to file…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Save the load report")
                .set_file_name("load-report.txt")
                .save_file()
        {
            let _ = std::fs::write(path, text);
        }
        if controls::marked(ui, Mark::Reload, "Load again").clicked() {
            action = ReportAction::LoadAgain;
        }
        if closable && ui.button("Close").clicked() {
            action = ReportAction::Close;
        }
    });
    action
}

pub fn not_loaded(editor: &mut Editor, ctx: &Context) {
    let Some(project) = &editor.project else {
        return;
    };
    let Some(report) = project.report.as_ref().filter(|_| project.broken_on_disk()) else {
        return;
    };
    let text = report.to_string();
    let folder = super::pretty_path(&project.root);
    let rewrote = project.outside_rewrote;
    let unsaved = project.unsaved().len();
    let screen = ctx.content_rect();
    let mut action = ReportAction::None;
    egui::Area::new(egui::Id::new("not-loaded"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.allocate_rect(screen, egui::Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(170));
            let width = screen.width().min(960.0);
            let left = screen.center().x - width / 2.0;
            let mut column = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        egui::pos2(left, screen.min.y + 90.0),
                        egui::vec2(width, screen.height() - 180.0),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            dialog::note(&mut column, width - 2.0 * f32::from(dialog::PADDING), |ui| {
                    ui.horizontal(|ui| {
                        glyph::show(ui, Mark::Warning, 16.0, theme::DANGER);
                        dialog::title(ui, "The folder no longer loads");
                    });
                    ui.add_space(4.0);
                    ui.label(theme::muted(format!(
                        "{rewrote} files changed on disk. The editor loaded the folder again and it was refused. Nothing was written."
                    )));
                    ui.add_space(12.0);
                    action = panel(ui, &folder, &text, false);
                    ui.add_space(8.0);
                    ui.label(theme::muted(match unsaved {
                        0 => "Nothing is unsaved here.".to_owned(),
                        1 => "Your unsaved edit is kept and comes back when the folder loads.".to_owned(),
                        n => format!("Your {n} unsaved edits are kept and come back when the folder loads."),
                    }));
                });
        });
    if matches!(action, ReportAction::LoadAgain)
        && let Some(project) = &mut editor.project
    {
        project.reload();
    }
}

/// Whether the line starts with the path of a file or a folder, as the lines naming a problem's
/// place do.
fn names_a_file(line: &str) -> bool {
    !line.starts_with(' ')
        && line
            .split_whitespace()
            .next()
            .is_some_and(|word| word.contains('/') || word.contains('.'))
}

fn colored(line: &str) -> RichText {
    let text = RichText::new(line).monospace().size(13.0);
    if line.starts_with('#') {
        text.color(theme::WARN)
    } else if line.starts_with("result") {
        text.color(theme::DANGER)
    } else if names_a_file(line) {
        text.color(theme::CODE)
    } else if line.starts_with("  ") {
        text.color(theme::TEXT)
    } else {
        text.color(theme::MUTED)
    }
}
