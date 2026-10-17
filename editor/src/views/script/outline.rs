use std::collections::{HashMap, HashSet};

use egui::{Align2, Color32, FontId, Sense, Ui};
use serde_json::Value;

use super::graph::{self, NodePaletteOpen, ScriptGraphAction};
use super::nodes::{self, LineSpeaker, OutlineRow};
use super::{ScriptDoc, select_nodes};
use crate::app::{Editor, RightTab};
use crate::project::{RowSelection, catalogue_of};
use crate::spans;
use crate::ui::{theme, widgets};
use crate::value::RowValue;

const ROW: f32 = 22.0;
const INDENT: f32 = 26.0;

pub fn show(
    editor: &mut Editor,
    ui: &mut Ui,
    doc: &ScriptDoc,
    actions: &mut Vec<ScriptGraphAction>,
) {
    let owner = doc.owner.clone();
    let rows: Vec<OutlineRow> = nodes::outline(doc.nodes(), catalogue_of(editor.project.as_ref()));
    let summaries = summaries(editor, doc);
    let (focus, folded) = editor
        .views
        .script
        .view(&owner)
        .map(|view| (view.outline_focus.clone(), view.folded.clone()))
        .unwrap_or_default();
    let keys = editor.views.script.keys;
    let lines: Vec<Line> = lines(&rows, &folded);
    let focus_at = focus
        .as_ref()
        .and_then(|key| lines.iter().position(|line| line.key == *key));
    let mut next_focus = focus.clone();
    if keys.up || keys.down {
        let moved = match (focus_at, keys.up) {
            (Some(at), true) => at.saturating_sub(1),
            (Some(at), false) => (at + 1).min(lines.len().saturating_sub(1)),
            (None, _) => 0,
        };
        next_focus = lines.get(moved).map(|line| line.key.clone());
    }
    let mut fold: Option<(String, bool)> = None;
    if let Some(key) = &next_focus {
        if keys.left {
            fold = Some((key.clone(), true));
        }
        if keys.right {
            fold = Some((key.clone(), false));
        }
    }
    let mut clicked: Option<String> = None;
    let mut unreachable_shown = false;
    egui::ScrollArea::vertical()
        .id_salt(("outline", owner.slug()))
        .show(ui, |ui| {
            ui.add_space(8.0);
            for line in &lines {
                let row = &rows[line.row];
                if !row.reachable && !unreachable_shown {
                    unreachable_shown = true;
                    ui.add_space(8.0);
                    ui.label(
                        theme::faint("UNREACHABLE · no arrow leads here")
                            .size(11.0)
                            .monospace(),
                    );
                }
                let selected = next_focus.as_deref() == Some(line.key.as_str());
                let response = draw(ui, doc, &rows, line, &summaries, &folded, selected);
                if response.clicked() {
                    clicked = Some(line.key.clone());
                }
                if selected && (keys.up || keys.down) {
                    response.scroll_to_me(None);
                }
            }
            ui.add_space(12.0);
        });
    if let Some(key) = clicked {
        next_focus = Some(key);
    }
    let view = editor
        .views
        .script
        .view_mut(&owner, &editor.user, editor.project.as_ref());
    if let Some((key, fold)) = fold {
        if fold {
            view.folded.insert(key);
        } else {
            view.folded.remove(&key);
        }
    }
    view.outline_focus = next_focus.clone();
    let Some(line) = next_focus
        .as_ref()
        .and_then(|key| lines.iter().find(|line| line.key == *key))
    else {
        return;
    };
    let row = &rows[line.row];
    let target = line.target.map(|index| rows[index].name.clone());
    if keys.up || keys.down || focus != next_focus {
        select(editor, doc, row);
    }
    if keys.enter {
        match &target {
            Some(target) => select_nodes(editor, doc, std::slice::from_ref(target)),
            None => select(editor, doc, row),
        }
        editor.panels.right = RightTab::Inspector;
    }
    if keys.tab {
        let link = match row.choice {
            Some(index) => target
                .is_none()
                .then(|| (row.name.clone(), format!("/choices/{index}/to"))),
            None => super::free_port_link(doc, catalogue_of(editor.project.as_ref()), &row.name),
        };
        actions.push(ScriptGraphAction::Palette(NodePaletteOpen {
            at: ui.max_rect().left_top() + egui::vec2(60.0 + row.depth as f32 * INDENT, 40.0),
            placed: false,
            link,
        }));
    }
    if keys.link {
        super::link_by_name(editor, &owner, row.name.clone());
    }
    if keys.delete {
        match row.choice {
            Some(index) => {
                let mut script = doc.script.clone();
                if let Some(choices) = script["nodes"][&row.name]["choices"].as_array_mut()
                    && index < choices.len()
                {
                    choices.remove(index);
                    let label = format!("Remove a choice in {}", doc.owner.title());
                    super::write(editor, doc, script, label);
                }
            }
            None => actions.push(ScriptGraphAction::Delete(row.name.clone())),
        }
    }
}

struct Line {
    row: usize,
    key: String,
    target: Option<usize>,
    hidden: usize,
}

fn key_of(row: &OutlineRow, index: usize) -> String {
    match (row.choice, row.link) {
        (Some(choice), _) => format!("{}#{choice}", row.name),
        (None, true) => format!("link:{index}"),
        (None, false) => row.name.clone(),
    }
}

