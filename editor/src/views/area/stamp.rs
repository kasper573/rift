use std::collections::{BTreeMap, HashSet};
use ui::egui;
use ui::ink;
use ui::input;
use ui::theme;

use engine::core::content::{Content, ContentScope, FolderReader, parent_of, rescoped};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Size};
use engine::core::script::VariableType;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::blueprint::{BlueprintDef, BlueprintId, Placement};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaModule};
use engine::systems::script::VarValue;
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Ui, Vec2};

use super::{AreaFrame, AreaSnapshot, modules, select, shapes};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::rows;

/// A placement of the blueprint `blueprint`, an id among the paths it was read against.
pub fn placement_node(
    blueprint: BlueprintId,
    at: Pos<Meters>,
    yaw: Degrees,
    values: BTreeMap<String, VarValue>,
) -> SceneNode {
    SceneNode::Module(ModuleNode::new(AreaModule::Blueprint(Placement {
        blueprint,
        at,
        yaw,
        values,
    })))
}

pub fn stamp_panel(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot) {
    let content = &snapshot.content;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for module in snapshot.modules.iter() {
        if let AreaModule::Blueprint(placement) = module.module()
            && let Some(path) = snapshot.reader.path_of(placement.blueprint)
        {
            *counts.entry(path).or_default() += 1;
        }
    }
    let area_key = snapshot.bundle.clone();
    let needle = {
        let view = editor.views.area.view_mut(view_key);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_width(220.0);
                input::search(ui, &mut view.blueprint_search, "Search blueprints", None);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    theme::muted(format!(
                        "one blueprint at a time · {} makes one from the selection",
                        EditorKey::MakeBlueprint.name()
                    ))
                    .size(12.0),
                );
            });
        });
        view.blueprint_search.to_lowercase()
    };
    let mut listed: Vec<(usize, String, usize)> = content
        .table::<BlueprintDef>()
        .paths()
        .iter()
        .filter(|path| needle.is_empty() || path.to_lowercase().contains(&needle))
        .map(|path| {
            let count = counts.get(path.as_str()).copied().unwrap_or(0);
            (folders_apart(&area_key, path), path.clone(), count)
        })
        .collect();
    listed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.2.cmp(&a.2))
            .then_with(|| a.1.cmp(&b.1))
    });
    let listed: Vec<(String, usize)> = listed
        .into_iter()
        .map(|(_, path, count)| (path, count))
        .collect();
    ui.add_space(6.0);
    let card = Vec2::new(72.0, 80.0);
    let gap = 8.0;
    let fit = ((ui.available_width() + gap) / (card.x + gap))
        .floor()
        .max(1.0) as usize;
    let shown = if listed.len() > fit {
        fit - 1
    } else {
        listed.len()
    };
    let mut picked: Option<String> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (key, count) in listed.iter().take(shown) {
            let selected = editor.views.area.view(view_key).stamp() == Some(key);
            let refused = editor
                .project
                .as_ref()
                .and_then(|project| super::placement::holds_itself(project, &area_key, key));
            let response = blueprint_card(ui, content, key, *count, selected, card);
            match refused {
                Some(reason) => {
                    ui.painter()
                        .rect_filled(response.rect, 4.0, ink::wash(theme::PANEL, 0.71));
                    response.on_hover_text(format!("{key}: {reason}"));
                }
                None => {
                    if response.on_hover_text(key).clicked() {
                        picked = Some(key.clone());
                    }
                }
            }
        }
        if listed.len() > shown {
            let (rect, _) = ui.allocate_exact_size(card, egui::Sense::hover());
            ui.painter().rect_stroke(
                rect,
                4.0,
                EStroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            let text = format!("{} more", listed.len() - shown);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.0),
                theme::MUTED,
            );
        }
    });
    if let Some(key) = picked {
        let view = editor.views.area.view_mut(view_key);
        view.set_tool(super::AreaTool::Place);
        view.placeable = Some(super::Placeable::Blueprint(key));
    }
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

/// The values the stamp `stamp` places with, as the user last set them, read among the files as
/// they stand now; one that no longer reads is left to its default.
pub(super) fn stamp_values(editor: &Editor, stamp: Option<&str>) -> BTreeMap<String, VarValue> {
    let (Some(stamp), Some(reader)) = (stamp, editor.reader()) else {
        return BTreeMap::new();
    };
    editor
        .settings
        .stamps
        .get(stamp)
        .map(|settings| {
            settings
                .values
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), reader.from_json(value).ok()?)))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn blueprint_card(
    ui: &mut Ui,
    content: &Content,
    key: &str,
    count: usize,
    selected: bool,
    card: Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(card, egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::RAISED);
    let stroke = if selected {
        EStroke::new(2.0, theme::ACCENT)
    } else {
        EStroke::new(1.0, theme::BORDER)
    };
    painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
    let art = ERect::from_min_size(
        rect.min + Vec2::new(6.0, 6.0),
        Vec2::new(card.x - 12.0, card.x - 12.0),
    );
    draw_blueprint_art(ui, content, key, art);
    if count > 0 {
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - 10.0),
            egui::Align2::CENTER_CENTER,
            format!("×{count}"),
            egui::FontId::monospace(11.0),
            theme::MUTED,
        );
    }
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, key));
    response.on_hover_text(key)
}

