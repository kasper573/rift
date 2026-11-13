use std::collections::BTreeSet;
use ui::controls;
use ui::egui;
use ui::face;
use ui::form;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::content::{Content, ContentFile, fields_file};
use engine::core::script::{Expr, MayDo};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::script::{
    EffectReach, Node, OwnerRow, Place, RiftEffect, RiftScripting, RiftScripts, RunnerKind,
    ScriptDef, VarValue,
};
use engine::systems::stat::StatKind;
use ui::egui::{RichText, Ui};

use super::nodes::{self, NodeReads, NodeText};
use super::{ScriptDoc, ScriptPath, ScriptRef, ScriptRow, run};
use crate::app::{BottomTab, Editor};
use crate::project::RowSelection;
use crate::project::files::content_of;
use crate::tabs::shown_path;
use crate::tabs::{EditorTab, file_title};

pub fn show(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, tab: BottomTab) {
    match tab {
        BottomTab::UsedBy => used_by(editor, ui, doc),
        BottomTab::Run => run::panel(editor, ui, doc),
        _ => may_do(editor, ui, doc),
    }
}

pub struct ScriptBinding {
    pub label: String,
    pub row: OwnerRow,
    pub file: String,
}

pub fn bindings(editor: &Editor, doc: &ScriptDoc) -> Vec<ScriptBinding> {
    let Some(project) = &editor.project else {
        return Vec::new();
    };
    let Some(content) = project.content.clone() else {
        return Vec::new();
    };
    let mut found: Vec<ScriptBinding> = Vec::new();
    for usage in project.usages.of_content(&doc.at.key) {
        let Some((table, row_key)) = content_of(&usage.file) else {
            continue;
        };
        let Some(module) = ScriptRef::parse(&usage.pointer).module else {
            continue;
        };
        if table == ScriptDef::TABLE {
            continue;
        }
        if let Some(bound) = owner_row(&content, table, row_key, &module.pointer())
            && !found.iter().any(|known| known.row == bound.row)
        {
            found.push(bound);
        }
    }
    found
}

pub fn owner_row(
    content: &Content,
    table: &str,
    key: &str,
    pointer: &str,
) -> Option<ScriptBinding> {
    let kind: RunnerKind = table.parse().ok()?;
    let file = fields_file(key);
    let bound = |label: String, row: OwnerRow| ScriptBinding {
        label,
        row,
        file: file.clone(),
    };
    Some(match kind {
        RunnerKind::Area => {
            let area = content.by_path::<AreaDef>(key)?;
            match ScriptPath::parse(pointer)
                .as_ref()
                .and_then(ScriptPath::object)
            {
                Some(module) => {
                    let placed = area::placed(content, area);
                    let index = placed
                        .iter()
                        .position(|p| p.chain.is_empty() && p.origin == module)?;
                    let name = match &placed[index].module {
                        AreaModule::Object(object) => object.display_name(index as u32),
                        _ => "object".to_owned(),
                    };
                    bound(
                        format!("{} › {name}", file_title(key)),
                        OwnerRow {
                            kind: RunnerKind::Object,
                            row: area.raw(),
                            object: Some(index as u32),
                        },
                    )
                }
                None => bound(
                    shown_path(key).to_owned(),
                    OwnerRow {
                        kind: RunnerKind::Area,
                        row: area.raw(),
                        object: None,
                    },
                ),
            }
        }
        RunnerKind::Blueprint => {
            let blueprint = content.by_path::<BlueprintDef>(key)?;
            content.ids::<AreaDef>().find_map(|area| {
                let ordinal = area::placements(content, area)
                    .iter()
                    .position(|placement| {
                        placement.blueprint == blueprint && !placement.scripts.is_empty()
                    })?;
                Some(bound(
                    format!(
                        "{} in {} {ordinal}",
                        shown_path(key),
                        shown_path(area.path(content))
                    ),
                    OwnerRow {
                        kind: RunnerKind::Blueprint,
                        row: area.raw(),
                        object: Some(ordinal as u32),
                    },
                ))
            })?
        }
        RunnerKind::Object | RunnerKind::Any => return None,
        kind => bound(
            shown_path(key).to_owned(),
            OwnerRow {
                kind,
                row: content.table_named(table)?.raw_of(key)?,
                object: None,
            },
        ),
    })
}