fn lines(rows: &[OutlineRow], folded: &HashSet<String>) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut hide_below: Option<usize> = None;
    let mut index = 0;
    while index < rows.len() {
        let row = &rows[index];
        if let Some(depth) = hide_below {
            if row.depth > depth {
                if let Some(last) = lines.last_mut() {
                    last.hidden += 1;
                }
                index += 1;
                continue;
            }
            hide_below = None;
        }
        let key = key_of(row, index);
        let mut target = None;
        if row.choice.is_some()
            && let Some(next) = rows.get(index + 1)
            && next.choice.is_none()
            && next.depth == row.depth + 1
        {
            target = Some(index + 1);
        }
        if folded.contains(&key) {
            hide_below = Some(row.depth);
        }
        lines.push(Line {
            row: index,
            key,
            target,
            hidden: 0,
        });
        index += if target.is_some() { 2 } else { 1 };
    }
    lines
}

fn draw(
    ui: &mut Ui,
    doc: &ScriptDoc,
    rows: &[OutlineRow],
    line: &Line,
    summaries: &HashMap<String, String>,
    folded: &HashSet<String>,
    selected: bool,
) -> egui::Response {
    let row = &rows[line.row];
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, theme::ACCENT_SOFT);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, ROW)),
            0.0,
            theme::ACCENT,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::RAISED);
    }
    let mono = FontId::monospace(13.0);
    let small = FontId::monospace(12.0);
    let mut x = rect.left() + 14.0 + row.depth as f32 * INDENT;
    let y = rect.center().y;
    let mut put = |ui: &Ui, text: &str, font: &FontId, ink: Color32| {
        let drawn = ui.painter().text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            ink,
        );
        x = drawn.right();
    };
    if let Some(via) = row.via.as_deref() {
        put(ui, via, &mono, theme::CODE);
        put(ui, " → ", &mono, theme::FAINT);
    }
    let target = match row.choice {
        Some(index) => {
            let label = doc
                .nodes()
                .get(&row.name)
                .map(|node| spans::plain(&node["choices"][index]["label"]))
                .unwrap_or_default();
            put(ui, &widgets::clip(&label, 60), &mono, theme::TEXT);
            put(ui, " → ", &mono, theme::FAINT);
            match line.target {
                Some(target) => Some(&rows[target]),
                None => {
                    put(ui, "end", &small, theme::FAINT);
                    None
                }
            }
        }
        None => Some(row),
    };
    if let Some(target) = target {
        if target.link {
            put(ui, "↑ back to ", &mono, theme::FAINT);
            put(ui, &target.name, &mono, theme::CODE);
        } else if let Some(node) = doc.nodes().get(&target.name) {
            let (glyph, color) = graph::kind_glyph(node);
            put(ui, glyph, &FontId::proportional(12.0), color);
            put(ui, " ", &mono, theme::TEXT);
            put(
                ui,
                &graph::card_title(&target.name, node),
                &mono,
                theme::TEXT,
            );
            if let Some(summary) = summaries
                .get(&target.name)
                .filter(|summary| !summary.is_empty())
            {
                put(ui, " · ", &mono, theme::FAINT);
                put(ui, summary, &small, theme::MUTED);
            }
        } else {
            put(ui, &target.name, &mono, theme::DANGER);
        }
    }
    if folded.contains(&line.key) {
        put(
            ui,
            &format!("  ▸ {} more", line.hidden),
            &small,
            theme::FAINT,
        );
    }
    response
}

fn summaries(editor: &Editor, doc: &ScriptDoc) -> HashMap<String, String> {
    let catalogue = catalogue_of(editor.project.as_ref());
    doc.nodes()
        .iter()
        .map(|(name, node)| {
            let summary = if nodes::is_dialogue(node) {
                dialogue_summary(editor, node)
            } else {
                nodes::summary(node, catalogue)
            };
            (name.clone(), widgets::clip(&summary, 70))
        })
        .collect()
}

fn dialogue_summary(editor: &Editor, node: &Value) -> String {
    let lines = node["lines"].as_array().cloned().unwrap_or_default();
    let speaker = lines
        .first()
        .map(|line| match LineSpeaker::of(&line["by"]) {
            Some(LineSpeaker::Npc { npc, face }) => {
                let name = editor
                    .row_value("npc", &npc)
                    .map(|row| row.text_or("name", &npc).to_owned())
                    .unwrap_or(npc);
                match face {
                    Some(face) => format!("{name}, {face}"),
                    None => name,
                }
            }
            Some(LineSpeaker::Object) => "the object".to_owned(),
            _ => "narrator".to_owned(),
        });
    let count = format!("{} line{}", lines.len(), widgets::plural(lines.len()));
    match speaker {
        Some(speaker) => format!("{speaker} · {count}"),
        None => count,
    }
}

fn select(editor: &mut Editor, doc: &ScriptDoc, row: &OutlineRow) {
    match row.choice {
        Some(index) => {
            editor.views.script.focus_on(&doc.owner);
            editor.select(RowSelection::parts(
                doc.owner.table(),
                doc.owner.key(),
                vec![
                    format!("node:{}", row.name),
                    format!("choice:{}#{index}", row.name),
                ],
            ));
            editor
                .views
                .script
                .view_mut(&doc.owner, &editor.user, editor.project.as_ref())
                .selected = vec![row.name.clone()];
        }
        None if !row.link => select_nodes(editor, doc, std::slice::from_ref(&row.name)),
        None => {}
    }
}
