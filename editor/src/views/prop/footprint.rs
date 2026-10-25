//! The footprint from above: the ground the prop takes up, drawn over its model and shaped by
//! handles, the grown outline walkers keep off beside it.

use bevy::math::Vec3;
use engine::core::geometry::Degrees;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::area::AreaModuleKind;
use engine::systems::area::prop::{Footprint, MAX_POLYGON_POINTS, MIN_POLYGON_POINTS, PropDef};
use ui::egui::{self, Sense, Shape, Stroke, Ui, Vec2};
use ui::theme;

use super::super::area::camera::{self, AreaCanvas};
use super::super::area::overlays::module_color;
use super::super::area::place::draw_footprint;
use super::super::stage;
use crate::app::Editor;

/// How close to a handle, in points, the pointer takes it.
const GRIP: f32 = 10.0;
const SMALLEST: f32 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    None,
    Circle,
    Box,
    Polygon,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::None, Kind::Circle, Kind::Box, Kind::Polygon];

    pub fn of(footprint: &Footprint) -> Kind {
        match footprint {
            Footprint::None => Kind::None,
            Footprint::Circle { .. } => Kind::Circle,
            Footprint::Box { .. } => Kind::Box,
            Footprint::Polygon { .. } => Kind::Polygon,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::None => "no footprint",
            Kind::Circle => "circle",
            Kind::Box => "box",
            Kind::Polygon => "polygon",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Radius,
    Width,
    Depth,
    Point(usize),
}

/// A handle being dragged, and the footprint as the drag has shaped it so far.
pub struct Drag {
    pub handle: Handle,
    pub footprint: Footprint,
}

/// `from` as a footprint of `kind`, as big as it was, or as the model's box (`bounds`) when it took
/// no ground.
pub fn converted(from: &Footprint, bounds: Option<(Vec3, Vec3)>, kind: Kind) -> Footprint {
    let (half_width, half_depth) = half_extents(from, bounds);
    match kind {
        Kind::None => Footprint::None,
        Kind::Circle => Footprint::Circle {
            radius: Meters(half_width.max(half_depth)),
        },
        Kind::Box => Footprint::Box {
            width: Meters(half_width * 2.0),
            depth: Meters(half_depth * 2.0),
        },
        Kind::Polygon => Footprint::Polygon {
            points: vec![
                Pos::new(-half_width, -half_depth),
                Pos::new(half_width, -half_depth),
                Pos::new(half_width, half_depth),
                Pos::new(-half_width, half_depth),
            ],
            fitted_to: None,
        },
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef, size: Vec2) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let Some(picture) = super::picture(editor, key) else {
        return;
    };
    let view_key = format!("prop-top:{key}");
    stage::show_picture(editor, ui, &painter, rect, &view_key);
    let bounds = super::model_bounds(editor, key);
    let view = editor.views.prop.view_mut(key);
    camera::navigate(ui, &response, &mut view.top, rect.height());
    view.top.top_down();
    let camera = view.top;
    let canvas = AreaCanvas::new(rect, camera, picture.1.area.clone());
    let at = stage::center();
    let color = module_color(Some(AreaModuleKind::Prop));
    if let Some((min, max)) = bounds {
        let corners = [
            (min.x, min.z),
            (max.x, min.z),
            (max.x, max.z),
            (min.x, max.z),
        ]
        .map(|(x, z)| at + Offset::new(x, z));
        painter.add(Shape::dashed_line(
            &canvas.closed(&corners),
            Stroke::new(1.0, theme::MUTED),
            3.0,
            3.0,
        ));
    }
    let working = view
        .drag
        .as_ref()
        .map_or_else(|| row.footprint.clone(), |drag| drag.footprint.clone());
    draw_footprint(
        &painter,
        &canvas,
        &working,
        at,
        Degrees(0.0),
        1.0,
        Stroke::new(1.5, color),
    );
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let near = |local: Pos<Meters>| {
        pointer.is_some_and(|pointer| {
            canvas.to_screen(at + local.to_vector()).distance(pointer) <= GRIP
        })
    };
    let handles = handles_of(&working);
    let hot = handles.iter().position(|(_, local)| near(*local));
    let held = view.drag.as_ref().map(|drag| drag.handle);
    for (index, (handle, local)) in handles.iter().enumerate() {
        let lit = hot == Some(index) || held == Some(*handle);
        stage::handle(
            &painter,
            canvas.to_screen(at + local.to_vector()),
            lit,
            color,
        );
    }
    let midpoints = midpoints_of(&working);
    for midpoint in &midpoints {
        painter.circle_stroke(
            canvas.to_screen(at + midpoint.to_vector()),
            3.5,
            Stroke::new(1.0, color),
        );
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(index) = hot {
            view.drag = Some(Drag {
                handle: handles[index].0,
                footprint: working.clone(),
            });
        } else if let Some(edge) = midpoints.iter().position(|midpoint| near(*midpoint))
            && let Footprint::Polygon { points, .. } = &working
            && points.len() < MAX_POLYGON_POINTS
        {
            let mut points = points.clone();
            points.insert(edge + 1, midpoints[edge]);
            view.drag = Some(Drag {
                handle: Handle::Point(edge + 1),
                footprint: Footprint::Polygon {
                    points,
                    fitted_to: None,
                },
            });
        }
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = &mut view.drag
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(ground) = canvas.to_tile(pointer)
    {
        move_handle(&mut drag.footprint, drag.handle, ground - at);
        ui.ctx().request_repaint();
    }
    let mut landed: Option<Footprint> = None;
    if response.drag_stopped()
        && let Some(drag) = view.drag.take()
    {
        landed = Some(drag.footprint);
    }
    if response.secondary_clicked()
        && let Some(index) = hot
        && let (Handle::Point(point), Footprint::Polygon { points, .. }) =
            (handles[index].0, &working)
        && points.len() > MIN_POLYGON_POINTS
    {
        let mut points = points.clone();
        points.remove(point);
        landed = Some(Footprint::Polygon {
            points,
            fitted_to: None,
        });
    }
    painter.text(
        rect.left_top() + Vec2::new(8.0, 6.0),
        egui::Align2::LEFT_TOP,
        describe(&working),
        egui::FontId::monospace(11.0),
        theme::TEXT,
    );
    if let Some(footprint) = landed {
        super::shape(editor, key, footprint);
    }
    stage::request(editor, ui, &view_key, rect, camera, picture, Vec::new());
}

fn half_extents(from: &Footprint, bounds: Option<(Vec3, Vec3)>) -> (f32, f32) {
    if let Some(taken) = from.bounds(Pos::origin(), Degrees(0.0), 1.0) {
        return (
            (taken.size.width / 2.0).max(SMALLEST),
            (taken.size.height / 2.0).max(SMALLEST),
        );
    }
    match bounds {
        Some((min, max)) => (
            ((max.x - min.x) / 2.0).max(0.1),
            ((max.z - min.z) / 2.0).max(0.1),
        ),
        None => (0.5, 0.5),
    }
}

fn handles_of(footprint: &Footprint) -> Vec<(Handle, Pos<Meters>)> {
    match footprint {
        Footprint::None => Vec::new(),
        Footprint::Circle { radius } => vec![(Handle::Radius, Pos::new(radius.0, 0.0))],
        Footprint::Box { width, depth } => vec![
            (Handle::Width, Pos::new(width.0 / 2.0, 0.0)),
            (Handle::Depth, Pos::new(0.0, depth.0 / 2.0)),
        ],
        Footprint::Polygon { points, .. } => points
            .iter()
            .enumerate()
            .map(|(index, point)| (Handle::Point(index), *point))
            .collect(),
    }
}

fn midpoints_of(footprint: &Footprint) -> Vec<Pos<Meters>> {
    match footprint {
        Footprint::Polygon { points, .. } => (0..points.len())
            .map(|index| {
                let next = points[(index + 1) % points.len()];
                points[index].lerp(next, 0.5)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn move_handle(footprint: &mut Footprint, handle: Handle, local: Offset<Meters>) {
    match (footprint, handle) {
        (Footprint::Circle { radius }, Handle::Radius) => {
            radius.0 = local.length().max(SMALLEST);
        }
        (Footprint::Box { width, .. }, Handle::Width) => {
            width.0 = (local.x.abs() * 2.0).max(SMALLEST);
        }
        (Footprint::Box { depth, .. }, Handle::Depth) => {
            depth.0 = (local.y.abs() * 2.0).max(SMALLEST);
        }
        (Footprint::Polygon { points, fitted_to }, Handle::Point(index)) => {
            if let Some(point) = points.get_mut(index) {
                *point = Pos::origin() + local;
                *fitted_to = None;
            }
        }
        _ => {}
    }
}

fn describe(footprint: &Footprint) -> String {
    match footprint {
        Footprint::None => "no footprint · walkers pass through".to_owned(),
        Footprint::Circle { radius } => format!("circle · {:.2} m across", radius.0 * 2.0),
        Footprint::Box { width, depth } => format!("box · {:.2} × {:.2} m", width.0, depth.0),
        Footprint::Polygon { points, .. } => format!("polygon · {} corners", points.len()),
    }
}