fn may_do(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        ui.label(theme::muted("The folder does not load"));
        return;
    };
    let bound = bindings(editor, doc);
    if bound.is_empty() {
        ui.label(theme::muted(
            "No row names this script yet, so it may do nothing in play.",
        ));
        return;
    }
    let pick_id = ui.id().with(("may-do-owner", doc.at.slug()));
    let mut picked: usize = ui.ctx().data(|d| d.get_temp(pick_id)).unwrap_or(0);
    if bound.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(theme::muted("as").size(13.0));
            controls::combo(pick_id)
                .selected_text(
                    face::mono(
                        bound
                            .get(picked)
                            .map(|b| b.label.clone())
                            .unwrap_or_default(),
                    )
                    .size(13.0),
                )
                .show_ui(ui, |ui| {
                    for (index, owner) in bound.iter().enumerate() {
                        ui.selectable_value(&mut picked, index, &owner.label);
                    }
                });
            ui.label(
                theme::muted(format!("· one summary per owner, {} in all", bound.len())).size(12.5),
            );
        });
        ui.ctx().data_mut(|d| d.insert_temp(pick_id, picked));
        ui.add_space(4.0);
    }
    let Some(owner) = bound.get(picked).or(bound.first()) else {
        return;
    };
    let may = content.built::<RiftScripts>().may_do(owner.row);
    let rows = may_do_rows(&content, doc, &may);
    let read_by = read_by_rows(&content, &may);
    let wide = ui.available_width() > 760.0 && !read_by.is_empty();
    egui::ScrollArea::vertical()
        .id_salt(("may-do", doc.at.slug()))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(if wide {
                        ui.available_width() * 0.55
                    } else {
                        ui.available_width()
                    });
                    for row in &rows {
                        may_row(ui, row);
                    }
                });
                if wide {
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        sheet::section(ui, "Read by");
                        for (who, what) in &read_by {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    egui::vec2(96.0, 20.0),
                                    egui::Label::new(theme::strong(who).size(13.0)),
                                );
                                ui.add(egui::Label::new(theme::muted(what).size(13.0)).wrap());
                            });
                            form::rule(ui);
                        }
                    });
                }
            });
            if !wide && !read_by.is_empty() {
                ui.add_space(6.0);
            }
        });
}

struct MayRow {
    label: &'static str,
    text: String,
    present: bool,
}

impl MayRow {
    fn present(label: &'static str, text: String) -> MayRow {
        MayRow {
            label,
            text,
            present: true,
        }
    }

    fn maybe(label: &'static str, parts: &[String], none: &str, join: &str) -> MayRow {
        MayRow {
            label,
            text: if parts.is_empty() {
                none.to_owned()
            } else {
                parts.join(join)
            },
            present: !parts.is_empty(),
        }
    }
}

fn may_row(ui: &mut Ui, row: &MayRow) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            egui::vec2(120.0, 20.0),
            egui::Label::new(
                RichText::new(row.label.to_uppercase())
                    .size(11.5)
                    .color(theme::MUTED)
                    .family(theme::medium()),
            ),
        );
        let rich = if row.present {
            RichText::new(&row.text).size(13.0).color(theme::TEXT)
        } else {
            theme::muted(&row.text).size(13.0)
        };
        ui.add(egui::Label::new(rich).wrap());
    });
    ui.add_space(2.0);
}

