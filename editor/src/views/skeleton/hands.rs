//! The hands page: the joints an item hangs from, in order (the first the main hand, the second the
//! off hand), each with a grip frame, a point and axes on its joint with +Y along the palm, and a
//! clasp closing its fingers as a clip's first moment poses them. A hand's joint and its clasp's
//! fingers are clicked on the stage; its grip frame is moved with G and turned with R, kept to one
//! of its axes with X, Y or Z, the stage hanging a chosen item from it in a chosen clip, so the
//! palm is judged with a real handle in it.

use bevy::math::{EulerRot, Mat4, Quat, Vec3};
use engine::core::geometry::Degrees;
use engine::core::units::Meters;
use engine::systems::actor::skeleton::{
    Clasp, Clip, Hand, JointName, LocalOffset, LocalTurn, SkeletonDef,
};
use ui::egui::{self, Pos2, RichText, Stroke, Ui, Vec2};
use ui::{controls, form, panel, theme};

use super::Staged;
use super::rigging::Rigging;
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::ContentEdit;
use crate::tabs::shown_path;

#[derive(Default)]
pub struct HandsState {
    /// What the next joint clicked on the stage sets.
    pub picking: Option<Picking>,
    /// The grip frame being moved or turned with the pointer.
    pub grab: Option<Grab>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picking {
    Joint,
    Fingers,
}

/// A grip frame being moved or turned: how, kept to which of its axes, and the hand and pointer as
/// they were when it began.
#[derive(Clone, Debug)]
pub struct Grab {
    pub turning: bool,
    pub axis: Option<usize>,
    pub from: Option<Pos2>,
    pub hand: Hand,
}

/// How far a pointer's point turns a grip frame.
const TURN_PER_POINT: f32 = 0.5;
/// How long the drawn axes of a grip frame are, in metres.
const AXIS_LENGTH: f32 = 0.15;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let hands = def.hands.clone();
    let chosen = editor
        .views
        .skeleton
        .view(key)
        .hand
        .min(hands.len().saturating_sub(1));
    try_by_digit(editor, ui, key, &content);
    let width = ui.available_width();
    let list_width = (width * 0.36).clamp(240.0, 400.0);
    let [left, right] = panel::split(ui, list_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("hands", key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list(editor, ui, key, rigging, &hands, chosen);
                if let Some(hand) = hands.get(chosen) {
                    ui.add_space(10.0);
                    form::rule(ui);
                    fields(editor, ui, key, rigging, &hands, chosen, hand);
                }
            });
    });
    panel::body(ui, right, |ui| {
        if rigging.rig.is_none() {
            form::hint(ui, "Pick our rig's files on the rigs page to see it here.");
            return;
        }
        super::played_clip(editor, key, def, rigging);
        super::stage_pickers(editor, ui, key, &content, rigging, true);
        super::item_picker(editor, ui, key, &content, "try with");
        let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
        let props = super::tried_in_hand(editor, key, &content, chosen);
        let Some(staged) = super::stage_rig(editor, ui, key, def, rigging, size, props) else {
            return;
        };
        if let Some(hand) = hands.get(chosen) {
            grip(editor, ui, key, &staged, &hands, chosen, hand);
            pick(editor, ui, key, &staged, &hands, chosen, hand);
        }
        form::hint(
            ui,
            "+Y runs along the palm. G moves the grip frame, R turns it, X, Y or Z keep it to one \
             axis; click to keep, Esc to undo. 1 to 9 try the item in that hand.",
        );
    });
}

/// Puts the item on trial in hand n as the digit key n asks.
fn try_by_digit(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &engine::core::content::Content,
) {
    if ui.ctx().text_edit_focused() {
        return;
    }
    let Some(index) = ui.input(|i| EditorKey::Pick.digit(i)) else {
        return;
    };
    let view = editor.views.skeleton.view_mut(key);
    view.hand = index;
    if view.trying.is_none() {
        view.trying = super::hand_items(content)
            .first()
            .map(|(path, _)| path.clone());
    }
}

