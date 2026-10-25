//! The palette: every prop of the game as a card, to pick one for the Place and Scatter tools.

use std::collections::BTreeMap;

use engine::core::content::parent_of;
use engine::systems::area::AreaModule;
use engine::systems::area::prop::PropDef;
use ui::egui;
use ui::egui::{Rect as ERect, Stroke as EStroke, Ui, Vec2};
use ui::input;
use ui::theme;

use super::{AreaSnapshot, AreaTool, Placeable};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;

const CARD: Vec2 = Vec2::new(88.0, 104.0);
const GAP: f32 = 8.0;

pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot) {
    let content = &snapshot.content;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for module in snapshot.modules.iter() {
        if let AreaModule::Prop(prop) = module.module()
            && let Some(path) = snapshot.reader.path_of(prop.prop)
        {
            *counts.entry(path).or_default() += 1;
        }
    }
    let bundle = snapshot.bundle.clone();
    let needle = {
        let view = editor.views.area.view_mut(view_key);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_width(220.0);
                input::search(ui, &mut view.palette_search, "Search props", None);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    theme::muted(format!(
                        "click a prop, then click the ground · {} turns it · {} strews it over a rectangle",
                        EditorKey::TurnClockwise.name(),
                        EditorKey::ScatterTool.name()
                    ))
                    .size(12.0),
                );
            });
        });
        view.palette_search.to_lowercase()
    };
    let mut listed: Vec<(usize, String, usize)> = content
        .table::<PropDef>()
        .paths()
        .iter()
        .filter(|path| needle.is_empty() || path.to_lowercase().contains(&needle))
        .map(|path| {
            let count = counts.get(path.as_str()).copied().unwrap_or(0);
            (folders_apart(&bundle, path), path.clone(), count)
        })
        .collect();
    listed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.2.cmp(&a.2))
            .then_with(|| a.1.cmp(&b.1))
    });
    ui.add_space(6.0);
    let picked_now = editor.views.area.view(view_key).prop().map(str::to_owned);
    let mut picked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt((view_key, "palette"))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(GAP);
                for (_, key, count) in &listed {
                    let selected = picked_now.as_deref() == Some(key.as_str());
                    if card(editor, ui, key, *count, selected).clicked() {
                        picked = Some(key.clone());
                    }
                }
            });
        });
    if listed.is_empty() {
        ui.label(theme::muted("No props match.").size(12.5));
    }
    if let Some(key) = picked {
        let view = editor.views.area.view_mut(view_key);
        if !matches!(view.tool, AreaTool::Place | AreaTool::Scatter) {
            view.set_tool(AreaTool::Place);
        }
        view.placeable = Some(Placeable::Prop(key));
    }
}

/// A prop's card: its thumbnail over its name, counting how many the scene places.
fn card(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    count: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CARD, egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::RAISED);
    let stroke = if selected {
        EStroke::new(2.0, theme::ACCENT)
    } else if response.hovered() {
        EStroke::new(1.0, theme::TEXT)
    } else {
        EStroke::new(1.0, theme::BORDER)
    };
    painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
    let art = ERect::from_min_size(
        rect.min + Vec2::new(6.0, 6.0),
        Vec2::new(CARD.x - 12.0, CARD.x - 12.0),
    );
    match editor.prop_thumbnails.thumbnail(key) {
        Some(texture) => {
            painter.image(texture, art, crate::thumbs::FULL_UV, egui::Color32::WHITE);
        }
        None => {
            painter.rect_filled(art, 3.0, theme::INPUT);
            painter.text(
                art.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(14.0),
                theme::FAINT,
            );
            ui.ctx().request_repaint();
        }
    }
    painter.text(
        egui::Pos2::new(rect.center().x, rect.max.y - 11.0),
        egui::Align2::CENTER_CENTER,
        file_title(key),
        egui::FontId::proportional(11.0),
        theme::TEXT,
    );
    if count > 0 {
        painter.text(
            rect.right_top() + Vec2::new(-6.0, 6.0),
            egui::Align2::RIGHT_TOP,
            format!("×{count}"),
            egui::FontId::monospace(10.5),
            theme::MUTED,
        );
    }
    response.on_hover_text(key)
}

/// How many folders up from `here` the path of `there` branches off: 0 for a file beside it.
fn folders_apart(here: &str, there: &str) -> usize {
    let folders = |path: &str| -> Vec<String> {
        parent_of(path)
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (here, there) = (folders(here), folders(there));
    let shared = here.iter().zip(&there).take_while(|(a, b)| a == b).count();
    here.len() - shared
}
