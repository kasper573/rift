//! The hands' grip frames on the skeleton view's stage, whatever its page: each a point and axes on
//! its hand's joint, laid out as every skeleton's are (see `Hand`) so an item's grip holds in any
//! hand. A click on a frame picks its hand for the inspector to set by
//! number; the picked one is moved with G and turned with R, kept to one of its axes with X, Y or Z,
//! kept by a click and undone by Esc.

use bevy::math::{EulerRot, Mat4, Quat, Vec3};
use engine::core::geometry::Degrees;
use engine::core::units::Meters;
use engine::systems::actor::render;
use engine::systems::actor::skeleton::{Hand, LocalOffset, LocalTurn};
use ui::egui::{self, Pos2, Stroke, Ui, Vec2};
use ui::{form, theme};

use super::Staged;
use crate::app::Editor;
use crate::keymap::EditorKey;

/// The grip frame being moved or turned on a stage, or set by a number field being dragged, held
/// here until it is kept so the file is written once rather than on every frame of the move.
#[derive(Default)]
pub struct Grips {
    grab: Option<Grab>,
    dragged: Option<(usize, Hand)>,
}

impl Grips {
    /// `hands` with the one being moved, turned or dragged as it stands so far.
    pub fn working(&self, hands: &[Hand]) -> Vec<Hand> {
        let mut working = hands.to_vec();
        let edited = match (&self.grab, &self.dragged) {
            (Some(grab), _) => Some((grab.index, &grab.edited)),
            (None, Some((index, hand))) => Some((*index, hand)),
            (None, None) => None,
        };
        if let Some((index, hand)) = edited
            && let Some(at) = working.get_mut(index)
        {
            *at = hand.clone();
        }
        working
    }
}

/// A grip frame being moved or turned: how, kept to which of its axes, the hand and pointer as they
/// were when it began, and the hand as the pointer has moved it since.
#[derive(Clone, Debug)]
struct Grab {
    turning: bool,
    axis: Option<usize>,
    from: Option<Pos2>,
    index: usize,
    hand: Hand,
    edited: Hand,
}

/// A hand's grip frame as its number fields leave it: still being dragged, or settled.
enum Edited {
    Dragging(Hand),
    Settled(Hand),
}

/// What the pointer did to the grip frames this frame: picked a hand's, or took the click to keep a
/// move or turn.
#[derive(Default)]
pub struct Touched {
    pub picked: Option<usize>,
    pub claims_click: bool,
}

/// How near the pointer, in points, takes a grip frame.
const REACH: f32 = 8.0;
/// How far a pointer's point turns a grip frame.
const TURN_PER_POINT: f32 = 0.5;
/// How long the drawn axes of a grip frame are, in metres.
const AXIS_LENGTH: f32 = 0.15;

/// Draws every hand's grip frame over `staged` as `hands` stand, the `chosen` hand's moved or
/// turned as the keys and pointer ask, and gives what the pointer did to them.
pub(super) fn overlay(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    staged: &Staged,
    hands: &[Hand],
    chosen: Option<usize>,
) -> Touched {
    let mut touched = Touched::default();
    let shown = editor.views.toggles.grips;
    let grips = &mut editor.views.skeleton.view_mut(key).grips;
    if !shown {
        grips.grab = None;
        grips.dragged = None;
        return touched;
    }
    if grips
        .grab
        .as_ref()
        .is_some_and(|grab| Some(grab.index) != chosen)
    {
        grips.grab = None;
    }
    let mut origins = Vec::new();
    for (index, hand) in hands.iter().enumerate() {
        if Some(index) == chosen {
            continue;
        }
        if let Some(joint) = staged.joint(hand.joint.as_str()) {
            origins.push((index, draw_frame(ui, staged, joint, hand, None, false)));
        }
    }
    if let Some(index) = chosen
        && let Some(hand) = hands.get(index)
        && let Some(joint) = staged.joint(hand.joint.as_str())
    {
        let (origin, keeps) = grab(editor, ui, key, staged, joint, index, hand);
        origins.push((index, origin));
        touched.claims_click = keeps;
    }
    if staged.response.clicked()
        && !touched.claims_click
        && let Some(pointer) = staged.response.interact_pointer_pos()
    {
        touched.picked = origins
            .iter()
            .map(|(index, origin)| (*index, origin.distance(pointer)))
            .filter(|(_, distance)| *distance <= REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index);
    }
    touched
}

