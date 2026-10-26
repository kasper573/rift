//! The joints page: the rig's joint tree with its hands and group roots marked, and the stage
//! showing the rig as a model wearing it. A joint clicked is lit on the stage; right-clicking it, or
//! H and G, makes it a hand or the root of a group.

use std::collections::BTreeMap;

use bevy::prelude::{Transform, Vec3};
use engine::core::content::Content;
use engine::core::content::gltf::NodeTree;
use engine::core::render::camera::{ground_pos, world_point};
use engine::core::units::Meters;
use engine::systems::actor::skeleton::{BuiltSkeleton, GroupName};
use ui::egui::{self, RichText, Stroke, Ui, Vec2};
use ui::{form, panel, theme};

use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::views::stage;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, built: &BuiltSkeleton) {
    keys(editor, ui, key, built);
    let width = ui.available_width();
    let tree_width = (width * 0.36).clamp(220.0, 380.0);
    let [left, right] = panel::split(ui, tree_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("joints", key))
            .auto_shrink([false, false])
            .show(ui, |ui| tree(editor, ui, key, built));
    });
    panel::body(ui, right, |ui| {
        super::stage_pickers(editor, ui, key, content, built, true);
        let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
        let lit = editor.views.skeleton.view(key).joint.clone();
        let at_rest = editor.views.skeleton.view(key).clip.is_none();
        if let Some((rect, canvas)) =
            super::stage_rig(editor, ui, key, content, built, size, Vec::new())
            && at_rest
            && let Some(joint) = lit.as_deref().and_then(|name| built.rig.named(name))
        {
            let at = rest_position(&built.rig, joint);
            let ground = ground_pos(at);
            let lift = at.y - canvas.area().heights.height_at(ground).0;
            let point = canvas.lifted(ground, Meters(lift));
            let painter = ui.painter_at(rect);
            painter.circle_stroke(point, 7.0, Stroke::new(2.0, theme::ACCENT));
            painter.circle_filled(point, 2.5, theme::ACCENT);
        }
        form::hint(
            ui,
            "Click a joint to light it on the stage at rest. Right-click, or H and G: make it a \
                 hand or the root of a group.",
        );
    });
}

/// Makes the lit joint a hand on H, the root of a group named after it on G.
fn keys(editor: &mut Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton) {
    if ui.ctx().text_edit_focused() {
        return;
    }
    let Some(joint) = editor.views.skeleton.view(key).joint.clone() else {
        return;
    };
    let (hand, group) = ui.input_mut(|i| {
        (
            EditorKey::MakeHand.consume(i),
            EditorKey::GroupRoot.consume(i),
        )
    });
    if hand {
        super::make_hand(editor, key, &joint, &hand_name(built, &joint));
    }
    if group {
        super::group_at(editor, key, &joint, &joint);
    }
}

fn tree(editor: &mut Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton) {
    let rig = &built.rig;
    let def = &built.def;
    let roots: BTreeMap<&str, Vec<&GroupName>> =
        def.groups
            .iter()
            .fold(BTreeMap::new(), |mut roots, (group, joints)| {
                for joint in joints {
                    roots.entry(joint.as_str()).or_default().push(group);
                }
                roots
            });
    form::section_count(ui, "Joints", rig.joints().count());
    let lit = editor.views.skeleton.view(key).joint.clone();
    let mut clicked: Option<String> = None;
    let mut hand: Option<String> = None;
    let mut grouped: Option<(String, String)> = None;
    for joint in depth_first(rig) {
        let name = &rig.nodes[joint].name;
        let depth = rig.path(joint).len().saturating_sub(1);
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 10.0);
            let selected = lit.as_deref() == Some(name.as_str());
            let response = ui.add(egui::Button::selectable(
                selected,
                theme::mono(name.as_str()).size(12.5),
            ));
            if let Some((index, held)) = def
                .hands
                .iter()
                .enumerate()
                .find(|(_, held)| held.joint.0 == *name)
            {
                let role = match index {
                    0 => " · main",
                    1 => " · off",
                    _ => "",
                };
                ui.label(
                    RichText::new(format!("hand {}{role}", held.name))
                        .size(12.0)
                        .color(theme::ACCENT),
                );
            }
            for group in roots.get(name.as_str()).into_iter().flatten() {
                ui.label(
                    RichText::new(format!("group {group}"))
                        .size(12.0)
                        .color(theme::OK),
                );
            }
            if response.clicked() {
                clicked = Some(name.clone());
            }
            response.context_menu(|ui| {
                if ui.button("Make it a hand").clicked() {
                    hand = Some(name.clone());
                    ui.close();
                }
                for group in def.groups.keys() {
                    if ui.button(format!("Add it as a root of {group}")).clicked() {
                        grouped = Some((name.clone(), group.0.clone()));
                        ui.close();
                    }
                }
                if ui.button("Start a group here").clicked() {
                    grouped = Some((name.clone(), name.clone()));
                    ui.close();
                }
            });
        });
    }
    let helpers = rig.nodes.len() - rig.joints().count();
    if helpers > 0 {
        ui.label(theme::faint(format!(
            "{helpers} nodes that are not joints: meshes and the scene's root"
        )));
    }
    if let Some(name) = clicked {
        let view = editor.views.skeleton.view_mut(key);
        view.joint = (view.joint.as_ref() != Some(&name)).then_some(name);
    }
    if let Some(joint) = hand {
        super::make_hand(editor, key, &joint, &hand_name(built, &joint));
    }
    if let Some((joint, group)) = grouped {
        super::group_at(editor, key, &joint, &group);
    }
}

/// The rig's joints as a tree reads, each under its parent, siblings in the file's order.
fn depth_first(rig: &NodeTree) -> Vec<usize> {
    let joints: Vec<usize> = rig.joints().collect();
    let mut order = Vec::with_capacity(joints.len());
    let mut open: Vec<usize> = joints
        .iter()
        .copied()
        .filter(|&joint| {
            rig.nodes[joint]
                .parent
                .is_none_or(|parent| !joints.contains(&parent))
        })
        .rev()
        .collect();
    while let Some(joint) = open.pop() {
        order.push(joint);
        open.extend(
            joints
                .iter()
                .copied()
                .filter(|&child| rig.nodes[child].parent == Some(joint))
                .rev(),
        );
    }
    order
}

/// A name for a new hand on `joint` no hand has yet: right, then left, then the joint's own.
fn hand_name(built: &BuiltSkeleton, joint: &str) -> String {
    let taken = |name: &str| built.def.hands.iter().any(|hand| hand.name == name);
    ["right", "left"]
        .into_iter()
        .find(|name| !taken(name))
        .map_or_else(|| joint.to_owned(), str::to_owned)
}

/// Where `joint` stands at rest on the stage, the rig standing in its middle.
fn rest_position(rig: &NodeTree, joint: usize) -> Vec3 {
    let mut global = Transform::IDENTITY;
    let mut at = Some(joint);
    while let Some(node) = at {
        let rest = rig.nodes[node].rest;
        let local = Transform {
            translation: rest.translation,
            rotation: rest.rotation,
            scale: rest.scale,
        };
        global = local.mul_transform(global);
        at = rig.nodes[node].parent;
    }
    world_point(stage::center(), Meters(0.0)) + global.translation
}
