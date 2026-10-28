//! The hands page: the joints an item hangs from, in order (the first the main hand, the second the
//! off hand), each with a grip frame (set on the stage, see `grips`) and a clasp closing its fingers
//! as a clip's first moment poses them. A hand's joint and its clasp's fingers are clicked on the
//! stage, which hangs a chosen item from the hand in a chosen clip, so the palm is judged with a
//! real handle in it.

use engine::systems::actor::skeleton::{
    Clasp, Clip, Hand, JointName, LocalOffset, LocalTurn, SkeletonDef,
};
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, panel, theme};

use super::Staged;
use super::rigging::Rigging;
use crate::app::Editor;
use crate::keymap::EditorKey;

#[derive(Default)]
pub struct HandsState {
    /// What the next joint clicked on the stage sets.
    pub picking: Option<Picking>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picking {
    Joint,
    Fingers,
}

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
                    fields(editor, ui, key, rigging, chosen, hand);
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
            pick(editor, ui, key, &staged, chosen, hand);
        }
        form::hint(
            ui,
            "Click a hand's grip frame to set it in the inspector. 1 to 9 try the item in that \
             hand.",
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
        let view = editor.views.skeleton.view_mut(key);
        view.hand = index;
        view.chosen = Some(super::Chosen::Grip(index));
    }
}

fn fields(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    rigging: &Rigging,
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
        "A clasp closes each finger from its root down as its clip turns them at its first moment, \
         while the hand holds an item.",
    );
    editor.views.skeleton.view_mut(key).hands.picking = asked;
    if edited != *hand {
        super::set_hand(editor, key, chosen, edited);
    }
}

/// A joint clicked on the stage: the hand's joint, or a finger's root toggled in its clasp; either
/// keeps the hand's grip frame picked for the inspector.
fn pick(editor: &mut Editor, ui: &Ui, key: &str, staged: &Staged, chosen: usize, hand: &Hand) {
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
    editor.views.skeleton.view_mut(key).chosen = Some(super::Chosen::Grip(chosen));
    super::set_hand(editor, key, chosen, edited);
}