/// The grip frame of the hand at `index` among `hands` as number fields, as the view's stage shows
/// it, holding what a drag on one leaves until the drag lets go; gives the hand once it is settled.
pub fn fields(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    index: usize,
    hands: &[Hand],
) -> Option<Hand> {
    let working = editor.views.skeleton.view(key).grips.working(hands);
    let edited = drag_fields(ui, working.get(index)?);
    let grips = &mut editor.views.skeleton.view_mut(key).grips;
    match edited? {
        Edited::Dragging(hand) => {
            grips.dragged = Some((index, hand));
            None
        }
        Edited::Settled(hand) => {
            grips.dragged = None;
            Some(hand)
        }
    }
}

fn drag_fields(ui: &mut Ui, hand: &Hand) -> Option<Edited> {
    let mut edited = hand.clone();
    let mut responses = Vec::new();
    form::kv(ui, "offset · m", |ui| {
        for axis in &mut edited.offset.0 {
            responses.push(
                ui.add(
                    egui::DragValue::new(&mut axis.0)
                        .speed(0.002)
                        .fixed_decimals(3),
                ),
            );
        }
    });
    form::kv(ui, "turn · °", |ui| {
        for axis in &mut edited.turn.0 {
            responses.push(
                ui.add(
                    egui::DragValue::new(&mut axis.0)
                        .speed(0.5)
                        .range(-180.0..=180.0)
                        .fixed_decimals(1),
                ),
            );
        }
    });
    form::hint(
        ui,
        "In the closed fist, +Y runs along the handle out of the thumb's side and +Z out of the back \
         of the hand. G moves the grip frame on the stage, R \
         turns it, X, Y or Z keep it to one axis; click to keep, Esc to undo.",
    );
    if responses.iter().any(egui::Response::drag_stopped) {
        Some(Edited::Settled(edited))
    } else if responses.iter().any(egui::Response::dragged) {
        Some(Edited::Dragging(edited))
    } else {
        (edited != *hand).then_some(Edited::Settled(edited))
    }
}

/// The chosen hand's grip frame drawn on its joint, moved or turned with the pointer once G or R
/// begins it; gives where its point shows and whether this frame's click keeps the move.
fn grab(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    staged: &Staged,
    joint: Mat4,
    index: usize,
    hand: &Hand,
) -> (Pos2, bool) {
    let typing = ui.ctx().text_edit_focused();
    let (moving, turning, axis) = if typing {
        (false, false, None)
    } else {
        ui.input_mut(|i| {
            let axis = [EditorKey::AxisX, EditorKey::AxisY, EditorKey::AxisZ]
                .iter()
                .position(|key| key.consume(i));
            (
                EditorKey::GripMove.consume(i),
                EditorKey::GripTurn.consume(i),
                axis,
            )
        })
    };
    let pointer = staged
        .response
        .hover_pos()
        .or(ui.input(|i| i.pointer.latest_pos()));
    let grips = &mut editor.views.skeleton.view_mut(key).grips;
    if moving || turning {
        grips.grab = Some(Grab {
            turning,
            axis: None,
            from: pointer,
            index,
            hand: hand.clone(),
            edited: hand.clone(),
        });
    }
    if let (Some(axis), Some(grab)) = (axis, grips.grab.as_mut()) {
        grab.axis = (grab.axis != Some(axis)).then_some(axis);
    }
    let Some(grab) = grips.grab.clone() else {
        return (draw_frame(ui, staged, joint, hand, None, true), false);
    };
    let (cancel, keep) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Escape) || i.pointer.secondary_clicked(),
            i.pointer.primary_clicked() || i.key_pressed(egui::Key::Enter),
        )
    });
    let moved = match (pointer, grab.from) {
        (Some(at), Some(from)) => at - from,
        _ => Vec2::ZERO,
    };
    let mut edited = grab.hand.clone();
    if grab.turning {
        edited.turn = turned(staged, joint, &grab, moved);
    } else {
        edited.offset = shifted(staged, joint, &grab, moved);
    }
    let origin = draw_frame(ui, staged, joint, &edited, grab.axis, true);
    let grips = &mut editor.views.skeleton.view_mut(key).grips;
    if cancel {
        grips.grab = None;
        return (origin, false);
    }
    if keep {
        grips.grab = None;
        super::set_hand(editor, key, index, edited);
    } else if let Some(grab) = &mut grips.grab {
        grab.edited = edited;
    }
    ui.ctx().request_repaint();
    (origin, keep)
}

