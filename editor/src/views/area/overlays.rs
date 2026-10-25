use std::cell::RefCell;
use std::collections::HashSet;
use ui::egui;
use ui::ink;
use ui::paint;
use ui::theme;

use engine::core::audio::soundscape::EDGE_FALLOFF;
use engine::core::math::{Pos, Rect};
use engine::core::units::{Cell, GridDims, Meters, TileSize};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::{self, AreaModule, AreaModuleKind, Pictured};
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Vec2};

use super::population;
use super::{AreaFrame, AreaTool, selected_parts, shapes};
use crate::app::Editor;
use crate::forms;
use crate::views::navigation::Viewport;

const LABEL_GAP: f32 = 2.0;
const LABEL_SHIFTS: u32 = 3;

pub(super) fn overlay_grid(
    painter: &egui::Painter,
    frame: &AreaFrame,
    area: &area::Area,
    lines: bool,
) {
    let canvas = frame.canvas;
    let bounds = area.size.bounds();
    if matches!(frame.pictured, Pictured::Blueprint(_)) {
        let rect = canvas.rect(bounds).expand(1.0);
        paint::dashed_rect(painter, rect, EStroke::new(1.5, theme::DROP), 6.0, 4.0);
        frame.labels.chip(
            rect.right_top() + Vec2::new(6.0, 18.0),
            format!("{} × {} canvas", area.size.width, area.size.height),
            theme::DROP,
        );
    } else {
        painter.rect_stroke(
            canvas.rect(bounds),
            0.0,
            EStroke::new(1.0, Color32::from_gray(120)),
            egui::StrokeKind::Outside,
        );
    }
    if !lines || canvas.scale() < 6.0 {
        return;
    }
    let visible = canvas.visible();
    let x0 = visible.min().x.floor().max(-0.5) as i32;
    let x1 = visible.max().x.ceil().min(area.size.width - 0.5) as i32;
    let y0 = visible.min().y.floor().max(-0.5) as i32;
    let y1 = visible.max().y.ceil().min(area.size.height - 0.5) as i32;
    let color = Color32::from_rgba_unmultiplied(255, 255, 255, 28);
    for x in x0..=x1 {
        let sx = canvas.to_screen(Pos::new(x as f32 - 0.5, 0.0)).x;
        painter.line_segment(
            [
                Pos2::new(sx, canvas.rect(bounds).min.y),
                Pos2::new(sx, canvas.rect(bounds).max.y),
            ],
            EStroke::new(1.0, color),
        );
    }
    for y in y0..=y1 {
        let sy = canvas.to_screen(Pos::new(0.0, y as f32 - 0.5)).y;
        painter.line_segment(
            [
                Pos2::new(canvas.rect(bounds).min.x, sy),
                Pos2::new(canvas.rect(bounds).max.x, sy),
            ],
            EStroke::new(1.0, color),
        );
    }
}

pub(super) fn overlay_walk(painter: &egui::Painter, frame: &AreaFrame, area: &area::Area) {
    let canvas = frame.canvas;
    let outline = EStroke::new(1.0, ink::wash(theme::BLOCKED, 0.43));
    for polygon in area.mesh.polygons() {
        let corners = polygon.iter().map(|at| canvas.to_screen(*at)).collect();
        painter.add(egui::Shape::closed_line(corners, outline));
    }
    for zone in &area.safe_zones {
        painter.rect_filled(
            canvas.rect(*zone),
            0.0,
            Color32::from_rgba_unmultiplied(80, 120, 220, 40),
        );
    }
}

pub fn module_color(kind: Option<AreaModuleKind>) -> Color32 {
    match kind {
        Some(AreaModuleKind::Object) => Color32::from_rgb(255, 200, 80),
        Some(AreaModuleKind::SafeZone) => Color32::from_rgb(90, 160, 255),
        Some(AreaModuleKind::Soundscape) => Color32::from_rgb(170, 110, 255),
        Some(AreaModuleKind::Prop) => Color32::from_rgb(140, 220, 140),
        Some(AreaModuleKind::Scatter) => Color32::from_rgb(110, 190, 110),
        Some(AreaModuleKind::Light) => Color32::from_rgb(255, 240, 160),
        Some(AreaModuleKind::Blueprint) => Color32::from_rgb(90, 220, 220),
        Some(AreaModuleKind::Population) => Color32::from_rgb(255, 120, 120),
        Some(AreaModuleKind::Resident) => Color32::from_rgb(255, 160, 200),
        _ => Color32::from_gray(200),
    }
}

