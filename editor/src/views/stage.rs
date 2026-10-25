//! A stage for one thing: the game's renderer drawing a flat patch of the home terrain with a
//! prop or a rig on it, into a view's viewport, from an orbit camera that turns on its own until
//! the pointer takes it.

use std::sync::Arc;

use engine::core::content::Content;
use engine::core::geometry::{Degrees, Radians};
use engine::core::math::Pos;
use engine::core::render::camera::{FOV, PITCH};
use engine::core::units::Meters;
use engine::systems::area::{self, Kept, PROP_STAGE, Picture, Pictured};
use ui::egui::{self, Color32, Pos2, Rect, Stroke, Ui};
use ui::theme;

use super::area::camera::OrbitCamera;
use super::area::viewport::{Request, Standing};
use crate::app::Editor;

/// How fast a stage turns on its own.
const TURN: Degrees = Degrees(24.0);
const YAW: Degrees = Degrees(30.0);

/// A picture of the stage, made again when the project's content moves.
#[derive(Default)]
pub struct StagePicture {
    made: Option<(u64, Arc<Picture>)>,
}

impl StagePicture {
    /// The picture of `pictured` as `content` stands at `generation`.
    pub fn of(
        &mut self,
        content: &Content,
        generation: u64,
        pictured: Pictured,
    ) -> Option<(u64, Arc<Picture>)> {
        if let Some((made, picture)) = &self.made
            && *made == generation
        {
            return Some((generation, picture.clone()));
        }
        let picture = Arc::new(area::picture(content, pictured, Kept::All)?);
        self.made = Some((generation, picture.clone()));
        Some((generation, picture))
    }
}

/// The middle of the stage, where what it shows stands.
pub fn center() -> Pos<Meters> {
    PROP_STAGE.bounds().center()
}

/// A camera at the game's pitch, looking at the middle of the stage from far enough to see a
/// thing `radius` metres big.
pub fn framing(radius: f32) -> OrbitCamera {
    OrbitCamera {
        target: center(),
        yaw: YAW.radians(),
        pitch: PITCH.radians(),
        distance: Meters((radius.max(0.5) / (FOV.radians().0 / 2.0).sin()).max(2.5)),
    }
}

/// Turns `camera` on by the frame's `dt` seconds.
pub fn spin(camera: &mut OrbitCamera, dt: f32) {
    camera.yaw = Radians(camera.yaw.0 + TURN.radians().0 * dt);
}

/// Draws the picture last rendered for `view_key` over `rect`, or says it is on its way.
pub fn show_picture(editor: &Editor, ui: &Ui, painter: &egui::Painter, rect: Rect, view_key: &str) {
    match editor.views.area.viewports.shown(view_key) {
        Some(shown) => {
            painter.image(shown.texture, rect, crate::thumbs::FULL_UV, Color32::WHITE);
        }
        None => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "standing the scenery…",
                egui::FontId::proportional(13.0),
                theme::MUTED,
            );
            ui.ctx().request_repaint();
        }
    }
}

/// Asks the viewport `view_key` to draw `picture` with `cast` on it from `camera` at `rect`'s
/// size next frame.
pub fn request(
    editor: &mut Editor,
    ui: &Ui,
    view_key: &str,
    rect: Rect,
    camera: OrbitCamera,
    (made, picture): (u64, Arc<Picture>),
    cast: Vec<Standing>,
) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    let generation = project.generation;
    let pixels_per_point = ui.ctx().pixels_per_point();
    let size = [
        (rect.width() * pixels_per_point).round().max(1.0) as u32,
        (rect.height() * pixels_per_point).round().max(1.0) as u32,
    ];
    editor.views.area.viewports.request(Request {
        key: view_key.to_owned(),
        size,
        camera,
        picture,
        made,
        content,
        generation,
        stroke: None,
        ghost: None,
        cast,
    });
}

/// A small round handle at `at`, filled when `hot`.
pub fn handle(painter: &egui::Painter, at: Pos2, hot: bool, color: Color32) {
    painter.circle_filled(at, 5.0, if hot { color } else { theme::RAISED });
    painter.circle_stroke(at, 5.0, Stroke::new(1.5, color));
}