fn unique(list: impl IntoIterator<Item = String>) -> Vec<String> {
    list.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn may_do_rows(content: &Content, doc: &ScriptDoc, may: &MayDo<RiftScripting>) -> Vec<MayRow> {
    let mut rows: Vec<MayRow> = Vec::new();
    let nodes = doc.nodes();
    let dialogue_count = nodes
        .values()
        .filter(|node| nodes::is_dialogue(node))
        .count();
    let speakers = unique(
        nodes
            .values()
            .flat_map(|node| match node {
                Node::Dialogue(dialogue) => dialogue.lines.as_slice(),
                _ => &[],
            })
            .map(|line| nodes::speaker_name(&doc.reader, line.by)),
    );
    let called_dialogue = may.dialogue.len().saturating_sub(dialogue_count);
    if dialogue_count > 0 || called_dialogue > 0 {
        let by = match speakers.as_slice() {
            [] => String::new(),
            [one] if one == "narrator" => ", all by the narrator".to_owned(),
            [one] => format!(", all lines by {}", shown_path(one)),
            many => format!(
                ", by {}",
                many.iter()
                    .map(|s| if s == "narrator" || s == "object" {
                        s.clone()
                    } else {
                        shown_path(s).to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let more = if called_dialogue > 0 {
            format!(" · {called_dialogue} more through {}", may.calls.join(", "))
        } else {
            String::new()
        };
        rows.push(MayRow::present(
            "says",
            format!(
                "{dialogue_count} dialogue node{} to the subject{by}{more}",
                words::plural(dialogue_count)
            ),
        ));
    }
    let shops: Vec<String> = may
        .reaches
        .iter()
        .filter_map(|r| match r {
            EffectReach::OpensShop(shop) => Some(shown_path(shop.path(content)).to_owned()),
            _ => None,
        })
        .collect();
    if !shops.is_empty() {
        rows.push(MayRow::present(
            "opens",
            format!("{} for the subject", shops.join(", ")),
        ));
    }
    if !may.calls.is_empty() {
        let calls: Vec<String> = may
            .calls
            .iter()
            .map(|script| shown_path(script).to_owned())
            .collect();
        rows.push(MayRow::present("runs", calls.join(" · ")));
    }
    let reads = NodeReads::of(&doc.reader, nodes).text();
    if !reads.is_empty() {
        rows.push(MayRow::present("reads", reads));
    }
    let warps: Vec<String> = may
        .reaches
        .iter()
        .filter_map(|r| match r {
            EffectReach::Warp(place) => Some(format!(
                "the subject to area/{} · {} · sets its spawn point there",
                place.area.path(content),
                place_coords(place)
            )),
            _ => None,
        })
        .collect();
    rows.push(MayRow::maybe("warps", &warps, "nowhere", " · "));
    let takes = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Takes(item) => Some(stack_text(doc, Hands::Take, item.path(content))),
        _ => None,
    }));
    let gives = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Gives(item) => Some(stack_text(doc, Hands::Give, item.path(content))),
        _ => None,
    }));
    if !takes.is_empty() || !gives.is_empty() {
        rows.push(MayRow {
            label: "takes",
            text: if takes.is_empty() {
                "nothing".to_owned()
            } else {
                format!("from the subject: {}", takes.join(", "))
            },
            present: !takes.is_empty(),
        });
        rows.push(MayRow {
            label: "gives",
            text: if gives.is_empty() {
                "nothing".to_owned()
            } else {
                format!("the subject {}", gives.join(", "))
            },
            present: !gives.is_empty(),
        });
    }
    let drops = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Drops(item) => Some(item.path(content).to_owned()),
        _ => None,
    }));
    if !drops.is_empty() {
        rows.push(MayRow::present(
            "drops",
            format!("{} at the owner, reserved to the subject", drops.join(", ")),
        ));
    }
    let memories = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Remembers(memory) => Some(memory.path(content).to_owned()),
        _ => None,
    }));
    if !memories.is_empty() {
        rows.push(MayRow::present(
            "remembers",
            format!("the subject's {}", memories.join(", ")),
        ));
    }
    let notices = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Notifies(n) => Some(shown_path(n.path(content)).to_owned()),
        _ => None,
    }));
    if !notices.is_empty() {
        rows.push(MayRow::present("notifies", notices.join(", ")));
    }
    let mut quests: Vec<(String, Vec<&str>)> = Vec::new();
    for reach in &may.reaches {
        let (verb, quest) = match reach {
            EffectReach::Offers(q) => ("offers", q),
            EffectReach::Accepts(q) => ("accepts", q),
            EffectReach::TurnsIn(q) => ("turns in", q),
            EffectReach::Fails(q) => ("fails", q),
            EffectReach::Completes(q) => ("completes a task of", q),
            EffectReach::Decides(q) => ("decides a path of", q),
            _ => continue,
        };
        let key = quest.path(content).to_owned();
        match quests.iter_mut().find(|(q, _)| *q == key) {
            Some((_, verbs)) => {
                if !verbs.contains(&verb) {
                    verbs.push(verb);
                }
            }
            None => quests.push((key, vec![verb])),
        }
    }
    if !quests.is_empty() {
        let text = quests
            .iter()
            .map(|(quest, verbs)| {
                let verbs = match verbs.as_slice() {
                    [one] => (*one).to_owned(),
                    [head @ .., last] => format!("{} and {last}", head.join(", ")),
                    [] => String::new(),
                };
                format!("{verbs} {}", shown_path(quest))
            })
            .collect::<Vec<_>>()
            .join(" · ");
        rows.push(MayRow::present("quests", text));
    }
    let npcs = unique(may.reaches.iter().filter_map(|reach| match reach {
        EffectReach::Spawns(npc) => Some(format!("spawns {}", shown_path(npc.path(content)))),
        EffectReach::SpawnsObject(spawnable) => {
            Some(format!("spawns {}", shown_path(spawnable.path(content))))
        }
        EffectReach::Hostile => Some("may turn hostile".to_owned()),
        EffectReach::Friendly => Some("may turn friendly".to_owned()),
        EffectReach::Despawns => Some("despawns".to_owned()),
        EffectReach::Walks => Some("walks".to_owned()),
        EffectReach::Targets | EffectReach::Damages => Some("attacks".to_owned()),
        _ => None,
    }));
    if !npcs.is_empty() {
        rows.push(MayRow::present("npcs", npcs.join(" · ")));
    }
    let stats = unique(may.reaches.iter().filter_map(|reach| match reach {
        EffectReach::Heals => Some("heals".to_owned()),
        EffectReach::Applies(effect) => {
            Some(format!("applies {}", shown_path(effect.path(content))))
        }
        EffectReach::Cleanses => Some("removes statuses".to_owned()),
        EffectReach::Boosts(stat, _) => Some(format!("boosts {}", stat_name(*stat))),
        EffectReach::GrantsXp => Some("grants xp".to_owned()),
        EffectReach::Casts(skill) => Some(format!("casts {}", shown_path(skill.path(content)))),
        EffectReach::ResetsSkills => Some("resets skills".to_owned()),
        EffectReach::Animates => Some("plays an action".to_owned()),
        EffectReach::ShowsVfx => Some("shows vfx".to_owned()),
        EffectReach::Plays(sfx) => Some(format!("plays {}", shown_path(sfx.path(content)))),
        EffectReach::Says => Some("says a chat line".to_owned()),
        _ => None,
    }));
    if !stats.is_empty() {
        rows.push(MayRow::present("also", stats.join(" · ")));
    }
    let mut waits: Vec<String> = Vec::new();
    if dialogue_count > 0 || called_dialogue > 0 {
        waits.push("choices".to_owned());
    }
    if !shops.is_empty() {
        waits.push("the shop".to_owned());
    }
    if may
        .reaches
        .iter()
        .any(|r| matches!(r, EffectReach::TurnsIn(_) | EffectReach::Offers(_)))
    {
        waits.push("the quest card".to_owned());
    }
    let seconds: Vec<String> = nodes
        .values()
        .filter_map(|node| match node {
            Node::Wait {
                seconds: Expr::Num(seconds),
                ..
            } => Some(*seconds),
            Node::Wait {
                seconds: Expr::Int(seconds),
                ..
            } => Some(*seconds as f64),
            _ => None,
        })
        .map(|s| format!("{} s", s))
        .collect();
    if !seconds.is_empty() {
        waits.push(seconds.join(", "));
    }
    rows.push(MayRow {
        label: "waits",
        text: if waits.is_empty() {
            "never".to_owned()
        } else if seconds.len() == waits.len() {
            waits.join(" · ")
        } else {
            format!("for the subject: {}", waits.join(", "))
        },
        present: !waits.is_empty(),
    });
    rows
}