pub(super) fn overlay_modules(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let canvas = frame.canvas;
    let parts: HashSet<String> = selected_parts(editor, &frame.bundle);
    let selected: HashSet<String> = frame.selected_modules(&parts).into_iter().collect();
    if let Some((bounds, label)) = frame.folder_box(&parts) {
        let rect = canvas.rect(bounds).expand(3.0);
        paint::dashed_rect(painter, rect, EStroke::new(1.5, theme::ACCENT), 6.0, 4.0);
        frame
            .labels
            .chip(rect.min + Vec2::new(0.0, -4.0), label, theme::ACCENT);
    }
    let view = editor.views.area.view(view_key);
    let editing = view.in_place.first().cloned();
    let visible = canvas.visible();
    let hovered = match view.tool {
        AreaTool::Select | AreaTool::Module => frame.pointer_tile.and_then(|tile| frame.hit(tile)),
        _ => None,
    };
    for node in frame.modules.iter() {
        if frame.flags.hidden(&node.pointer) {
            continue;
        }
        let module = node.module();
        let Some(bounds) = frame.bounds_of(node) else {
            continue;
        };
        if !visible.intersects(&bounds) {
            continue;
        }
        let kind = module.kind();
        let is_selected = selected.contains(&node.pointer);
        let is_hovered = hovered.as_ref() == Some(&node.pointer);
        let label = shapes::label(module, frame.reader.as_ref());
        if let AreaModule::Blueprint(placement) = module
            && frame
                .reader
                .path_of(placement.blueprint)
                .and_then(|path| frame.content.by_path::<BlueprintDef>(path))
                .is_none()
        {
            let rect = canvas.rect(bounds);
            paint::dashed_rect(painter, rect, EStroke::new(1.5, theme::DANGER), 5.0, 3.0);
            frame.labels.chip(
                rect.min + Vec2::new(0.0, -4.0),
                format!("{label} doesn't load"),
                theme::DANGER,
            );
            continue;
        }
        if kind == AreaModuleKind::Blueprint && !is_selected && !is_hovered {
            continue;
        }
        let color = module_color(Some(kind));
        let rect = canvas.rect(bounds);
        let dim = editing.as_ref().is_some_and(|e| *e != node.pointer);
        let alpha = if dim {
            0.24
        } else if is_selected {
            1.0
        } else {
            0.67
        };
        let color = ink::wash(color, alpha);
        let width = if is_selected { 2.0 } else { 1.0 };
        match kind {
            AreaModuleKind::Soundscape if shapes::is_ellipse(module) => {
                painter.add(egui::Shape::closed_line(
                    ellipse_points(rect, 48),
                    EStroke::new(width, color),
                ));
            }
            AreaModuleKind::Prop => {
                draw_prop(painter, frame, module, color, width);
            }
            AreaModuleKind::Light => {
                if let AreaModule::Light(light) = module {
                    painter.circle_stroke(
                        rect.center(),
                        (canvas.scale() * 0.3).max(3.0),
                        EStroke::new(width, color),
                    );
                    let reach = Rect::new(
                        light.at - engine::core::math::Offset::new(light.radius.0, light.radius.0),
                        engine::core::math::Size::splat(light.radius.0 * 2.0),
                    );
                    painter.add(egui::Shape::dashed_line(
                        &ellipse_points(canvas.rect(reach), 48),
                        EStroke::new(1.0, ink::wash(color, 0.5)),
                        6.0,
                        6.0,
                    ));
                }
            }
            AreaModuleKind::Resident | AreaModuleKind::Object if shapes::is_point(module) => {
                painter.circle_stroke(
                    rect.center(),
                    (canvas.scale() * 0.35).max(3.0),
                    EStroke::new(width, color),
                );
            }
            _ => {
                painter.rect_stroke(
                    rect,
                    0.0,
                    EStroke::new(width, color),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if is_selected
            || is_hovered
            || (kind == AreaModuleKind::Population && canvas.scale() >= 2.0)
        {
            frame
                .labels
                .chip(rect.min + Vec2::new(0.0, -4.0), label, color);
        }
        if parts.contains(&node.pointer) && shapes::has_size(module) {
            for corner in [
                rect.min,
                Pos2::new(rect.max.x, rect.min.y),
                Pos2::new(rect.min.x, rect.max.y),
                rect.max,
            ] {
                painter.rect_filled(
                    ERect::from_center_size(corner, Vec2::splat(6.0)),
                    0.0,
                    color,
                );
            }
        }
    }
}

pub(super) fn overlay_soundscape_reach(
    editor: &Editor,
    painter: &egui::Painter,
    frame: &AreaFrame,
) {
    let canvas = frame.canvas;
    for part in selected_parts(editor, &frame.bundle) {
        let Some(module @ AreaModule::Soundscape(soundscape)) = frame.module(&part) else {
            continue;
        };
        let bounds = soundscape.shape.bounds();
        let reach = soundscape.emit + EDGE_FALLOFF.0;
        let outer = bounds.inflate(reach, reach);
        let rect = canvas.rect(outer);
        let color = Color32::from_rgba_unmultiplied(170, 110, 255, 120);
        if shapes::is_ellipse(module) {
            painter.add(egui::Shape::dashed_line(
                &ellipse_points(rect, 64),
                EStroke::new(1.0, color),
                6.0,
                6.0,
            ));
        } else {
            paint::dashed_rect(painter, rect, EStroke::new(1.0, color), 6.0, 6.0);
        }
        frame.labels.chip(
            rect.left_bottom() + Vec2::new(0.0, 22.0),
            format!("heard within {} m", forms::metres(reach)),
            Color32::from_rgb(190, 150, 255),
        );
    }
}

pub(super) fn overlay_populations(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
    area: &area::Area,
) {
    let canvas = frame.canvas;
    let live: HashSet<String> = frame
        .modules
        .iter()
        .filter(|module| matches!(module.module(), AreaModule::Population(_)))
        .map(|module| module.pointer.clone())
        .collect();
    let view = editor.views.area.view_mut(view_key);
    view.population_previews.retain(|key, _| live.contains(key));
    let previews = view.population_previews.clone();
    for node in frame.modules.iter() {
        let AreaModule::Population(population) = node.module() else {
            continue;
        };
        if frame.flags.hidden(&node.pointer) {
            continue;
        }
        let preview = previews.get(&node.pointer).copied().unwrap_or_default();
        if preview.hidden {
            continue;
        }
        for at in population::population_places(area, population, preview.seed) {
            draw_stand(painter, canvas, at);
        }
    }
}

#[derive(Default)]
pub struct AreaLabels(RefCell<Vec<AreaLabel>>);

struct AreaLabel {
    bottom_left: Pos2,
    text: String,
    color: Color32,
}

impl AreaLabels {
    pub fn chip(&self, bottom_left: Pos2, text: impl Into<String>, color: Color32) {
        self.0.borrow_mut().push(AreaLabel {
            bottom_left,
            text: text.into(),
            color,
        });
    }

    pub fn draw(&self, painter: &egui::Painter) {
        let mut placed: Vec<ERect> = Vec::new();
        for label in self.0.take() {
            let galley =
                painter.layout_no_wrap(label.text, egui::FontId::monospace(11.5), theme::TEXT);
            let size = galley.size() + Vec2::new(14.0, 6.0);
            let preferred = ERect::from_min_size(label.bottom_left - Vec2::new(0.0, size.y), size);
            let back = free_slot(preferred, &placed);
            placed.push(back);
            ui::slip::paint(painter, back);
            painter.rect_filled(
                ERect::from_min_size(back.min, Vec2::new(3.0, back.height())),
                0.0,
                label.color,
            );
            painter.galley(back.min + Vec2::new(9.0, 3.0), galley, theme::TEXT);
        }
    }
}

fn free_slot(preferred: ERect, placed: &[ERect]) -> ERect {
    let row = preferred.height() + LABEL_GAP;
    let shifted = (1..=LABEL_SHIFTS)
        .flat_map(|shift| [-(shift as f32), shift as f32])
        .map(|rows| preferred.translate(Vec2::new(0.0, rows * row)));
    std::iter::once(preferred)
        .chain(shifted)
        .find(|slot| !placed.iter().any(|other| other.intersects(*slot)))
        .unwrap_or(preferred)
}

pub(super) fn overlay_surfaces(painter: &egui::Painter, frame: &AreaFrame, area: &area::Area) {
    let canvas = frame.canvas;
    if canvas.scale() < 3.0 {
        return;
    }
    let visible = canvas.visible();
    let mut palette: Vec<(String, Color32)> = Vec::new();
    for cell in area.size.grid().cells() {
        let tile = cell.center();
        if !visible.contains(tile) {
            continue;
        }
        let Some(surface) = area.footstep_at(tile) else {
            continue;
        };
        let key = surface.path(&frame.content);
        let color = match palette.iter().find(|(k, _)| k == key) {
            Some((_, color)) => *color,
            None => {
                let hue = (palette.len() as f32 * 0.37) % 1.0;
                let color = hsv(hue, 0.55, 0.95);
                palette.push((key.to_owned(), color));
                color
            }
        };
        let rect = canvas.rect(cell.bounds());
        painter.rect_filled(
            rect,
            0.0,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70),
        );
        painter.line_segment(
            [rect.left_bottom(), rect.right_top()],
            EStroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120),
            ),
        );
    }
}