fn list(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    rigging: &Rigging,
    hands: &[Hand],
    chosen: usize,
) {
    form::section_count(ui, "Hands · in order", hands.len());
    let mut pick = None;
    let mut moved: Option<(usize, usize)> = None;
    let mut removed = None;
    for (index, hand) in hands.iter().enumerate() {
        ui.horizontal(|ui| {
            let role = match index {
                0 => "main",
                1 => "off",
                _ => "",
            };
            if ui
                .add(egui::Button::selectable(
                    index == chosen,
                    format!("{} · {}", hand.name, hand.joint),
                ))
                .clicked()
            {
                pick = Some(index);
            }
            ui.label(theme::faint(role));
            if index > 0 && ui.small_button("↑").on_hover_text("earlier").clicked() {
                moved = Some((index, index - 1));
            }
            if index + 1 < hands.len() && ui.small_button("↓").on_hover_text("later").clicked() {
                moved = Some((index, index + 1));
            }
            if controls::remove(ui, "hand").clicked() {
                removed = Some(index);
            }
        });
    }
    let first_joint = rigging.rig.as_ref().and_then(|rig| {
        rig.joints()
            .next()
            .map(|joint| rig.nodes[joint].name.clone())
    });
    if let Some(joint) = first_joint
        && ui.button("+ Add hand").clicked()
    {
        let mut more = hands.to_vec();
        more.push(Hand {
            name: ["right", "left"].get(hands.len()).map_or_else(
                || format!("hand {}", hands.len() + 1),
                |name| (*name).to_owned(),
            ),
            joint: JointName::new(joint),
            offset: LocalOffset::default(),
            turn: LocalTurn::default(),
            clasp: None,
        });
        if super::set_hands(editor, key, more) {
            pick = Some(hands.len());
            editor.views.skeleton.view_mut(key).hands.picking = Some(Picking::Joint);
        }
    }
    form::hint(
        ui,
        "The first hand is main, the second off; a stance's hand field names one of these. Nothing \
         else on a body takes an item.",
    );
    if let Some((from, to)) = moved {
        let mut order = hands.to_vec();
        order.swap(from, to);
        if super::set_hands(editor, key, order) {
            pick = Some(to);
        }
    }
    if let Some(index) = removed {
        let mut fewer = hands.to_vec();
        fewer.remove(index);
        super::set_hands(editor, key, fewer);
    }
    if let Some(index) = pick {
        editor.views.skeleton.view_mut(key).hand = index;
    }
}

fn fields(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    rigging: &Rigging,
    hands: &[Hand],
    chosen: usize,
    hand: &Hand,
) {
    ui.label(RichText::new(&hand.name).strong());
    let picking = editor.views.skeleton.view(key).hands.picking;
    let mut asked = picking;
    let mut edited = hand.clone();
    egui::Grid::new(("hand-fields", key))
        .num_columns(2)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(theme::muted("name"));
            ui.add(egui::TextEdit::singleline(&mut edited.name).desired_width(140.0));
            ui.end_row();
            ui.label(theme::muted("joint"));
            ui.horizontal(|ui| {
                ui.label(theme::mono(edited.joint.as_str()));
                let on = picking == Some(Picking::Joint);
                if ui
                    .toggle_value(&mut on.clone(), "click it on the stage")
                    .clicked()
                {
                    asked = (!on).then_some(Picking::Joint);
                }
            });
            ui.end_row();
            ui.label(theme::muted("offset · m"));
            ui.horizontal(|ui| {
                for axis in &mut edited.offset.0 {
                    let mut value = axis.0;
                    ui.add(
                        egui::DragValue::new(&mut value)
                            .speed(0.002)
                            .fixed_decimals(3),
                    );
                    *axis = Meters(value);
                }
            });
            ui.end_row();
            ui.label(theme::muted("turn · °"));
            ui.horizontal(|ui| {
                for axis in &mut edited.turn.0 {
                    let mut value = axis.0;
                    ui.add(
                        egui::DragValue::new(&mut value)
                            .speed(0.5)
                            .range(-180.0..=180.0)
                            .fixed_decimals(1),
                    );
                    *axis = Degrees(value);
                }
            });
            ui.end_row();
            let mut clip: Option<Clip> = edited.clasp.as_ref().map(|clasp| clasp.clip.clone());
            ui.label(theme::muted("clasp"));
            super::clip_picker(
                ui,
                "",
                ("hand-clasp", key),
                rigging,
                &mut clip,
                Some("none"),
            );
            ui.end_row();
            if clip.is_some() {
                let fingers = edited
                    .clasp
                    .as_ref()
                    .map_or_else(Vec::new, |clasp| clasp.fingers.clone());
                ui.label(theme::muted("fingers"));
                ui.horizontal_wrapped(|ui| {
                    for finger in &fingers {
                        ui.label(theme::mono(finger.as_str()));
                    }
                    let on = picking == Some(Picking::Fingers);
                    if ui
                        .toggle_value(&mut on.clone(), "click each finger's root on the stage")
                        .clicked()
                    {
                        asked = (!on).then_some(Picking::Fingers);
                    }
                });
                ui.end_row();
                edited.clasp = clip.map(|clip| Clasp { clip, fingers });
            } else {
                edited.clasp = None;
            }
        });
    form::hint(
        ui,
        "The grip frame's offset and turn are in the joint's own axes, so items need no correction \
         of their own beyond their grip. A clasp closes each finger from its root down as its clip \
         turns them at its first moment, while the hand holds an item.",
    );
    editor.views.skeleton.view_mut(key).hands.picking = asked;
    if edited != *hand {
        change_hand(editor, key, hands, chosen, edited);
    }
}

