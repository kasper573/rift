//! The skeleton's inspector: its budgets, libraries and who wears it, then what the page in front
//! is about: the lit joint, the table's health and the moves no skeleton declares, the chosen clip,
//! hand or layer, the parts and materials made on it.

use std::collections::BTreeSet;

use engine::core::assets::FileSize;
use engine::core::content::Content;
use engine::core::script::node::Node;
use engine::systems::actor::playing::Animate;
use engine::systems::actor::skeleton::{
    BuiltSkeleton, MAX_JOINTS, MAX_LIBRARIES_BYTES, MoveName, SkeletonDef,
};
use engine::systems::actor::{Action, MaterialDef, ModelPartDef, Motion};
use engine::systems::script::{RiftEffect, ScriptDef};
use strum::VariantArray;
use ui::egui::Ui;
use ui::{form, panel, theme};

use super::SkeletonPage;
use crate::app::Editor;
use crate::tabs::shown_path;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &SkeletonDef, page: SkeletonPage) {
    let Some((content, id, built)) = super::built(editor, key) else {
        panel::card(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.colored_label(
                theme::DANGER,
                "the skeleton did not load: its problems are in the output panel",
            );
        });
        return;
    };
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Rig", "", |_| {});
        ui.add_space(4.0);
        rig(editor, ui, &content, id, &built);
    });
    ui.add_space(8.0);
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match page {
            SkeletonPage::Joints => joint(editor, ui, key, &built),
            SkeletonPage::Sets => sets(editor, ui, &content, row, &built),
            SkeletonPage::Clips => clip(editor, ui, key, &built),
            SkeletonPage::Hands => hand(editor, ui, key, &content, id, &built),
            SkeletonPage::Body => body(ui, &content, id),
            SkeletonPage::Layers => layer(editor, ui, key, &built),
        }
    });
    ui.add_space(8.0);
    crate::shell::row_form::usages::show_usages_grouped(editor, ui, key, true);
}

fn rig(
    editor: &Editor,
    ui: &mut Ui,
    content: &Content,
    id: engine::systems::actor::skeleton::SkeletonId,
    built: &BuiltSkeleton,
) {
    let joints = built.rig.joints().count();
    form::kv(ui, "joints", |ui| {
        let ink = if joints > MAX_JOINTS {
            theme::DANGER
        } else {
            theme::TEXT
        };
        ui.colored_label(ink, format!("{joints} of {MAX_JOINTS}"));
    });
    let weight = FileSize(
        built
            .def
            .libraries
            .iter()
            .filter_map(|library| super::weight(editor, &library.file.path()))
            .map(|weight| weight.0)
            .sum(),
    );
    form::kv(ui, "libraries", |ui| {
        let ink = if weight > MAX_LIBRARIES_BYTES {
            theme::DANGER
        } else {
            theme::TEXT
        };
        ui.colored_label(
            ink,
            format!(
                "{} · {weight} of {MAX_LIBRARIES_BYTES}",
                built.def.libraries.len()
            ),
        );
    });
    form::kv_text(ui, "clips", built.clips.len().to_string());
    let wearers = super::wearers(content, id);
    let unfit: Vec<&str> = wearers
        .iter()
        .filter(|(_, model)| model.is_none())
        .map(|(path, _)| shown_path(path))
        .collect();
    form::kv_text(
        ui,
        "worn by",
        match (wearers.len(), unfit.len()) {
            (0, _) => "no model: nothing shows it".to_owned(),
            (count, 0) => format!("{count} models, all fitting"),
            (count, _) => format!("{count} models · {} do not fit", unfit.len()),
        },
    );
    for path in unfit {
        ui.colored_label(theme::DANGER, format!("{path} does not fit"));
    }
}