/// Where someone stands, as the shadow at their feet.
fn draw_stand(painter: &egui::Painter, canvas: Viewport<Meters>, at: Pos<Meters>) {
    let feet = canvas.to_screen(at) + Vec2::new(0.0, canvas.scale() * 0.5);
    painter.add(egui::Shape::ellipse_filled(
        feet,
        Vec2::new(canvas.scale() * 0.35, canvas.scale() * 0.12),
        Color32::from_rgba_unmultiplied(0, 0, 0, 70),
    ));
}

fn ellipse_points(rect: ERect, segments: usize) -> Vec<Pos2> {
    let center = rect.center();
    let radius = rect.size() / 2.0;
    (0..segments)
        .map(|i| {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            Pos2::new(center.x + radius.x * t.cos(), center.y + radius.y * t.sin())
        })
        .collect()
}

pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// A prop as its footprint stands on the ground, or a stake where it has none, and the props a
/// scatter strews shown by the area that holds them.
fn draw_prop(
    painter: &egui::Painter,
    frame: &AreaFrame,
    module: &AreaModule,
    color: Color32,
    width: f32,
) {
    let AreaModule::Prop(prop) = module else {
        return;
    };
    let canvas = frame.canvas;
    let footprint = frame
        .reader
        .path_of(prop.prop)
        .and_then(|path| frame.content.by_path::<PropDef>(path))
        .map(|id| id.get(&frame.content).footprint.clone())
        .filter(|footprint| !footprint.is_none());
    match footprint {
        Some(footprint) => {
            let points: Vec<Pos2> = footprint
                .outline(prop.at, prop.yaw, prop.scale)
                .into_iter()
                .map(|point| canvas.to_screen(point))
                .collect();
            painter.add(egui::Shape::closed_line(points, EStroke::new(width, color)));
        }
        None => {
            painter.circle_stroke(
                canvas.to_screen(prop.at),
                (canvas.scale() * 0.25).max(3.0),
                EStroke::new(width, color),
            );
        }
    }
    let heading =
        canvas.to_screen(prop.at + prop.yaw.rotate(engine::core::math::Offset::new(0.0, -0.5)));
    painter.line_segment(
        [canvas.to_screen(prop.at), heading],
        EStroke::new(width, color),
    );
}

/// Every prop the area stands, scattered ones too, as faint marks under the module outlines.
pub(super) fn overlay_props(painter: &egui::Painter, frame: &AreaFrame, area: &area::Area) {
    let canvas = frame.canvas;
    if canvas.scale() < 4.0 {
        return;
    }
    let visible = canvas.visible();
    let color = ink::wash(module_color(Some(AreaModuleKind::Prop)), 0.45);
    for prop in &area.props {
        if !visible.contains(prop.at) {
            continue;
        }
        let def = prop.prop.get(&frame.content);
        let points: Vec<Pos2> = def
            .footprint
            .outline(prop.at, prop.yaw, prop.scale)
            .into_iter()
            .map(|point| canvas.to_screen(point))
            .collect();
        if points.len() >= 3 {
            painter.add(egui::Shape::closed_line(points, EStroke::new(1.0, color)));
        } else {
            painter.circle_stroke(
                canvas.to_screen(prop.at),
                (canvas.scale() * 0.2).max(2.0),
                EStroke::new(1.0, color),
            );
        }
    }
}