fn change_hand(editor: &mut Editor, key: &str, hands: &[Hand], chosen: usize, hand: Hand) -> bool {
    let name = hands[chosen].name.clone();
    let mut changed = hands.to_vec();
    changed[chosen] = hand;
    super::change_with(
        editor,
        key,
        ContentEdit::new(format!("Change hand {name} in {}", shown_path(key))).merging(),
        |skeleton| {
            skeleton.hands = changed;
            Ok(())
        },
    )
}

/// A joint clicked on the stage: the hand's joint, or a finger's root toggled in its clasp.
fn pick(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    staged: &Staged,
    hands: &[Hand],
    chosen: usize,
    hand: &Hand,
) {
    if editor.views.skeleton.view(key).hands.grab.is_some() {
        return;
    }
    let picking = editor.views.skeleton.view(key).hands.picking;
    let fingers: Vec<&str> = hand
        .clasp
        .iter()
        .flat_map(|clasp| clasp.fingers.iter().map(JointName::as_str))
        .collect();
    let mut lit = vec![hand.joint.as_str()];
    lit.extend(&fingers);
    let Some(clicked) = staged.pick(ui, &lit) else {
        return;
    };
    let mut edited = hand.clone();
    match picking {
        Some(Picking::Joint) => {
            edited.joint = JointName::new(clicked);
            editor.views.skeleton.view_mut(key).hands.picking = None;
        }
        Some(Picking::Fingers) => {
            if let Some(clasp) = &mut edited.clasp {
                let finger = JointName::new(clicked);
                match clasp.fingers.iter().position(|held| *held == finger) {
                    Some(at) => {
                        clasp.fingers.remove(at);
                    }
                    None => clasp.fingers.push(finger),
                }
            }
        }
        None => return,
    }
    change_hand(editor, key, hands, chosen, edited);
}

/// The chosen hand's grip frame drawn on its joint, moved or turned with the pointer once G or R
/// begins it, kept to an axis by X, Y or Z, kept by a click and undone by Esc.
fn grip(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    staged: &Staged,
    hands: &[Hand],
    chosen: usize,
    hand: &Hand,
) {
    let Some(joint) = staged.joint(hand.joint.as_str()) else {
        return;
    };
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
    let state = &mut editor.views.skeleton.view_mut(key).hands;
    if moving || turning {
        state.grab = Some(Grab {
            turning,
            axis: None,
            from: pointer,
            hand: hand.clone(),
        });
    }
    if let (Some(axis), Some(grab)) = (axis, state.grab.as_mut()) {
        grab.axis = (grab.axis != Some(axis)).then_some(axis);
    }
    let Some(grab) = state.grab.clone() else {
        draw_frame(ui, staged, joint, hand, None);
        return;
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
    draw_frame(ui, staged, joint, &edited, grab.axis);
    if cancel {
        editor.views.skeleton.view_mut(key).hands.grab = None;
        change_hand(editor, key, hands, chosen, grab.hand);
        return;
    }
    if keep {
        editor.views.skeleton.view_mut(key).hands.grab = None;
    }
    if edited != *hand {
        change_hand(editor, key, hands, chosen, edited);
    }
    ui.ctx().request_repaint();
}

/// The grip frame of `hand` on its joint standing `joint`: its point, and its axes x red, y green,
/// z blue, the one kept to drawn longer.
fn draw_frame(ui: &Ui, staged: &Staged, joint: Mat4, hand: &Hand, axis: Option<usize>) {
    let frame = joint * held(hand);
    let at = frame.w_axis.truncate();
    let painter = ui.painter_at(staged.rect);
    let origin = staged.project(at);
    let colours = [
        egui::Color32::from_rgb(230, 70, 70),
        egui::Color32::from_rgb(80, 200, 90),
        egui::Color32::from_rgb(80, 130, 240),
    ];
    for (index, colour) in colours.into_iter().enumerate() {
        let length = if axis == Some(index) { 4.0 } else { 1.0 } * AXIS_LENGTH;
        let direction = frame.col(index).truncate().normalize_or_zero();
        let end = staged.project(at + direction * length);
        painter.line_segment([origin, end], Stroke::new(2.0, colour));
    }
    painter.circle_filled(origin, 4.0, theme::ACCENT);
}

/// The move and turn a hand's grip frame makes in its joint's frame.
fn held(hand: &Hand) -> Mat4 {
    let [x, y, z] = hand.turn.array().map(f32::to_radians);
    Mat4::from_rotation_translation(
        Quat::from_euler(EulerRot::XYZ, x, y, z),
        Vec3::from(hand.offset.array()),
    )
}

/// The grip's offset once the pointer has moved `moved` since the grab began: along its kept axis,
/// or across the screen's plane, in the joint's own units.
fn shifted(staged: &Staged, joint: Mat4, grab: &Grab, moved: Vec2) -> LocalOffset {
    let start = Vec3::from(grab.hand.offset.array());
    let at = (joint * held(&grab.hand)).w_axis.truncate();
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
    let at = (joint * held(&grab.hand)).w_axis.truncate();
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
