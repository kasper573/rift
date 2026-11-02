use std::time::SystemTime;
use ui::egui;
use ui::face;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::content::MODULES;
use serde_json::Value;
use ui::egui::{Align2, FontId, Sense, Ui};
use ui::glyph::Mark;

use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::FileBytes;
use crate::project::files::content_of;
use crate::project::history::EditStep;
use crate::tabs::shown_path;
use engine::core::time::UnixMillis;

const ROW: f32 = 28.0;

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    let Some(project) = &editor.project else {
        return;
    };
    let applied = project.history.applied();
    let saved_at = project.history.saved();
    let rows: Vec<(usize, String, String, bool)> = project
        .history
        .steps()
        .iter()
        .enumerate()
        .map(|(index, step)| {
            (
                index + 1,
                step.label.clone(),
                clock(step.at),
                index < applied,
            )
        })
        .collect();
    let opened = super::pretty_path(&project.root);
    let focus = editor.history_focus;
    editor.status.left.push(format!(
        "{} undo · {} redo · click a step to go back to it",
        EditorKey::Undo.name(),
        EditorKey::Redo.name()
    ));
    let mut jump = None;
    let card_width = if focus.is_some() {
        (ui.available_width() * 0.4).clamp(240.0, 420.0)
    } else {
        0.0
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - card_width);
            egui::ScrollArea::vertical()
                .id_salt("history")
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    if rows.is_empty() {
                        ui.label(theme::muted("No steps yet.").size(13.0));
                    }
                    for (number, label, time, done) in rows.iter().rev() {
                        let current = *number == applied;
                        let response = row(
                            ui,
                            label,
                            time,
                            *done,
                            current,
                            saved_at == Some(*number),
                            focus == Some(*number),
                        );
                        if response.clicked() {
                            jump = Some(*number);
                        }
                    }
                    let response = row(
                        ui,
                        &format!("Open {opened}"),
                        "",
                        true,
                        applied == 0,
                        saved_at == Some(0),
                        focus == Some(0),
                    );
                    if response.clicked() {
                        jump = Some(0);
                    }
                });
        });
        if focus.is_some() {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(card_width - 12.0);
                egui::ScrollArea::vertical()
                    .id_salt("history-step")
                    .show(ui, |ui| step_card(editor, ui));
            });
        }
    });
    if let Some(index) = jump {
        editor.history_focus = Some(index);
        if let Some(applied) = editor
            .project
            .as_mut()
            .and_then(|project| project.jump(index))
        {
            editor.return_to(applied);
        }
    }
}

fn row(
    ui: &mut Ui,
    label: &str,
    time: &str,
    done: bool,
    current: bool,
    saved: bool,
    focused: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::click());
    ui.painter().add(ui::controls::row_marks(
        rect,
        focused || response.hovered(),
        current,
    ));
    let ink = if done { theme::TEXT } else { theme::FAINT };
    (mark_of(label)).paint(
        ui.painter(),
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(20.0, 0.0),
            egui::vec2(12.0, 12.0),
        ),
        if done { theme::MUTED } else { theme::FAINT },
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(36.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        ink,
    );
    let mut right = rect.right() - 10.0;
    if !time.is_empty() {
        let drawn = ui.painter().text(
            egui::pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            time,
            FontId::monospace(12.0),
            theme::FAINT,
        );
        right = drawn.left() - 10.0;
    }
    if saved {
        let badge = ui.painter().text(
            egui::pos2(right - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "SAVED",
            FontId::monospace(10.5),
            theme::OK,
        );
        ui.painter().rect_stroke(
            badge.expand2(egui::vec2(6.0, 3.0)),
            3.0,
            egui::Stroke::new(1.0, theme::OK),
            egui::StrokeKind::Outside,
        );
    }
    let y = rect.bottom();
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    response
}

fn mark_of(label: &str) -> Mark {
    let verb = label.split_whitespace().next().unwrap_or("").to_lowercase();
    match verb.as_str() {
        "move" | "drag" | "place" | "stamp" | "resize" => Mark::Move,
        "delete" | "remove" | "erase" | "clear" => Mark::Cross,
        "paint" | "fill" => Mark::Paint,
        "set" | "edit" | "change" | "toggle" | "link" | "unlink" => Mark::Edit,
        "add" | "new" | "duplicate" | "paste" | "make" | "import" => Mark::Plus,
        "rename" => Mark::Tag,
        "open" | "recover" => Mark::File,
        _ => Mark::Dot,
    }
}

fn step_card(editor: &Editor, ui: &mut Ui) {
    let Some(project) = &editor.project else {
        return;
    };
    let Some(index) = editor.history_focus else {
        return;
    };
    sheet::section(ui, "Step");
    if index == 0 {
        ui.label(theme::strong(format!("Open {}", super::pretty_path(&project.root))).size(14.0));
        return;
    }
    let Some(step) = project.history.steps().get(index - 1) else {
        return;
    };
    ui.label(theme::strong(&step.label).size(14.0));
    sheet::section(ui, "Touched");
    for change in &step.after {
        let before = step.before.iter().find(|b| b.file == change.file);
        ui.horizontal(|ui| {
            let name = content_of(&change.file)
                .map(|(_, key)| shown_path(key).to_owned())
                .unwrap_or_else(|| change.file.clone());
            ui.label(face::mono(name).size(13.0));
            ui.label(
                theme::muted(touched(
                    before.and_then(|b| b.bytes.as_ref()),
                    change.bytes.as_ref(),
                ))
                .size(12.5),
            );
        });
    }
    sheet::section(ui, "Restores");
    ui.label(theme::muted(restores(step)).size(13.0));
}

fn touched(before: Option<&FileBytes>, after: Option<&FileBytes>) -> String {
    let parse = |bytes: Option<&FileBytes>| {
        bytes.and_then(|b| serde_json::from_slice::<Value>(&b[..]).ok())
    };
    match (before, after) {
        (None, Some(_)) => return "new file".to_owned(),
        (Some(_), None) => return "removed".to_owned(),
        _ => {}
    }
    let (Some(before), Some(after)) = (parse(before), parse(after)) else {
        return "the grid".to_owned();
    };
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        return "the row".to_owned();
    };
    let modules = |value: &serde_json::Map<String, Value>| {
        value
            .get(MODULES)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (old_modules, new_modules) = (modules(before), modules(after));
    let changed_modules = (0..old_modules.len().max(new_modules.len()))
        .filter(|i| old_modules.get(*i) != new_modules.get(*i))
        .count();
    let changed_fields = before
        .iter()
        .filter(|(k, v)| *k != MODULES && after.get(*k) != Some(v))
        .count()
        + after
            .keys()
            .filter(|k| *k != MODULES && !before.contains_key(*k))
            .count();
    let mut parts = Vec::new();
    if changed_modules > 0 {
        parts.push(format!(
            "{changed_modules} module{}",
            words::plural(changed_modules)
        ));
    }
    if changed_fields > 0 {
        parts.push(format!(
            "{changed_fields} field{}",
            words::plural(changed_fields)
        ));
    }
    if parts.is_empty() {
        "the row".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn restores(step: &EditStep) -> String {
    let mut parts = vec!["the files as they were".to_owned()];
    if !step.selection.keys.is_empty() {
        parts.push("the selection".to_owned());
    }
    if let Some(camera) = &step.camera {
        parts.push(format!("the camera on {}", camera.bundle));
    }
    words::join_and(&parts)
}

fn clock(at: SystemTime) -> String {
    ui::words::clock(UnixMillis::from(at).0, true)
}
