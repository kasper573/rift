//! What the view draws over the viewport's picture: the area's edge, its modules as outlines on
//! the ground, the navmesh, the height contours and every prop's footprint, each bent over the
//! terrain the way the picture shows it.

use std::cell::RefCell;
use std::collections::HashSet;
use ui::egui;
use ui::ink;
use ui::theme;

use engine::core::audio::soundscape::EDGE_FALLOFF;
use engine::core::math::Rect;
use engine::core::units::{MeterSize, Meters};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::{AreaModule, AreaModuleKind, Pictured};
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Vec2};

use super::{AreaFrame, AreaTool, place, selected_parts, shapes};
use crate::app::Editor;
use crate::forms;

const LABEL_GAP: f32 = 2.0;
const LABEL_SHIFTS: u32 = 3;

/// The edge of the area's ground, or a blueprint's canvas with its size.
pub(super) fn overlay_bounds(painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let area = frame.area();
    let bounds = area.size.bounds();
    if matches!(frame.pictured, Pictured::Blueprint(_)) {
        painter.add(egui::Shape::dashed_line(
            &canvas.outline(bounds),
            EStroke::new(1.5, theme::DROP),
            6.0,
            4.0,
        ));
        frame.labels.chip(
            canvas.rect(bounds).right_top() + Vec2::new(6.0, 18.0),
            format!("{} × {} canvas", area.size.width, area.size.height),
            theme::DROP,
        );
    } else {
        painter.add(egui::Shape::closed_line(
            canvas.outline(bounds),
            EStroke::new(1.0, Color32::from_gray(120)),
        ));
    }
}

/// The navmesh's polygons and the safe zones.
pub(super) fn overlay_navmesh(painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let area = frame.area();
    let outline = EStroke::new(1.0, ink::wash(theme::BLOCKED, 0.43));
    for polygon in area.mesh.polygons() {
        painter.add(egui::Shape::closed_line(canvas.closed(polygon), outline));
    }
    for zone in &area.safe_zones {
        let corners = engine::core::geometry::corners(*zone).map(|corner| canvas.to_screen(corner));
        painter.add(egui::Shape::convex_polygon(
            corners.to_vec(),
            Color32::from_rgba_unmultiplied(80, 120, 220, 40),
            EStroke::NONE,
        ));
    }
}

/// Lines where the ground crosses each whole metre of height, every fifth one stronger.
pub(super) fn overlay_contours(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let canvas = &frame.canvas;
    let contours = editor
        .views
        .area
        .view_mut(view_key)
        .contours
        .of(frame.made, &frame.area().heights);
    for contour in contours.iter() {
        let major = (contour.height.0 / 5.0).fract().abs() < 0.01;
        let color = if major {
            Color32::from_rgba_unmultiplied(255, 255, 255, 150)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 70)
        };
        for [from, to] in &contour.segments {
            painter.line_segment(
                [canvas.to_screen(*from), canvas.to_screen(*to)],
                EStroke::new(1.0, color),
            );
        }
        if major && let Some([from, _]) = contour.segments.first() {
            frame.labels.chip(
                canvas.to_screen(*from),
                format!("{:+} m", contour.height.0),
                Color32::from_gray(200),
            );
        }
    }
}