/// The blueprint's modules as outlines, fitted into `into`: what a placement of it would stand on
/// the ground.
pub fn draw_blueprint_art(ui: &mut Ui, content: &Content, key: &str, into: ERect) {
    let Some(id) = content.by_path::<BlueprintDef>(key) else {
        return;
    };
    let def = id.get(content);
    let mut errors = Vec::new();
    let flattening = area::blueprint::flatten_blueprint(content, id, &|_| true, &mut errors);
    let modules: Vec<(Rect<Meters>, Color32)> = flattening
        .modules
        .iter()
        .filter_map(|placed| {
            let bounds = shapes::bounds(&placed.module, None)?;
            Some((
                bounds,
                super::overlays::module_color(Some(placed.module.kind())),
            ))
        })
        .collect();
    let painter = ui.painter_at(into);
    if modules.is_empty() {
        painter.text(
            into.center(),
            egui::Align2::CENTER_CENTER,
            "empty",
            egui::FontId::proportional(11.0),
            theme::FAINT,
        );
        return;
    }
    let canvas_bounds = def.size.bounds();
    let span = canvas_bounds
        .size
        .width
        .max(canvas_bounds.size.height)
        .max(1.0);
    let scale = (into.width() / span).min(into.height() / span);
    let origin = into.center()
        - Vec2::new(
            canvas_bounds.size.width * scale / 2.0,
            canvas_bounds.size.height * scale / 2.0,
        );
    let to_screen = |at: Pos<Meters>| {
        origin
            + Vec2::new(
                (at.x - canvas_bounds.min().x) * scale,
                (at.y - canvas_bounds.min().y) * scale,
            )
    };
    for (bounds, color) in modules {
        let rect = ERect::from_min_max(to_screen(bounds.min()), to_screen(bounds.max()));
        painter.rect_stroke(
            rect.expand(0.5),
            0.0,
            EStroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }
}

/// Turns the stamp a quarter turn, clockwise or counter.
pub fn turn_stamp(editor: &mut Editor, view_key: &str, clockwise: bool) {
    let Some(stamp) = editor.views.area.view(view_key).stamp().map(str::to_owned) else {
        return;
    };
    let settings = editor.settings.stamps.entry(stamp).or_default();
    settings.quarter_turns = (settings.quarter_turns + if clockwise { 1 } else { 3 }) % 4;
    editor.save_settings();
}

pub(super) fn stamp_yaw(editor: &Editor, stamp: &str) -> Degrees {
    let quarters = editor
        .settings
        .stamps
        .get(stamp)
        .map_or(0, |settings| settings.quarter_turns);
    Degrees(f32::from(quarters % 4) * 90.0)
}

/// What one placement of the stamp covers, `values` read among `reader`'s paths.
pub fn stamp_footprint(
    editor: &Editor,
    content: &Content,
    reader: &FolderReader,
    stamp: &str,
    values: &BTreeMap<String, VarValue>,
) -> Size<Meters> {
    let yaw = stamp_yaw(editor, stamp);
    let loaded = content
        .by_path::<BlueprintDef>(stamp)
        .and_then(|blueprint| {
            let values = rescoped(values, reader.scope(), &ContentScope::of(content)).ok()?;
            Some(placement_node(blueprint, Pos::new(0.0, 0.0), yaw, values))
        });
    let mut errors = Vec::new();
    let flattening = area::blueprint::flatten(content, loaded.as_slice(), &mut errors);
    let mut bounds: Option<Rect<Meters>> = None;
    for placed in flattening.modules {
        if let Some(b) = shapes::bounds(&placed.module, None) {
            bounds = Some(bounds.map_or(b, |known| known.union(&b)));
        }
    }
    bounds.map_or(Size::splat(1.0), |b| {
        Size::new(b.size.width.max(1.0), b.size.height.max(1.0))
    })
}

pub fn stamp_input(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let response = frame.response;
    let Some(stamp) = editor.views.area.view(view_key).stamp().map(str::to_owned) else {
        return;
    };
    let alt = ui.input(|i| EditorKey::Unsnapped.held(i));
    let Some(tile) = frame.drag_tile() else {
        return;
    };
    let at = select::snap_to(tile, alt);
    let values = stamp_values(editor, Some(&stamp));
    let footprint = stamp_footprint(editor, &frame.content, &frame.reader, &stamp, &values);
    let spacing = editor
        .settings
        .stamps
        .get(&stamp)
        .and_then(|s| s.spacing)
        .map(|s| s as f32);
    let step = spacing.unwrap_or(footprint.width.max(footprint.height).max(1.0));
    if response.drag_started_by(egui::PointerButton::Primary) || response.clicked() {
        let view = editor.views.area.view_mut(view_key);
        view.set_drag(super::AreaDrag::Stamp {
            placed: vec![at],
            last: at,
        });
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        let view = editor.views.area.view_mut(view_key);
        if let Some(super::AreaDrag::Stamp { placed, last }) = view.drag_mut()
            && last.distance(at).0 >= step
        {
            placed.push(at);
            *last = at;
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) || response.clicked() {
        let view = editor.views.area.view_mut(view_key);
        let Some(super::AreaDrag::Stamp { placed, .. }) = view.take_drag() else {
            return;
        };
        place_stamps(editor, frame, &stamp, &values, placed);
    }
}

pub(super) fn place_stamps(
    editor: &mut Editor,
    frame: &AreaFrame,
    stamp: &str,
    values: &BTreeMap<String, VarValue>,
    placed: Vec<Pos<Meters>>,
) {
    let reader = frame.reader.clone();
    let (Some(blueprint), Some(Ok(def))) = (
        reader.id_of::<BlueprintDef>(stamp),
        editor.read::<BlueprintDef>(stamp),
    ) else {
        return;
    };
    let yaw = stamp_yaw(editor, stamp);
    let mut taken: HashSet<String> = frame.object_names();
    let mut modules = Vec::new();
    for at in &placed {
        let mut own = values.clone();
        for (name, variable) in &def.variables {
            if variable.ty == VariableType::Name {
                let text = |value: Option<&VarValue>| match value {
                    Some(VarValue::Str(text)) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                };
                let base = text(own.get(name))
                    .or_else(|| text(variable.default.as_ref()))
                    .unwrap_or_else(|| name.clone());
                let fresh = rows::free_name(&base, &taken);
                taken.insert(fresh.clone());
                own.insert(name.clone(), VarValue::Str(fresh));
            }
        }
        modules.push(placement_node(blueprint, *at, yaw, own));
    }
    let count = modules.len();
    let label = if count == 1 {
        format!("Place {stamp} in {}", frame.bundle)
    } else {
        format!("Place {count} × {stamp} in {}", frame.bundle)
    };
    let area_key = frame.bundle.clone();
    let text = match modules::place_modules(editor, &area_key, modules, &label) {
        modules::PlaceOutcome::Whole => None,
        modules::PlaceOutcome::Partly { skipped, reason } => {
            Some(format!("{skipped} of {count} skipped: {reason}"))
        }
    };
    if let Some(text) = &text {
        editor.toast(text.clone());
    }
    if let Some(view) = editor.views.area.views.get_mut(&super::view_key(&area_key)) {
        view.skipped = text;
    }
}

pub fn object_names(placed: &[area::blueprint::Flattened]) -> HashSet<String> {
    area::objects(placed)
        .filter_map(|(_, object)| object.name.clone())
        .collect()
}

pub fn stamp_overlay(editor: &Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let view = editor.views.area.view(view_key);
    let Some(stamp) = view.stamp().map(str::to_owned) else {
        return;
    };
    let alt = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
    let values = stamp_values(editor, Some(&stamp));
    let footprint = stamp_footprint(editor, &frame.content, &frame.reader, &stamp, &values);
    let color = Color32::from_rgb(90, 220, 220);
    let draw_ghost = |at: Pos<Meters>| {
        let rect = Rect::new(Pos::new(at.x - 0.5, at.y - 0.5), footprint);
        painter.add(egui::Shape::closed_line(
            canvas.outline(rect),
            EStroke::new(1.5, color),
        ));
        painter.circle_filled(canvas.to_screen(at), 3.0, color);
    };
    if let Some(super::AreaDrag::Stamp { placed, .. }) = view.drag() {
        for at in placed {
            draw_ghost(*at);
        }
    }
    if let Some(tile) = frame.pointer_tile {
        let at = select::snap_to(tile, alt);
        draw_ghost(at);
        painter.text(
            canvas.to_screen(at) + Vec2::new(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            &stamp,
            egui::FontId::proportional(11.0),
            color,
        );
    }
}
