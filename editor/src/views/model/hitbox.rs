//! The hitbox page: the capsule the game picks and hits the rig by, drawn round it on the stage
//! and sized by two handles.

use engine::core::geometry::Capsule;
use engine::core::math::Offset;
use engine::core::units::Meters;
use engine::systems::actor::Action;
use engine::systems::actor::model::ModelDef;
use ui::egui::{self, Stroke, Ui, Vec2};
use ui::{form, theme};

use super::super::stage;
use crate::app::Editor;

const GRIP: f32 = 10.0;
const SMALLEST: f32 = 0.1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Radius,
    Height,
}

/// A handle being dragged, and the capsule as the drag has sized it.
pub struct Drag {
    pub handle: Handle,
    pub hitbox: Capsule,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef) {
    let names = super::clip_names(editor, row);
    let idle = row
        .clips
        .get(&Action::Idle)
        .and_then(|clip| names.iter().position(|name| *name == clip.name));
    let height = ui.available_height();
    let Some(turntable) = super::turntable(
        editor,
        ui,
        key,
        Vec2::new(ui.available_width(), height - 60.0),
    ) else {
        super::nothing_to_stand(ui);
        return;
    };
    let painter = ui.painter_at(turntable.rect);
    let canvas = &turntable.canvas;
    let response = &turntable.response;
    let view = editor.views.model.view_mut(key);
    let working = view.drag.as_ref().map_or(row.hitbox, |drag| drag.hitbox);
    let at = stage::center();
    let color = theme::ACCENT;
    let (sin, cos) = canvas.camera.yaw.0.sin_cos();
    let right = Offset::<Meters>::new(cos, -sin);
    for level in [0.0, working.height.0 / 2.0, working.height.0] {
        painter.add(egui::Shape::closed_line(
            canvas
                .circle(at, working.radius.0)
                .into_iter()
                .map(|point| point + (canvas.lifted(at, Meters(level)) - canvas.to_screen(at)))
                .collect(),
            Stroke::new(1.0, ui::ink::wash(color, 0.7)),
        ));
    }
    for side in [-1.0, 1.0] {
        let foot = at + right * (working.radius.0 * side);
        painter.line_segment(
            [canvas.to_screen(foot), canvas.lifted(foot, working.height)],
            Stroke::new(1.0, ui::ink::wash(color, 0.7)),
        );
    }
    let radius_handle = canvas.to_screen(at + right * working.radius.0);
    let height_handle = canvas.lifted(at, working.height);
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let hot = pointer.and_then(|pointer| {
        if pointer.distance(radius_handle) <= GRIP {
            Some(Handle::Radius)
        } else if pointer.distance(height_handle) <= GRIP {
            Some(Handle::Height)
        } else {
            None
        }
    });
    let held = view.drag.as_ref().map(|drag| drag.handle);
    stage::handle(
        &painter,
        radius_handle,
        hot == Some(Handle::Radius) || held == Some(Handle::Radius),
        color,
    );
    stage::handle(
        &painter,
        height_handle,
        hot == Some(Handle::Height) || held == Some(Handle::Height),
        color,
    );
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(handle) = hot
    {
        view.drag = Some(Drag {
            handle,
            hitbox: working,
        });
        view.spinning = false;
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = &mut view.drag
    {
        match drag.handle {
            Handle::Radius => {
                if let Some(pointer) = response.interact_pointer_pos()
                    && let Some(ground) = canvas.to_ground(pointer)
                {
                    drag.hitbox.radius = Meters((ground - at).length().max(SMALLEST));
                }
            }
            Handle::Height => {
                let per_metre = canvas.to_screen(at).y - canvas.lifted(at, Meters(1.0)).y;
                if per_metre.abs() > f32::EPSILON {
                    let delta = -response.drag_delta().y / per_metre;
                    drag.hitbox.height = Meters((drag.hitbox.height.0 + delta).max(SMALLEST));
                }
            }
        }
        ui.ctx().request_repaint();
    }
    let mut landed = None;
    if response.drag_stopped()
        && let Some(drag) = view.drag.take()
    {
        landed = Some(drag.hitbox);
    }
    painter.text(
        turntable.rect.left_top() + Vec2::new(8.0, 6.0),
        egui::Align2::LEFT_TOP,
        format!(
            "capsule · {:.2} m across · {:.2} m tall",
            working.radius.0 * 2.0,
            working.height.0
        ),
        egui::FontId::monospace(11.0),
        theme::TEXT,
    );
    turntable.stand(editor, ui, row, idle, None);
    if let Some(hitbox) = landed {
        super::set_hitbox(editor, key, hitbox);
    }
    ui.add_space(4.0);
    form::hint(
        ui,
        "Drag the side handle for the radius and the top handle for the height. The game picks \
         and hits the rig by this capsule.",
    );
}
