//! The skeleton's inspector: its rig, the source rigs and clips that play on it, and who wears it,
//! then what the page in front is about: the table's health and the moves no skeleton declares, the
//! chosen hand or layer, the parts and materials made on it.

use std::collections::BTreeSet;

use engine::core::content::Content;
use engine::core::script::node::Node;
use engine::systems::actor::playing::Animate;
use engine::systems::actor::skeleton::{GestureName, MAX_JOINTS, MoveName, SkeletonDef};
use engine::systems::actor::{Action, MaterialDef, ModelPartDef, Motion};
use engine::systems::script::{RiftEffect, ScriptDef};
use strum::VariantArray;
use ui::egui::Ui;
use ui::{form, panel, theme, words};

use super::SkeletonPage;
use super::rigging::Rigging;
use crate::app::Editor;
use crate::tabs::shown_path;

pub fn show(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    def: &SkeletonDef,
    rigging: &Rigging,
    page: SkeletonPage,
) {
    let content = editor.loaded();
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Rig", "", |_| {});
        ui.add_space(4.0);
        rig(ui, key, def, rigging, content.as_ref());
    });
    ui.add_space(8.0);
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match page {
            SkeletonPage::Rigs => sources(ui, def, rigging),
            SkeletonPage::Sets => sets(editor, ui, content.as_ref(), def, rigging),
            SkeletonPage::Hands => hand(editor, ui, key, def),
            SkeletonPage::Body => body(ui, key, content.as_ref(), def),
            SkeletonPage::Layers => layer(editor, ui, key, def, rigging),
        }
    });
}

fn rig(ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging, content: Option<&Content>) {
    let joints = rigging.rig.as_ref().map_or(0, |rig| rig.joints().count());
    form::kv(ui, "joints", |ui| {
        let ink = if joints > MAX_JOINTS {
            theme::DANGER
        } else {
            theme::TEXT
        };
        ui.colored_label(ink, format!("{joints} of {MAX_JOINTS}"));
    });
    form::kv_text(ui, "read from", words::counted(def.rig.files.len(), "file"));
    form::kv_text(ui, "source rigs", def.sources.len().to_string());
    form::kv_text(ui, "clips that play", rigging.clips.len().to_string());
    if rigging.rig.is_none() {
        ui.colored_label(
            theme::DANGER,
            "no rig stands: pick its files on the rigs page",
        );
    }
    let Some(content) = content else {
        return;
    };
    let loaded = content
        .by_path::<SkeletonDef>(key)
        .is_some_and(|id| engine::systems::actor::model::Skeletons::loaded(content, id).is_some());
    if !loaded {
        ui.colored_label(
            theme::DANGER,
            "the skeleton did not load: its problems are in the output panel",
        );
    }
    let wearers = super::wearers(content, key);
    let unfit: Vec<&str> = wearers
        .iter()
        .filter(|(_, model)| model.is_none())
        .map(|(path, _)| shown_path(path))
        .collect();
    form::kv_text(
        ui,
        "worn by",
        match (wearers.len(), unfit.len()) {
            (0, _) => "no model yet".to_owned(),
            (count, 0) => words::counted(count, "model"),
            (count, _) => format!(
                "{} · {} did not load",
                words::counted(count, "model"),
                unfit.len()
            ),
        },
    );
}

fn sources(ui: &mut Ui, def: &SkeletonDef, rigging: &Rigging) {
    panel::header(ui, "Source rigs", "", |_| {});
    if def.sources.is_empty() {
        form::hint(ui, "None: only clips made on our rig play.");
    }
    for (index, source) in def.sources.iter().enumerate() {
        let plays = rigging
            .clips
            .iter()
            .filter(|clip| clip.source == Some(index))
            .count();
        form::kv_text(
            ui,
            &source.name,
            format!(
                "{} of {} joints paired · {} play",
                source.pairs.len(),
                source.joints.len(),
                words::counted(plays, "clip")
            ),
        );
    }
}