fn place_coords(place: &Place) -> String {
    format!("{}, {}", place.at.x, place.at.y)
}

/// Which way items change hands.
#[derive(Clone, Copy)]
enum Hands {
    Take,
    Give,
}

fn stack_text(doc: &ScriptDoc, hands: Hands, item: &str) -> String {
    for node in doc.nodes().values() {
        let Node::Effect(RiftEffect::Items(items)) = node else {
            continue;
        };
        let stacks = match hands {
            Hands::Take => items.named_take(),
            Hands::Give => items.named_give(),
        };
        for stack in stacks {
            if doc.reader.path_of(stack.item) == Some(item) {
                return format!("{} {}", stack.count, shown_path(item));
            }
        }
    }
    shown_path(item).to_owned()
}

fn read_by_rows(content: &Content, may: &MayDo<RiftScripting>) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let warp = may.reaches.iter().find_map(|r| match r {
        EffectReach::Warp(place) => Some(place),
        _ => None,
    });
    if let Some(place) = warp {
        let area = place.area.path(content);
        rows.push(("World map".to_owned(), format!("a route to {area}")));
        rows.push((
            "Area view".to_owned(),
            format!("{area} · arrival pin at {}", place_coords(place)),
        ));
        rows.push((
            "Loader".to_owned(),
            format!("{area}'s {} is a valid place", place_coords(place)),
        ));
    }
    let quests = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Offers(q) | EffectReach::Accepts(q) | EffectReach::TurnsIn(q) => {
            Some(q.path(content).to_owned())
        }
        _ => None,
    }));
    if !quests.is_empty() {
        rows.push((
            "Quest log".to_owned(),
            format!("given by and turned in at, for {}", quests.join(", ")),
        ));
        rows.push((
            "World map".to_owned(),
            "quest marks on this owner".to_owned(),
        ));
    }
    let items = unique(may.reaches.iter().filter_map(|r| match r {
        EffectReach::Takes(i) | EffectReach::Gives(i) | EffectReach::Drops(i) => {
            Some(i.path(content).to_owned())
        }
        _ => None,
    }));
    if !items.is_empty() {
        rows.push((
            "Item sources".to_owned(),
            format!("{} change hands here", items.join(", ")),
        ));
    }
    rows
}

