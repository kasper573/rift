//! The hands page: the joints an item hangs from, in order (the first the main hand, the second the
//! off hand), each moved and turned so +Y runs along its palm; the stage hangs a chosen item from
//! the chosen hand in a chosen clip, so the palm is judged with a real handle in it.

use engine::core::content::Content;
use engine::core::geometry::Degrees;
use engine::core::units::Meters;
use engine::systems::actor::skeleton::{
    BuiltSkeleton, Clasp, ClipName, Hand, JointName, LocalOffset, LocalTurn,
};
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, panel, theme};

use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::ContentEdit;
use crate::tabs::shown_path;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, built: &BuiltSkeleton) {
    let hands = built.def.hands.clone();
    let chosen = editor
        .views
        .skeleton
        .view(key)
        .hand
        .min(hands.len().saturating_sub(1));
    try_by_digit(editor, ui, key, content);
    let width = ui.available_width();
    let list_width = (width * 0.36).clamp(240.0, 380.0);
    let [left, right] = panel::split(ui, list_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("hands", key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list(editor, ui, key, built, &hands, chosen);
                if let Some(hand) = hands.get(chosen) {
                    ui.add_space(10.0);
                    form::rule(ui);
                    fields(editor, ui, key, built, &hands, chosen, hand);
                }
            });
    });
    panel::body(ui, right, |ui| {
        super::played_clip(editor, key, built);
        super::stage_pickers(editor, ui, key, content, built, false);
        super::item_picker(editor, ui, key, content, "try with");
        let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
        let props = super::tried_in_hand(editor, key, content, chosen);
        super::stage_rig(editor, ui, key, content, built, size, props);
        form::hint(
            ui,
            "+Y runs along the palm. 1 to 9 try the item in that hand.",
        );
    });
}

/// Puts the item on trial in hand n as the digit key n asks.
fn try_by_digit(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content) {
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
    built: &BuiltSkeleton,
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
    let free = built
        .rig
        .joints()
        .map(|joint| built.rig.nodes[joint].name.clone())
        .find(|name| hands.iter().all(|hand| hand.joint.0 != *name));
    if let Some(joint) = free
        && ui.button("+ Add hand").clicked()
    {
        let mut more = hands.to_vec();
        more.push(Hand {
            name: format!("hand {}", hands.len() + 1),
            joint: JointName::new(joint),
            offset: LocalOffset::default(),
            turn: LocalTurn::default(),
            clasp: None,
        });
        if super::set_hands(editor, key, more) {
            pick = Some(hands.len());
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
    built: &BuiltSkeleton,
    hands: &[Hand],
    chosen: usize,
    hand: &Hand,
) {
    ui.label(RichText::new(&hand.name).strong());
    let mut edited = hand.clone();
    egui::Grid::new(("hand-fields", key))
        .num_columns(2)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(theme::muted("name"));
            ui.add(egui::TextEdit::singleline(&mut edited.name).desired_width(140.0));
            ui.end_row();
            ui.label(theme::muted("joint"));
            controls::combo(("hand-joint", key))
                .selected_text(edited.joint.as_str())
                .width(160.0)
                .show_ui(ui, |ui| {
                    for joint in built.rig.joints() {
                        let name = &built.rig.nodes[joint].name;
                        ui.selectable_value(&mut edited.joint, JointName::new(name.clone()), name);
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
            clasp_fields(ui, key, built, &hand.joint, &mut edited.clasp);
        });
    form::hint(
        ui,
        "Offset and turn normalise the joint so +Y runs along the palm. Items then need no \
         correction of their own beyond their grip. A clasp closes the fingers as a clip's first \
         moment poses them, while the hand holds an item.",
    );
    if edited != *hand {
        let mut changed = hands.to_vec();
        changed[chosen] = edited;
        super::change_with(
            editor,
            key,
            ContentEdit::new(format!("Change hand {} in {}", hand.name, shown_path(key))).merging(),
            |skeleton| {
                skeleton.hands = changed;
                Ok(())
            },
        );
    }
}

/// The clip a hand's fingers close as, and the joints at the root of each finger, picked among the
/// joints just under the hand's.
fn clasp_fields(
    ui: &mut Ui,
    key: &str,
    built: &BuiltSkeleton,
    hand: &JointName,
    clasp: &mut Option<Clasp>,
) {
    let mut clip: Option<ClipName> = clasp.as_ref().map(|clasp| clasp.clip.clone());
    ui.label(theme::muted("clasp"));
    controls::combo(("hand-clasp", key))
        .selected_text(clip.as_ref().map_or("none", ClipName::as_str))
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut clip, None, "none");
            for name in built.clips.keys() {
                ui.selectable_value(&mut clip, Some(name.clone()), name.as_str());
            }
        });
    ui.end_row();
    let rig = &built.rig;
    let under: Vec<JointName> = rig
        .named(hand.as_str())
        .map(|hand| {
            rig.joints()
                .filter(|&joint| rig.nodes[joint].parent == Some(hand))
                .map(|joint| JointName::new(rig.nodes[joint].name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut fingers: Vec<JointName> = clasp
        .as_ref()
        .map_or_else(|| under.clone(), |clasp| clasp.fingers.clone());
    if clip.is_some() {
        ui.label(theme::muted("fingers"));
        ui.horizontal_wrapped(|ui| {
            for joint in &under {
                let mut on = fingers.contains(joint);
                if ui.toggle_value(&mut on, joint.as_str()).changed() {
                    if on {
                        fingers.push(joint.clone());
                    } else {
                        fingers.retain(|finger| finger != joint);
                    }
                }
            }
            if under.is_empty() {
                ui.label(theme::faint("no joints under the hand"));
            }
        });
        ui.end_row();
    }
    *clasp = clip.map(|clip| Clasp { clip, fingers });
}