/// Every prop the area stands, scattered ones too, as its footprint and the outline the navmesh
/// keeps clear of it.
pub(super) fn overlay_footprints(painter: &egui::Painter, frame: &AreaFrame) {
    let color = ink::wash(module_color(Some(AreaModuleKind::Prop)), 0.6);
    for prop in &frame.area().props {
        let def = prop.prop.get(&frame.content);
        place::draw_footprint(
            painter,
            &frame.canvas,
            &def.footprint,
            prop.at,
            prop.yaw,
            prop.scale,
            EStroke::new(1.0, color),
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
    let canvas = &frame.canvas;
    let parts: HashSet<String> = selected_parts(editor, &frame.bundle);
    let selected: HashSet<String> = frame.selected_modules(&parts).into_iter().collect();
    if let Some((bounds, label)) = frame.folder_box(&parts) {
        painter.add(egui::Shape::dashed_line(
            &canvas.outline(bounds.inflate(0.2, 0.2)),
            EStroke::new(1.5, theme::ACCENT),
            6.0,
            4.0,
        ));
        frame.labels.chip(
            canvas.rect(bounds).min + Vec2::new(0.0, -4.0),
            label,
            theme::ACCENT,
        );
    }
    let view = editor.views.area.view(view_key);
    let editing = view.in_place.first().cloned();
    let visible = canvas.visible();
    let hovered = match view.tool {
        AreaTool::Select | AreaTool::Module => frame.hit_pointer(),
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
            painter.add(egui::Shape::dashed_line(
                &canvas.outline(bounds),
                EStroke::new(1.5, theme::DANGER),
                5.0,
                3.0,
            ));
            frame.labels.chip(
                canvas.rect(bounds).min + Vec2::new(0.0, -4.0),
                format!("{label} doesn't load"),
                theme::DANGER,
            );
            continue;
        }
        if kind == AreaModuleKind::Blueprint && !is_selected && !is_hovered {
            continue;
        }
        let color = module_color(Some(kind));
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
        let screen_box = canvas.rect(bounds);
        match module {
            AreaModule::Soundscape(_) if shapes::is_ellipse(module) => {
                painter.add(egui::Shape::closed_line(
                    canvas.ellipse(bounds),
                    EStroke::new(width, color),
                ));
            }
            AreaModule::Prop(prop) => {
                let def = frame
                    .reader
                    .path_of(prop.prop)
                    .and_then(|path| frame.content.by_path::<PropDef>(path))
                    .map(|id| id.get(&frame.content));
                match def {
                    Some(def) => place::draw_footprint(
                        painter,
                        &frame.canvas,
                        &def.footprint,
                        prop.at,
                        prop.yaw,
                        prop.scale,
                        EStroke::new(width, color),
                    ),
                    None => {
                        painter.add(egui::Shape::closed_line(
                            canvas.circle(prop.at, 0.25),
                            EStroke::new(width, color),
                        ));
                    }
                }
            }
            AreaModule::Light(light) => {
                let foot = canvas.to_screen(light.at);
                let bulb = canvas.lifted(light.at, light.height);
                painter.line_segment([foot, bulb], EStroke::new(width, ink::wash(color, 0.6)));
                painter.circle_stroke(bulb, 5.0, EStroke::new(width, color));
                painter.add(egui::Shape::dashed_line(
                    &canvas.circle(light.at, light.radius.0),
                    EStroke::new(1.0, ink::wash(color, 0.5)),
                    6.0,
                    6.0,
                ));
            }
            AreaModule::Resident(resident) => {
                painter.add(egui::Shape::closed_line(
                    canvas.circle(resident.at, 0.35),
                    EStroke::new(width, color),
                ));
                let heading = resident.at
                    + resident
                        .facing
                        .rotate(engine::core::math::Offset::new(0.0, -0.7));
                painter.add(egui::Shape::line(
                    canvas.polyline(&[resident.at, heading]),
                    EStroke::new(width, color),
                ));
            }
            AreaModule::Object(_) if shapes::is_point(module) => {
                let center = bounds.center();
                painter.add(egui::Shape::closed_line(
                    canvas.circle(engine::core::math::Pos::new(center.x, center.y), 0.35),
                    EStroke::new(width, color),
                ));
            }
            _ => {
                painter.add(egui::Shape::closed_line(
                    canvas.outline(bounds),
                    EStroke::new(width, color),
                ));
            }
        }
        if is_selected || is_hovered || kind == AreaModuleKind::Population {
            frame
                .labels
                .chip(screen_box.min + Vec2::new(0.0, -4.0), label, color);
        }
        if parts.contains(&node.pointer) && shapes::has_size(module) {
            for corner in engine::core::geometry::corners(bounds) {
                painter.rect_filled(
                    ERect::from_center_size(canvas.to_screen(corner), Vec2::splat(6.0)),
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
    let canvas = &frame.canvas;
    for part in selected_parts(editor, &frame.bundle) {
        let Some(module @ AreaModule::Soundscape(soundscape)) = frame.module(&part) else {
            continue;
        };
        let bounds = soundscape.shape.bounds();
        let reach = soundscape.emit + EDGE_FALLOFF.0;
        let outer = bounds.inflate(reach, reach);
        let color = Color32::from_rgba_unmultiplied(170, 110, 255, 120);
        let line = if shapes::is_ellipse(module) {
            canvas.ellipse(outer)
        } else {
            canvas.outline(outer)
        };
        painter.add(egui::Shape::dashed_line(
            &line,
            EStroke::new(1.0, color),
            6.0,
            6.0,
        ));
        frame.labels.chip(
            canvas.rect(outer).left_bottom() + Vec2::new(0.0, 22.0),
            format!("heard within {} m", forms::metres(reach)),
            Color32::from_rgb(190, 150, 255),
        );
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
        let clip = painter.clip_rect();
        for label in self.0.take() {
            if !clip.contains(label.bottom_left) {
                continue;
            }
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

/// A rectangle of ground as the overlays box it on the screen.
pub fn screen_rect(frame: &AreaFrame, rect: Rect<Meters>) -> ERect {
    frame.canvas.rect(rect)
}