/// What the skeleton's bodies are made of: its parts, and the materials made on it with the tints
/// each paints.
fn body(ui: &mut Ui, key: &str, content: Option<&Content>, def: &SkeletonDef) {
    panel::header(ui, "Body", "", |_| {});
    let Some((content, id)) =
        content.and_then(|content| Some((content, content.by_path::<SkeletonDef>(key)?)))
    else {
        return;
    };
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
    for (material, row) in materials {
        let painted: usize = row.masks.iter().map(|mask| mask.painting().count()).sum();
        form::kv_code(ui, "material", shown_path(material.path(content)));
        form::kv_text(
            ui,
            "paints",
            format!("{painted} of {} tints", def.tints.len()),
        );
    }
}

fn sets(
    editor: &Editor,
    ui: &mut Ui,
    content: Option<&Content>,
    def: &SkeletonDef,
    rigging: &Rigging,
) {
    panel::header(ui, "Sets", "", |_| {});
    let moves: BTreeSet<&MoveName> = def.moves().collect();
    form::kv_text(ui, "stances", def.stances.len().to_string());
    form::kv_text(ui, "builds", def.builds.len().max(1).to_string());
    form::kv_text(ui, "moves", moves.len().to_string());
    let gestures: BTreeSet<&GestureName> = def.gestures().collect();
    form::kv_text(ui, "gestures", gestures.len().to_string());
    form::kv_text(
        ui,
        "engine actions",
        format!(
            "{} of {} in base",
            Action::VARIANTS
                .iter()
                .filter(|action| def.base.clips.contains_key(action))
                .count(),
            Action::VARIANTS.len()
        ),
    );
    let lacking: BTreeSet<String> = def
        .sets()
        .flat_map(|(_, set)| set.attacks().map(|(_, clip)| clip.clip().clone()))
        .filter(|clip| !def.cues_of(clip).iter().any(|cue| cue.apex))
        .filter(|clip| rigging.playable(clip).is_some())
        .map(|clip| clip.shown())
        .collect();
    if lacking.is_empty() {
        form::kv_text(ui, "apex", "every attack and move clip has one");
    } else {
        ui.colored_label(
            theme::DANGER,
            format!(
                "no apex on {}: a hit must land",
                lacking.into_iter().collect::<Vec<_>>().join(", ")
            ),
        );
    }
    let Some(content) = content else {
        return;
    };
    let declared = crate::body_names::move_declarations(content);
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

fn hand(editor: &Editor, ui: &mut Ui, key: &str, def: &SkeletonDef) {
    let index = editor.views.skeleton.view(key).hand;
    let Some(hand) = def.hands.get(index) else {
        panel::header(ui, "Hand", "", |_| {});
        form::hint(ui, "No hands: nothing this skeleton holds is ever shown.");
        return;
    };
    panel::header(ui, &format!("Hand · {}", hand.name), "", |_| {});
    form::kv_code(ui, "joint", hand.joint.as_str());
    form::kv_text(
        ui,
        "role",
        match index {
            0 => "main: a stance's main hand",
            1 => "off: a stance's off hand",
            _ => "taken only by stances asking for this many hands",
        },
    );
    if let Some(clasp) = &hand.clasp {
        form::kv_text(ui, "clasp", clasp.clip.shown());
        form::kv_text(ui, "fingers", clasp.fingers.len().to_string());
    }
}

fn layer(editor: &Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    panel::header(ui, "Layer", "", |_| {});
    let index = editor.views.skeleton.view(key).layers.layer;
    let Some(layer) = def.layers.get(index) else {
        form::hint(ui, "No layers.");
        return;
    };
    let joints = layer.joints();
    form::kv_text(ui, "joints", joints.len().to_string());
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
    if let Some(rig) = &rigging.rig {
        for refusal in super::layers::refusals(rig, std::slice::from_ref(layer)) {
            ui.colored_label(theme::DANGER, refusal);
        }
    }
}
