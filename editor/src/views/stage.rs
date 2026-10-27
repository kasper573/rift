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

use std::collections::BTreeMap;

use engine::systems::actor::Action;
use engine::systems::actor::model::{ActorModel, ModelId};
use engine::systems::actor::render;
use engine::systems::actor::skeleton::Cue;
use engine::systems::actor::stance::HandIndex;
use engine::systems::item::{Grip, ItemId, Wear};

use ui::egui::{Response, Sense, Vec2};

use super::area::camera::{self, AreaCanvas, OrbitCamera};
use super::area::viewport::{Request, StageLayers, Standing, StandingProp};
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

/// What a view's stage keeps from frame to frame: where its camera looks from, whether it turns on
/// its own, and the picture it stands things on.
#[derive(Default)]
pub struct StageState {
    pub camera: OrbitCamera,
    pub spinning: bool,
    picture: StagePicture,
}

impl StageState {
    /// A stage framing a thing `radius` metres big, turning on its own when `spinning`.
    pub fn framing(radius: f32, spinning: bool) -> StageState {
        StageState {
            camera: framing(radius),
            spinning,
            picture: StagePicture::default(),
        }
    }
}

/// The stage drawn this frame with the camera moved as the pointer asks, for a view to draw over
/// and then stand its cast on.
pub struct Turntable {
    pub rect: Rect,
    pub response: Response,
    pub canvas: AreaCanvas,
    view_key: String,
    picture: (u64, Arc<Picture>),
}

/// Draws the stage `view_key` keeps in `state` over `size`, or says why there is nothing to stand
/// on.
pub fn turntable(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    size: Vec2,
    state: &mut StageState,
) -> Option<Turntable> {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let picture = editor.project.as_ref().and_then(|project| {
        let content = project.content.clone()?;
        state
            .picture
            .of(&content, project.generation, Pictured::Stage)
    });
    let Some(picture) = picture else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no terrain to stand the rig on",
            egui::FontId::proportional(13.0),
            theme::MUTED,
        );
        return None;
    };
    show_picture(editor, ui, &painter, rect, view_key);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let busy = camera::navigate(ui, &response, &mut state.camera, rect.height());
    if busy && response.dragged_by(egui::PointerButton::Secondary) {
        state.spinning = false;
    }
    if response.double_clicked() {
        state.camera.target = center();
    }
    if state.spinning {
        spin(&mut state.camera, dt);
        ui.ctx().request_repaint();
    }
    let canvas = AreaCanvas::new(rect, state.camera, picture.1.area.clone());
    Some(Turntable {
        rect,
        response,
        canvas,
        view_key: view_key.to_owned(),
        picture,
    })
}

impl Turntable {
    /// Asks for the picture with `cast` standing on it.
    pub fn stand(self, editor: &mut Editor, ui: &Ui, cast: Vec<Standing>) {
        request(
            editor,
            ui,
            &self.view_key,
            self.rect,
            self.canvas.camera,
            self.picture,
            cast,
        );
    }
}

/// A rig standing in the middle of the stage, in `clip` held at `held`, sounding the cues it
/// `crossed` this frame, holding `props`.
pub fn standing(
    body: engine::core::render::world::body::BodyFiles,
    clip: Option<engine::core::render::world::mannequin::RigClip>,
    held: Option<engine::core::time::Seconds>,
    crossed: Vec<Cue>,
    props: Vec<StandingProp>,
) -> Standing {
    Standing {
        body,
        clip,
        transform: bevy::prelude::Transform::from_translation(
            engine::core::render::camera::world_point(center(), Meters(0.0)),
        ),
        held,
        crossed,
        props,
        layers: None,
    }
}

/// What the layers of a rig standing still and holding props are told: idling, so the clasps of
/// the hands that hold a prop close as they do in the game.
pub fn holding(model: ModelId) -> StageLayers {
    StageLayers {
        model,
        action: Action::Idle,
        stepped: false,
        reduce_motion: false,
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

/// The props `model` holds showing `shown` in its hands, hung as the game hangs them.
pub fn props(
    content: &Content,
    model: &ActorModel,
    shown: &BTreeMap<HandIndex, ItemId>,
) -> Vec<StandingProp> {
    render::hung(content, model, shown)
        .into_iter()
        .filter_map(|(hand, item)| {
            let Some(Wear::Held { grip, .. }) = item.get(content).wear() else {
                return None;
            };
            prop_in_hand(content, model, hand, item, grip)
        })
        .collect()
}

/// The prop of `item` hanging from `hand` of `model` by `grip`, which may be a grip being edited.
pub fn prop_in_hand(
    content: &Content,
    model: &ActorModel,
    hand: HandIndex,
    item: ItemId,
    grip: &Grip,
) -> Option<StandingProp> {
    let held_by = hand;
    let hand = model.hands().get(hand.index())?;
    Some(StandingProp {
        hand: held_by,
        joint: hand.joint.0.clone(),
        model: item.get(content).look.get(content).model.path(),
        transform: render::held_at(model, hand, grip),
    })
}
