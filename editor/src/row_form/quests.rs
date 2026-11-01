use engine::core::content::ContentFile;
use engine::core::script::node::Node;
use engine::systems::area::AreaModule;
use engine::systems::area::scene::SCENE_FILE;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::quest::QuestDef;
use engine::systems::script::{RiftEffect, ScriptDef};
use ui::egui;
use ui::egui::{Sense, Ui};
use ui::sheet;
use ui::theme;

use super::widgets;
use crate::app::Editor;
use crate::project::files::content_of;
use crate::scene_graph;
use crate::tabs::shown_path;
use crate::views::area::nodes;

pub(super) fn quest_parties_sections(editor: &mut Editor, ui: &mut Ui, quest: &str) {
    let (givers, turn_ins) = quest_parties(editor, quest);
    let mut open: Option<(String, String)> = None;
    for (title, rows, none) in [
        ("Given by · worked out", &givers, "no script offers it yet"),
        (
            "Turned in at · worked out",
            &turn_ins,
            "no script takes it back yet",
        ),
    ] {
        sheet::section(ui, title);
        if rows.is_empty() {
            ui.label(theme::muted(none).size(13.0));
        }
        for party in rows {
            ui.horizontal(|ui| {
                widgets::badge(
                    ui,
                    if party.object.is_some() {
                        "object"
                    } else {
                        &party.table
                    },
                );
                let row = shown_path(&party.key);
                if ui
                    .add(egui::Label::new(theme::mono(row).size(13.0)).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    open = Some((party.table.clone(), party.key.clone()));
                }
                if let Some(object) = &party.object {
                    ui.label(theme::muted(object).size(13.0));
                }
                ui.label(
                    theme::faint(format!("{} · {}", party.script, party.node))
                        .size(12.5)
                        .monospace(),
                );
            });
        }
    }
    if let Some((table, key)) = open {
        editor.open_row(&table, &key);
    }
}

/// A row whose script offers, takes or turns in a quest: an object of an area's when `object`
/// names one.
struct Party {
    table: String,
    key: String,
    object: Option<String>,
    script: String,
    node: String,
}

/// Every row whose scripts offer the quest at `quest`, and every one whose scripts take it back:
/// the scripts with a node doing so, run by whatever names them.
fn quest_parties(editor: &Editor, quest: &str) -> (Vec<Party>, Vec<Party>) {
    let mut givers = Vec::new();
    let mut turn_ins = Vec::new();
    let Some(project) = editor.project.as_ref() else {
        return (givers, turn_ins);
    };
    let reader = project.reader();
    let Some(quest) = reader.id_of::<QuestDef>(quest) else {
        return (givers, turn_ins);
    };
    for script_path in reader.paths_of::<ScriptDef>() {
        let Ok(script) = project.read::<ScriptDef>(script_path) else {
            continue;
        };
        for (name, node) in &script.nodes {
            let gives = match node {
                Node::Effect(RiftEffect::OfferQuest(offer)) if offer.quest == quest => true,
                Node::Effect(RiftEffect::AcceptQuest(accept)) if accept.quest == quest => true,
                Node::Effect(RiftEffect::TurnIn(turn_in)) if turn_in.quest == quest => false,
                _ => continue,
            };
            for usage in project.usages.of_content(script_path) {
                let Some((table, key)) = content_of(&usage.file) else {
                    continue;
                };
                if table == ScriptDef::TABLE || table == CharacterSnapshot::TABLE {
                    continue;
                }
                let object = usage
                    .file
                    .strip_suffix(&format!("/{SCENE_FILE}"))
                    .and_then(|bundle| nodes::bundle_in(project, bundle))
                    .and_then(|bundle| {
                        let node = scene_graph::node_pointer(&usage.pointer);
                        match &nodes::module_node(bundle.nodes(), &node)?.module {
                            AreaModule::Object(object) => Some(
                                object
                                    .name
                                    .clone()
                                    .or(object.title.clone())
                                    .unwrap_or_else(|| "object".to_owned()),
                            ),
                            _ => None,
                        }
                    });
                let party = Party {
                    table: table.to_owned(),
                    key: key.to_owned(),
                    object,
                    script: shown_path(script_path).to_owned(),
                    node: name.clone(),
                };
                if gives {
                    givers.push(party);
                } else {
                    turn_ins.push(party);
                }
            }
        }
    }
    (givers, turn_ins)
}