/// What the skeleton's bodies are made of: its parts, and the materials made on it with the tints
/// each paints.
fn body(ui: &mut Ui, content: &Content, id: engine::systems::actor::skeleton::SkeletonId) {
    panel::header(ui, "Body", "", |_| {});
    let parts = content
        .table::<ModelPartDef>()
        .iter()
        .filter(|(_, part)| part.skeleton == id)
        .count();
    form::kv_text(ui, "parts", parts.to_string());
    let materials: Vec<_> = content
        .table::<MaterialDef>()
        .iter()
        .filter(|(_, material)| material.skeleton == id)
        .collect();
    if materials.is_empty() {
        form::kv_text(
            ui,
            "materials",
            "none: every part is drawn in its own texture",
        );
    }
    let tints = id.get(content).tints.len();
    for (material, def) in materials {
        let painted: usize = def
            .masks
            .values()
            .map(|channels| channels.painting().count())
            .sum();
        form::kv_code(ui, "material", shown_path(material.path(content)));
        form::kv_text(ui, "paints", format!("{painted} of {tints} tints"));
    }
}

fn joint(editor: &Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton) {
    panel::header(ui, "Joint", "", |_| {});
    let Some(name) = editor.views.skeleton.view(key).joint.clone() else {
        form::hint(ui, "Click a joint in the tree to see it here.");
        return;
    };
    let Some(joint) = built.rig.named(&name) else {
        return;
    };
    form::kv_code(ui, "name", &name);
    form::kv_code(ui, "under", built.rig.parent_name(joint));
    let below = built
        .rig
        .joints()
        .filter(|&other| other != joint && built.rig.descends(other, joint))
        .count();
    form::kv_text(ui, "joints below", below.to_string());
    if let Some(hand) = built.def.hands.iter().find(|hand| hand.joint.0 == name) {
        form::kv_text(ui, "hand", &hand.name);
    }
    let groups: Vec<&str> = built
        .def
        .groups
        .iter()
        .filter(|(_, roots)| {
            roots.iter().any(|root| {
                built
                    .rig
                    .named(root.as_str())
                    .is_some_and(|root| built.rig.descends(joint, root))
            })
        })
        .map(|(group, _)| group.as_str())
        .collect();
    if !groups.is_empty() {
        form::kv_text(ui, "in group", groups.join(", "));
    }
}

fn sets(editor: &Editor, ui: &mut Ui, content: &Content, row: &SkeletonDef, built: &BuiltSkeleton) {
    panel::header(ui, "Sets", "", |_| {});
    let def = &built.def;
    let moves: BTreeSet<&MoveName> = def.moves().collect();
    form::kv_text(ui, "stances", def.stances.len().to_string());
    form::kv_text(ui, "moves", moves.len().to_string());
    form::kv_text(
        ui,
        "engine actions",
        format!(
            "{} of {} in base",
            Action::VARIANTS
                .iter()
                .filter(|action| row.base.clips.contains_key(action))
                .count(),
            Action::VARIANTS.len()
        ),
    );
    let lacking: Vec<String> = def
        .sets()
        .flat_map(|(_, set)| set.attacks().map(|(_, clip)| clip.clip().clone()))
        .filter(|clip| {
            !def.cues
                .get(clip)
                .is_some_and(|cues| cues.iter().any(|cue| cue.apex))
        })
        .filter(|clip| built.clips.contains_key(clip))
        .map(|clip| clip.0)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    if lacking.is_empty() {
        form::kv_text(ui, "apex", "every attack and move clip has one");
    } else {
        ui.colored_label(
            theme::DANGER,
            format!("no apex on {}: a hit must land", lacking.join(", ")),
        );
    }
    let declared = crate::body_names::declarations(content);
    for (script, name) in asked_moves(editor, content) {
        if declared.contains_key(&name) {
            continue;
        }
        ui.colored_label(
            theme::DANGER,
            format!(
                "{} asks for {name}, which no skeleton's set declares: the script is refused \
                 until one does",
                shown_path(&script)
            ),
        );
    }
    form::hint(
        ui,
        "A move a skill asks for and no skeleton declares is shown on every skeleton's sets page. \
         Once any skeleton declares it, the others play their stance's attack.",
    );
}