pub(crate) fn describe_reach(content: &Content, reach: &EffectReach) -> String {
    match reach {
        EffectReach::Warp(place) => format!(
            "warps to {} {}",
            place.area.path(content),
            place_coords(place)
        ),
        EffectReach::Takes(item) => format!("takes {}", shown_path(item.path(content))),
        EffectReach::Gives(item) => format!("gives {}", shown_path(item.path(content))),
        EffectReach::Drops(item) => format!("drops {}", shown_path(item.path(content))),
        EffectReach::Offers(quest) => format!("offers quest {}", shown_path(quest.path(content))),
        EffectReach::Accepts(quest) => format!("accepts quest {}", shown_path(quest.path(content))),
        EffectReach::TurnsIn(quest) => {
            format!("turns in quest {}", shown_path(quest.path(content)))
        }
        EffectReach::Fails(quest) => format!("fails quest {}", shown_path(quest.path(content))),
        EffectReach::Completes(quest) => {
            format!("completes a task of {}", shown_path(quest.path(content)))
        }
        EffectReach::Decides(quest) => {
            format!("decides a path of {}", shown_path(quest.path(content)))
        }
        EffectReach::Remembers(memory) => format!("remembers {}", shown_path(memory.path(content))),
        EffectReach::OpensShop(shop) => format!("opens shop {}", shown_path(shop.path(content))),
        EffectReach::Spawns(npc) => format!("spawns {}", shown_path(npc.path(content))),
        EffectReach::SpawnsObject(spawnable) => {
            format!("spawns {}", shown_path(spawnable.path(content)))
        }
        EffectReach::Hostile => "may turn hostile".to_owned(),
        EffectReach::Friendly => "may turn friendly".to_owned(),
        EffectReach::Despawns => "despawns".to_owned(),
        EffectReach::Targets => "targets actors".to_owned(),
        EffectReach::Walks => "walks".to_owned(),
        EffectReach::Damages => "damages".to_owned(),
        EffectReach::Heals => "heals".to_owned(),
        EffectReach::Applies(effect) => format!("applies {}", shown_path(effect.path(content))),
        EffectReach::Cleanses => "removes statuses".to_owned(),
        EffectReach::Boosts(stat, _) => format!("boosts {}", stat_name(*stat)),
        EffectReach::GrantsXp => "grants xp".to_owned(),
        EffectReach::Casts(skill) => format!("casts {}", shown_path(skill.path(content))),
        EffectReach::ResetsSkills => "resets skills".to_owned(),
        EffectReach::Animates => "plays an action".to_owned(),
        EffectReach::ShowsVfx => "shows vfx".to_owned(),
        EffectReach::Notifies(notification) => {
            format!("notifies {}", shown_path(notification.path(content)))
        }
        EffectReach::Says => "says".to_owned(),
        EffectReach::Plays(sfx) => format!("plays {}", shown_path(sfx.path(content))),
        EffectReach::Speaks(npc) => format!("speaks as {}", shown_path(npc.path(content))),
        EffectReach::Prompts => "asks for items".to_owned(),
        EffectReach::ReadsOffer => "reads what was offered".to_owned(),
        EffectReach::TakesOffer => "takes what was offered".to_owned(),
        EffectReach::Crafts(book) => format!("crafts from {}", shown_path(book.path(content))),
    }
}

