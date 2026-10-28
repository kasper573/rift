//! A model's hitbox on a stage: the capsule the game picks and hits the body by, drawn round where
//! the body stands, picked by a click on its outline, sized by its two handles, and set by number
//! in the inspector.

use std::ops::RangeInclusive;

use engine::core::geometry::Capsule;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::actor::model::{HITBOX_HEIGHTS, HITBOX_RADII};
use ui::egui::{self, Pos2, Response, Stroke, Ui, Vec2};
use ui::{form, theme};

use super::super::area::camera::AreaCanvas;
use super::super::stage;

/// How near the pointer, in points, takes a handle or the outline.
const REACH: f32 = 8.0;

/// Whether a stage shows the hitbox, and the handle being dragged.
pub struct HitboxHandles {
    pub shown: bool,
    drag: Option<Drag>,
}

impl HitboxHandles {
    /// Whether a handle of the hitbox is being dragged.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

impl Default for HitboxHandles {
    fn default() -> HitboxHandles {
        HitboxHandles {
            shown: true,
            drag: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Handle {
    Radius,
    Height,
}

/// A handle being dragged, and the capsule as the drag has sized it.
struct Drag {
    handle: Handle,
    hitbox: Capsule,
}

/// What the pointer did to the hitbox this frame: picked it, or sized it once a drag let go.
#[derive(Default)]
pub struct Touched {
    pub picked: bool,
    pub sized: Option<Capsule>,
}

/// Draws `hitbox` round a body standing on the ground at `at` over `canvas`, as `chosen` or not,
/// and gives what the pointer on `response` did to it.
pub fn overlay(
    ui: &Ui,
    canvas: &AreaCanvas,
    response: &Response,
    at: Pos<Meters>,
    hitbox: Capsule,
    chosen: bool,
    handles: &mut HitboxHandles,
) -> Touched {
    if !handles.shown {
        handles.drag = None;
        return Touched::default();
    }
    let working = handles.drag.as_ref().map_or(hitbox, |drag| drag.hitbox);
    let painter = ui.painter_at(canvas.screen);
    let ink = if chosen {
        theme::ACCENT
    } else {
        ui::ink::wash(theme::ACCENT, 0.7)
    };
    let stroke = Stroke::new(if chosen { 1.5 } else { 1.0 }, ink);
    let (sin, cos) = canvas.camera.yaw.0.sin_cos();
    let right = Offset::<Meters>::new(cos, -sin);
    let mut outline: Vec<[Pos2; 2]> = Vec::new();
    for level in [0.0, working.height.0 / 2.0, working.height.0] {
        let lift = canvas.lifted(at, Meters(level)) - canvas.to_screen(at);
        let ring: Vec<Pos2> = canvas
            .circle(at, working.radius.0)
            .into_iter()
            .map(|point| point + lift)
            .collect();
        outline.extend(
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| [*a, *b]),
        );
        painter.add(egui::Shape::closed_line(ring, stroke));
    }
    for side in [-1.0, 1.0] {
        let foot = at + right * (working.radius.0 * side);
        let side = [canvas.to_screen(foot), canvas.lifted(foot, working.height)];
        painter.line_segment(side, stroke);
        outline.push(side);
    }
    let radius_handle = canvas.to_screen(at + right * working.radius.0);
    let height_handle = canvas.lifted(at, working.height);
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let hot = pointer.and_then(|pointer| {
        if pointer.distance(radius_handle) <= REACH {
            Some(Handle::Radius)
        } else if pointer.distance(height_handle) <= REACH {
            Some(Handle::Height)
        } else {
            None
        }
    });
    let held = handles.drag.as_ref().map(|drag| drag.handle);
    for (handle, place) in [
        (Handle::Radius, radius_handle),
        (Handle::Height, height_handle),
    ] {
        stage::handle(
            &painter,
            place,
            hot == Some(handle) || held == Some(handle),
            ink,
        );
    }
    let mut touched = Touched::default();
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        touched.picked = hot.is_some()
            || outline
                .iter()
                .any(|[a, b]| distance_to_segment(pointer, *a, *b) <= REACH);
    }
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(handle) = hot
    {
        handles.drag = Some(Drag {
            handle,
            hitbox: working,
        });
        touched.picked = true;
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = &mut handles.drag
    {
        match drag.handle {
            Handle::Radius => {
                if let Some(pointer) = response.interact_pointer_pos()
                    && let Some(ground) = canvas.to_ground(pointer)
                {
                    drag.hitbox.radius = Meters(within(&HITBOX_RADII, (ground - at).length()));
                }
            }
            Handle::Height => {
                let per_metre = canvas.to_screen(at).y - canvas.lifted(at, Meters(1.0)).y;
                if per_metre.abs() > f32::EPSILON {
                    let delta = -response.drag_delta().y / per_metre;
                    drag.hitbox.height =
                        Meters(within(&HITBOX_HEIGHTS, drag.hitbox.height.0 + delta));
                }
            }
        }
        painter.text(
            height_handle + Vec2::new(10.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            across_and_tall(drag.hitbox),
            egui::FontId::monospace(11.0),
            theme::TEXT,
        );
        ui.ctx().request_repaint();
    }
    if response.drag_stopped()
        && let Some(drag) = handles.drag.take()
    {
        touched.sized = Some(drag.hitbox);
    }
    touched
}

/// The hitbox's sizes as number fields, giving the capsule as they leave it when one changed.
pub fn fields(ui: &mut Ui, hitbox: Capsule) -> Option<Capsule> {
    let mut edited = hitbox;
    for (label, value, range) in [
        ("radius · m", &mut edited.radius, HITBOX_RADII),
        ("height · m", &mut edited.height, HITBOX_HEIGHTS),
    ] {
        form::kv(ui, label, |ui| {
            ui.add(
                egui::DragValue::new(&mut value.0)
                    .speed(0.005)
                    .range(range)
                    .fixed_decimals(3),
            );
        });
    }
    form::hint(
        ui,
        "The game picks and hits the body by this capsule. Drag its side handle on the stage for \
         the radius and its top handle for the height.",
    );
    (edited != hitbox).then_some(edited)
}

fn across_and_tall(hitbox: Capsule) -> String {
    format!(
        "{:.2} m across · {:.2} m tall",
        hitbox.radius.0 * 2.0,
        hitbox.height.0
    )
}

fn within(range: &RangeInclusive<f32>, value: f32) -> f32 {
    value.clamp(*range.start(), *range.end())
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() <= f32::EPSILON {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}