/// Every move a script asks an animate node to play, with the script's path, as its file reads.
pub fn asked_moves(editor: &Editor, content: &Content) -> Vec<(String, MoveName)> {
    let mut asked = Vec::new();
    for path in content.table::<ScriptDef>().paths() {
        let Some(Ok(script)) = editor.read::<ScriptDef>(path) else {
            continue;
        };
        for node in script.nodes.values() {
            if let Node::Effect(RiftEffect::Animate(Animate {
                motion: Motion::Move(name),
                ..
            })) = node
            {
                asked.push((path.clone(), name.clone()));
            }
        }
    }
    asked
}

fn clip(editor: &Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton) {
    panel::header(ui, "Clip", "", |_| {});
    let unused = super::clips::unused(built).len();
    let Some(name) = editor.views.skeleton.view(key).clip.clone() else {
        return;
    };
    let Some(found) = built.clips.get(&name) else {
        return;
    };
    form::kv_code(ui, "name", name.as_str());
    form::kv_code(
        ui,
        "library",
        format!("clips/{}.glb", built.def.libraries[found.library].key),
    );
    form::kv_text(ui, "length", format!("{:.2} s", found.length.0));
    let cues = built.def.cues.get(&name).map_or(&[][..], Vec::as_slice);
    form::kv_text(
        ui,
        "cues",
        format!(
            "{} · {} apex · {} steps",
            cues.len(),
            cues.iter().filter(|cue| cue.apex).count(),
            cues.iter().filter(|cue| cue.step).count()
        ),
    );
    form::kv_text(
        ui,
        "set cells",
        super::clips::uses(built, &name).len().to_string(),
    );
    if unused > 0 {
        form::kv_text(ui, "unused clips", format!("{unused} in the libraries"));
    }
    form::hint(
        ui,
        "Cues live on the skeleton, keyed by clip name, so one apex serves every model that plays \
         the clip.",
    );
}

fn hand(
    editor: &Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    id: engine::systems::actor::skeleton::SkeletonId,
    built: &BuiltSkeleton,
) {
    let index = editor.views.skeleton.view(key).hand;
    let Some(hand) = built.def.hands.get(index) else {
        panel::header(ui, "Hand", "", |_| {});
        form::hint(ui, "No hands: nothing this skeleton wears is ever shown.");
        return;
    };
    panel::header(ui, &format!("Hand · {}", hand.name), "", |_| {});
    form::kv_code(ui, "joint", hand.joint.as_str());
    let wearers = super::wearers(content, id);
    let fitting = wearers.iter().filter(|(_, model)| model.is_some()).count();
    form::kv_text(
        ui,
        "in every model",
        format!("{fitting} of {}", wearers.len()),
    );
    form::kv_text(
        ui,
        "role",
        match index {
            0 => "main: a stance's main hand",
            1 => "off: a stance's off hand",
            _ => "taken only by stances asking for this many hands",
        },
    );
    form::hint(
        ui,
        "A hand's joint must be in the rig and in every model wearing the skeleton; a model missing \
         it is refused at load.",
    );
}

fn layer(editor: &Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton) {
    panel::header(ui, "Layer", "", |_| {});
    let index = editor.views.skeleton.view(key).layers.layer;
    let Some(layer) = built.def.layers.get(index) else {
        form::hint(ui, "No layers.");
        return;
    };
    let joints = layer.joints();
    let in_rig = joints
        .iter()
        .all(|(_, joint)| built.rig.named(joint.as_str()).is_some());
    form::kv_text(
        ui,
        "joints",
        format!(
            "{}{}",
            joints.len(),
            if in_rig { ", every one in the rig" } else { "" }
        ),
    );
    form::kv_text(
        ui,
        "during",
        layer
            .during()
            .iter()
            .map(|action| action.as_ref())
            .collect::<Vec<_>>()
            .join(", "),
    );
    if let engine::systems::actor::skeleton::Layer::Feet { legs, .. } = layer {
        form::kv_text(
            ui,
            "cost",
            format!("{0} raycasts + {0} solves a frame", legs.len()),
        );
    }
    for refusal in super::layers::refusals(&built.rig, std::slice::from_ref(layer)) {
        ui.colored_label(theme::DANGER, refusal);
    }
    form::hint(
        ui,
        "A leg must be a chain down the tree. The page refuses to save one that is not, and names \
         the joint.",
    );
}