struct NamedBy {
    table: String,
    key: String,
    name: String,
    module: Option<ScriptPath>,
    what: String,
}

struct Open {
    table: String,
    key: String,
    module: Option<ScriptPath>,
}

fn used_by(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let mut open: Option<Open> = None;
    let key = &doc.at.key;
    let Some(project) = &editor.project else {
        return;
    };
    let mut rows: Vec<NamedBy> = Vec::new();
    let text = doc.text(project);
    for (file, pointers) in project.usages.files_naming(key) {
        let Some((table, row_key)) = content_of(&file) else {
            continue;
        };
        for pointer in pointers {
            let at = ScriptRef::parse(&pointer);
            let module = at
                .module
                .as_ref()
                .and_then(|module| module.module_in(project, &file));
            let (name, what) = match at.node {
                Some(node) => (node, "run node".to_owned()),
                None => (
                    String::new(),
                    module
                        .map(|module| summarise_values(&text, &module.values))
                        .unwrap_or_default(),
                ),
            };
            rows.push(NamedBy {
                table: table.to_owned(),
                key: row_key.to_owned(),
                name,
                module: at.module,
                what,
            });
        }
    }
    if rows.is_empty() {
        form::hint(ui, "Nothing names this script yet.");
        return;
    }
    let owners: BTreeSet<(&str, &str)> = rows
        .iter()
        .map(|row| (row.table.as_str(), row.key.as_str()))
        .collect();
    let shown = 8;
    egui::ScrollArea::vertical()
        .id_salt(("used-by", key))
        .show(ui, |ui| {
            for row in rows.iter().take(shown) {
                let response = ui
                    .horizontal(|ui| {
                        ui.add_sized(
                            egui::vec2(150.0, 20.0),
                            egui::Label::new(theme::code(shown_path(&row.key)).size(13.0))
                                .truncate(),
                        );
                        ui.add_sized(
                            egui::vec2(120.0, 20.0),
                            egui::Label::new(face::mono(&row.name).size(13.0)).truncate(),
                        );
                        ui.label(theme::muted(&row.what).size(13.0));
                    })
                    .response
                    .interact(egui::Sense::click());
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    open = Some(Open {
                        table: row.table.clone(),
                        key: row.key.clone(),
                        module: row.module.clone(),
                    });
                }
                form::rule(ui);
            }
            if rows.len() > shown {
                let all_areas = owners.iter().all(|(table, _)| *table == AreaDef::TABLE);
                ui.label(
                    theme::muted(format!(
                        "… {} more in {} {}{} · click one to open its {}",
                        rows.len() - shown,
                        owners.len(),
                        if all_areas { "area" } else { "owner" },
                        words::plural(owners.len()),
                        if all_areas {
                            "area with it selected"
                        } else {
                            "row"
                        }
                    ))
                    .monospace()
                    .size(12.5),
                );
            } else {
                ui.label(
                    theme::muted("click one to open its row")
                        .monospace()
                        .size(12.5),
                );
            }
        });
    let Some(Open { table, key, module }) = open else {
        return;
    };
    editor.select(RowSelection::row(&table, &key));
    match module {
        Some(module) if table == AreaDef::TABLE => {
            editor.select(RowSelection::parts(
                &table,
                &key,
                vec![module.module().to_owned()],
            ));
            editor.open_tab(EditorTab::Area { key });
        }
        _ => editor.open_tab(EditorTab::for_row(&table, &key)),
    }
}

pub(crate) fn used_by_count(editor: &Editor, script: &ScriptRow) -> usize {
    editor
        .project
        .as_ref()
        .map_or(0, |project| project.usages.of_content(&script.key).len())
}

fn summarise_values(
    text: &NodeText,
    values: &std::collections::BTreeMap<String, VarValue>,
) -> String {
    values
        .iter()
        .map(|(name, value)| format!("{name} {}", nodes::var_value_text(text, value)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn stat_name(stat: StatKind) -> String {
    let name: &'static str = stat.into();
    name.to_lowercase()
}