/// The grip frame of `hand` on its joint standing `joint`: its point, and its axes x red, y green,
/// z blue, the one kept to drawn longer, bold when `chosen`. Gives where its point shows.
fn draw_frame(
    ui: &Ui,
    staged: &Staged,
    joint: Mat4,
    hand: &Hand,
    axis: Option<usize>,
    chosen: bool,
) -> Pos2 {
    let frame = joint * render::grip_frame(hand);
    let at = frame.w_axis.truncate();
    let painter = ui.painter_at(staged.rect);
    let origin = staged.project(at);
    let colours = [
        egui::Color32::from_rgb(230, 70, 70),
        egui::Color32::from_rgb(80, 200, 90),
        egui::Color32::from_rgb(80, 130, 240),
    ];
    let (width, alpha) = if chosen { (2.0, 1.0) } else { (1.0, 0.5) };
    for (index, colour) in colours.into_iter().enumerate() {
        let length = if axis == Some(index) { 4.0 } else { 1.0 } * AXIS_LENGTH;
        let direction = frame.col(index).truncate().normalize_or_zero();
        let end = staged.project(at + direction * length);
        painter.line_segment(
            [origin, end],
            Stroke::new(width, ui::ink::wash(colour, alpha)),
        );
    }
    if chosen {
        painter.circle_filled(origin, 4.0, theme::ACCENT);
    } else {
        painter.circle_stroke(origin, 3.5, Stroke::new(1.5, theme::ACCENT));
    }
    origin
}

/// The grip's offset once the pointer has moved `moved` since the grab began: along its kept axis,
/// or across the screen's plane, in the joint's own units.
fn shifted(staged: &Staged, joint: Mat4, grab: &Grab, moved: Vec2) -> LocalOffset {
    let start = Vec3::from(grab.hand.offset.array());
    let at = (joint * render::grip_frame(&grab.hand)).w_axis.truncate();
    let origin = staged.project(at);
    let along = |axis: usize| {
        let step = joint.col(axis).truncate() * 0.1;
        (staged.project(at + step) - origin) / 0.1
    };
    let axes: Vec<usize> = match grab.axis {
        Some(axis) => vec![axis],
        None => vec![0, 1, 2],
    };
    let screen: Vec<Vec2> = axes.iter().map(|&axis| along(axis)).collect();
    let local = least_move(&screen, moved);
    let mut offset = start;
    for (index, &axis) in axes.iter().enumerate() {
        offset[axis] += local[index];
    }
    LocalOffset(offset.to_array().map(Meters))
}

/// The smallest move along the axes whose screen moves per metre are `screen` that moves the screen
/// by `moved`.
fn least_move(screen: &[Vec2], moved: Vec2) -> Vec<f32> {
    let gram = screen.iter().fold([[0.0f32; 2]; 2], |mut gram, axis| {
        gram[0][0] += axis.x * axis.x;
        gram[0][1] += axis.x * axis.y;
        gram[1][0] += axis.y * axis.x;
        gram[1][1] += axis.y * axis.y;
        gram
    });
    let det = gram[0][0] * gram[1][1] - gram[0][1] * gram[1][0];
    let solved = if det.abs() > 1.0e-6 {
        Vec2::new(
            (gram[1][1] * moved.x - gram[0][1] * moved.y) / det,
            (-gram[1][0] * moved.x + gram[0][0] * moved.y) / det,
        )
    } else {
        let length = gram[0][0] + gram[1][1];
        if length < 1.0e-6 {
            return vec![0.0; screen.len()];
        }
        moved / length
    };
    screen.iter().map(|axis| axis.dot(solved)).collect()
}

/// The grip's turn once the pointer has moved `moved` since the grab began: about its kept axis, or
/// about the one facing the camera most.
fn turned(staged: &Staged, joint: Mat4, grab: &Grab, moved: Vec2) -> LocalTurn {
    let at = (joint * render::grip_frame(&grab.hand)).w_axis.truncate();
    let origin = staged.project(at);
    let axis = grab.axis.unwrap_or_else(|| {
        (0..3)
            .min_by(|&a, &b| {
                let on_screen = |axis: usize| {
                    (staged.project(at + joint.col(axis).truncate().normalize_or_zero() * 0.1)
                        - origin)
                        .length()
                };
                on_screen(a).total_cmp(&on_screen(b))
            })
            .unwrap_or(1)
    });
    let angle = (moved.x - moved.y) * TURN_PER_POINT;
    let [x, y, z] = grab.hand.turn.array().map(f32::to_radians);
    let start = Quat::from_euler(EulerRot::XYZ, x, y, z);
    let about = [Vec3::X, Vec3::Y, Vec3::Z][axis];
    let (x, y, z) =
        (start * Quat::from_axis_angle(about, angle.to_radians())).to_euler(EulerRot::XYZ);
    LocalTurn([x, y, z].map(|radians| Degrees((radians.to_degrees() * 10.0).round() / 10.0)))
}
