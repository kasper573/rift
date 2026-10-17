use std::collections::BTreeSet;

use egui::{RichText, Ui};
use engine::core::content::Content;
use engine::core::script::MayDo;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::item::ItemDef;
use engine::systems::npc::NpcDef;
use engine::systems::quest::QuestDef;
use engine::systems::script::{OwnerKind, OwnerRow, Place, Reach, RiftScripting, RiftScripts};
use engine::systems::shop::ShopDef;
use engine::systems::skill::SkillDef;
use engine::systems::status::StatusEffectDef;
use serde_json::Value;

use super::nodes::{self, NodeReads};
use super::{ScriptDoc, ScriptOwner, ScriptPanel, ScriptPath, ScriptRef, run};
use crate::app::Editor;
use crate::condition;
use crate::project::{RowSelection, row_file, row_of};
use crate::tabs::EditorTab;
use crate::ui::{theme, widgets};
use crate::value::RowValue;

pub fn show(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, panel: ScriptPanel) {
    match panel {
        ScriptPanel::MayDo => may_do(editor, ui, doc),
        ScriptPanel::UsedBy => used_by(editor, ui, doc),
        ScriptPanel::Run => run::panel(editor, ui, doc),
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
    match &doc.owner {
        ScriptOwner::Inline { table, key, module } => {
            owner_row(&content, table, key, &module.pointer())
                .into_iter()
                .collect()
        }
        ScriptOwner::Shared { key } => {
            let mut found: Vec<ScriptBinding> = Vec::new();
            for usage in project.usages.of_row("script", key) {
                let Some((table, row_key)) = row_of(&usage.file) else {
                    continue;
                };
                let Some(module) = ScriptRef::parse(&usage.pointer).module else {
                    continue;
                };
                if table == "script" {
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
    }
}

pub fn owner_row(
    content: &Content,
    table: &str,
    key: &str,
    pointer: &str,
) -> Option<ScriptBinding> {
    let kind: OwnerKind = table.parse().ok()?;
    let file = row_file(table, key);
    let bound = |label: String, row: OwnerRow| ScriptBinding {
        label,
        row,
        file: file.clone(),
    };
    Some(match kind {
        OwnerKind::Area => {
            let area = content.by_key::<AreaDef>(key)?;
            match ScriptPath::parse(pointer).and_then(ScriptPath::object) {
                Some(module) => {
                    let placed = area::placed(content, area);
                    let index = placed
                        .iter()
                        .position(|p| p.chain.is_empty() && p.origin == module)?;
                    let name = match &placed[index].module {
                        AreaModule::Object(object) => object.title(index as u32),
                        _ => "object".to_owned(),
                    };
                    bound(
                        format!("area/{key} › {name}"),
                        OwnerRow {
                            kind: OwnerKind::Object,
                            row: area.raw(),
                            object: Some(index as u32),
                        },
                    )
                }
                None => bound(
                    format!("area/{key}"),
                    OwnerRow {
                        kind: OwnerKind::Area,
                        row: area.raw(),
                        object: None,
                    },
                ),
            }
        }
        OwnerKind::Blueprint => {
            let blueprint = content.by_key::<BlueprintDef>(key)?;
            content.ids::<AreaDef>().find_map(|area| {
                let ordinal = area::placements(content, area)
                    .iter()
                    .position(|placement| {
                        placement.blueprint == blueprint && !placement.scripts.is_empty()
                    })?;
                Some(bound(
                    format!("blueprint/{key} in {} {ordinal}", area.key(content)),
                    OwnerRow {
                        kind: OwnerKind::Blueprint,
                        row: area.raw(),
                        object: Some(ordinal as u32),
                    },
                ))
            })?
        }
        OwnerKind::Object | OwnerKind::Any => return None,
        kind => bound(
            format!("{table}/{key}"),
            OwnerRow {
                kind,
                row: raw_of(content, kind, key)?,
                object: None,
            },
        ),
    })
}

fn raw_of(content: &Content, kind: OwnerKind, key: &str) -> Option<u32> {
    Some(match kind {
        OwnerKind::Npc => content.by_key::<NpcDef>(key)?.raw(),
        OwnerKind::Item => content.by_key::<ItemDef>(key)?.raw(),
        OwnerKind::Quest => content.by_key::<QuestDef>(key)?.raw(),
        OwnerKind::Shop => content.by_key::<ShopDef>(key)?.raw(),
        OwnerKind::Skill => content.by_key::<SkillDef>(key)?.raw(),
        OwnerKind::StatusEffect => content.by_key::<StatusEffectDef>(key)?.raw(),
        OwnerKind::Area | OwnerKind::Object => content.by_key::<AreaDef>(key)?.raw(),
        OwnerKind::Blueprint => content.by_key::<BlueprintDef>(key)?.raw(),
        OwnerKind::Any => return None,
    })
}

fn may_do(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        ui.label(theme::muted("The folder does not load"));
        return;
    };
    let bound = bindings(editor, doc);
    if bound.is_empty() {
        ui.label(theme::muted(if doc.is_shared() {
            "No owner names this script yet, so it may do nothing in play."
        } else {
            "No owner found for this script."
        }));
        return;
    }
    let pick_id = ui.id().with(("may-do-owner", doc.owner.slug()));
    let mut picked: usize = ui.ctx().data(|d| d.get_temp(pick_id)).unwrap_or(0);
    if bound.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(theme::muted("as").size(13.0));
            widgets::combo_box(pick_id)
                .selected_text(
                    theme::mono(
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
        .id_salt(("may-do", doc.owner.slug()))
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
                        widgets::section(ui, "Read by");
                        for (who, what) in &read_by {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    egui::vec2(96.0, 20.0),
                                    egui::Label::new(theme::strong(who).size(13.0)),
                                );
                                ui.add(egui::Label::new(theme::muted(what).size(13.0)).wrap());
                            });
                            widgets::rule(ui);
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
            .flat_map(|node| node["lines"].as_array().cloned().unwrap_or_default())
            .map(|line| nodes::speaker_name(&line["by"])),
    );
    let called_dialogue = may.dialogue.len().saturating_sub(dialogue_count);
    if dialogue_count > 0 || called_dialogue > 0 {
        let by = match speakers.as_slice() {
            [] => String::new(),
            [one] if one == "narrator" => ", all by the narrator".to_owned(),
            [one] => format!(", all lines by npc/{one}"),
            many => format!(
                ", by {}",
                many.iter()
                    .map(|s| if s == "narrator" || s == "object" {
                        s.clone()
                    } else {
                        format!("npc/{s}")
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
                widgets::plural(dialogue_count)
            ),
        ));
    }
    let shops: Vec<String> = may
        .reaches
        .iter()
        .filter_map(|r| match r {
            Reach::OpensShop(shop) => Some(format!("shop/{}", shop.key(content))),
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
            .map(|script| format!("script/{script}"))
            .collect();
        rows.push(MayRow::present("runs", calls.join(" · ")));
    }
    let reads = NodeReads::of(nodes).text();
    if !reads.is_empty() {
        rows.push(MayRow::present("reads", reads));
    }
    let warps: Vec<String> = may
        .reaches
        .iter()
        .filter_map(|r| match r {
            Reach::Warp(place) => Some(format!(
                "the subject to area/{} · {} · sets its spawn point there",
                place.area.key(content),
                place_coords(place)
            )),
            _ => None,
        })
        .collect();
    rows.push(MayRow::maybe("warps", &warps, "nowhere", " · "));
    let takes = unique(may.reaches.iter().filter_map(|r| match r {
        Reach::Takes(item) => Some(stack_text(doc, "take", item.key(content))),
        _ => None,
    }));
    let gives = unique(may.reaches.iter().filter_map(|r| match r {
        Reach::Gives(item) => Some(stack_text(doc, "give", item.key(content))),
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
        Reach::Drops(item) => Some(item.key(content).to_owned()),
        _ => None,
    }));
    if !drops.is_empty() {
        rows.push(MayRow::present(
            "drops",
            format!("{} at the owner, reserved to the subject", drops.join(", ")),
        ));
    }
    let memories = unique(may.reaches.iter().filter_map(|r| match r {
        Reach::Remembers(memory) => Some(memory.key(content).to_owned()),
        _ => None,
    }));
    if !memories.is_empty() {
        rows.push(MayRow::present(
            "remembers",
            format!("the subject's {}", memories.join(", ")),
        ));
    }
    let notices = unique(may.reaches.iter().filter_map(|r| match r {
        Reach::Notifies(n) => Some(format!("notification/{}", n.key(content))),
        _ => None,
    }));
    if !notices.is_empty() {
        rows.push(MayRow::present("notifies", notices.join(", ")));
    }
    let mut quests: Vec<(String, Vec<&str>)> = Vec::new();
    for reach in &may.reaches {
        let (verb, quest) = match reach {
            Reach::Offers(q) => ("offers", q),
            Reach::Accepts(q) => ("accepts", q),
            Reach::TurnsIn(q) => ("turns in", q),
            Reach::Fails(q) => ("fails", q),
            Reach::Completes(q) => ("completes a task of", q),
            Reach::Decides(q) => ("decides a path of", q),
            _ => continue,
        };
        let key = quest.key(content).to_owned();
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
                format!("{verbs} quest/{quest}")
            })
            .collect::<Vec<_>>()
            .join(" · ");
        rows.push(MayRow::present("quests", text));
    }
    let npcs = unique(may.reaches.iter().filter_map(|reach| match reach {
        Reach::Spawns(npc) => Some(format!("spawns npc/{}", npc.key(content))),
        Reach::Hostile => Some("may turn hostile".to_owned()),
        Reach::Friendly => Some("may turn friendly".to_owned()),
        Reach::Despawns => Some("despawns".to_owned()),
        Reach::Walks => Some("walks".to_owned()),
        Reach::Targets | Reach::Damages => Some("attacks".to_owned()),
        _ => None,
    }));
    if !npcs.is_empty() {
        rows.push(MayRow::present("npcs", npcs.join(" · ")));
    }
    let stats = unique(may.reaches.iter().filter_map(|reach| match reach {
        Reach::Heals => Some("heals".to_owned()),
        Reach::Applies(effect) => Some(format!("applies {}", effect.key(content))),
        Reach::Cleanses => Some("removes statuses".to_owned()),
        Reach::Boosts(stat, _) => Some(format!("boosts {}", stat_name(*stat))),
        Reach::GrantsXp => Some("grants xp".to_owned()),
        Reach::Casts(skill) => Some(format!("casts {}", skill.key(content))),
        Reach::ResetsSkills => Some("resets skills".to_owned()),
        Reach::Animates => Some("plays an action".to_owned()),
        Reach::ShowsVfx => Some("shows vfx".to_owned()),
        Reach::Plays(sfx) => Some(format!("plays {}", sfx.key(content))),
        Reach::Says => Some("says a chat line".to_owned()),
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
        .any(|r| matches!(r, Reach::TurnsIn(_) | Reach::Offers(_)))
    {
        waits.push("the quest card".to_owned());
    }
    let seconds: Vec<String> = nodes
        .values()
        .filter(|node| node.kind() == "wait")
        .filter_map(|node| node["seconds"].as_f64())
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

fn stack_text(doc: &ScriptDoc, field: &str, item: &str) -> String {
    for node in doc.nodes().values() {
        for stack in node[field].as_array().into_iter().flatten() {
            if stack["item"].as_str() == Some(item) {
                let count = stack["count"].as_u64().unwrap_or(1);
                return format!("{count} {item}");
            }
        }
    }
    item.to_owned()
}

fn read_by_rows(content: &Content, may: &MayDo<RiftScripting>) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let warp = may.reaches.iter().find_map(|r| match r {
        Reach::Warp(place) => Some(place),
        _ => None,
    });
    if let Some(place) = warp {
        let area = place.area.key(content);
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
        Reach::Offers(q) | Reach::Accepts(q) | Reach::TurnsIn(q) => Some(q.key(content).to_owned()),
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
        Reach::Takes(i) | Reach::Gives(i) | Reach::Drops(i) => Some(i.key(content).to_owned()),
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

pub fn describe_reach(content: &Content, reach: &Reach) -> String {
    match reach {
        Reach::Warp(place) => format!(
            "warps to {} {}",
            place.area.key(content),
            place_coords(place)
        ),
        Reach::Takes(item) => format!("takes {}", item.key(content)),
        Reach::Gives(item) => format!("gives {}", item.key(content)),
        Reach::Drops(item) => format!("drops {}", item.key(content)),
        Reach::Offers(quest) => format!("offers quest {}", quest.key(content)),
        Reach::Accepts(quest) => format!("accepts quest {}", quest.key(content)),
        Reach::TurnsIn(quest) => format!("turns in quest {}", quest.key(content)),
        Reach::Fails(quest) => format!("fails quest {}", quest.key(content)),
        Reach::Completes(quest) => format!("completes a task of {}", quest.key(content)),
        Reach::Decides(quest) => format!("decides a path of {}", quest.key(content)),
        Reach::Remembers(memory) => format!("remembers {}", memory.key(content)),
        Reach::OpensShop(shop) => format!("opens shop {}", shop.key(content)),
        Reach::Spawns(npc) => format!("spawns {}", npc.key(content)),
        Reach::Hostile => "may turn hostile".to_owned(),
        Reach::Friendly => "may turn friendly".to_owned(),
        Reach::Despawns => "despawns".to_owned(),
        Reach::Targets => "targets actors".to_owned(),
        Reach::Walks => "walks".to_owned(),
        Reach::Damages => "damages".to_owned(),
        Reach::Heals => "heals".to_owned(),
        Reach::Applies(effect) => format!("applies {}", effect.key(content)),
        Reach::Cleanses => "removes statuses".to_owned(),
        Reach::Boosts(stat, _) => format!("boosts {}", stat_name(*stat)),
        Reach::GrantsXp => "grants xp".to_owned(),
        Reach::Casts(skill) => format!("casts {}", skill.key(content)),
        Reach::ResetsSkills => "resets skills".to_owned(),
        Reach::Animates => "plays an action".to_owned(),
        Reach::ShowsVfx => "shows vfx".to_owned(),
        Reach::Notifies(notification) => format!("notifies {}", notification.key(content)),
        Reach::Says => "says".to_owned(),
        Reach::Plays(sfx) => format!("plays {}", sfx.key(content)),
        Reach::Speaks(npc) => format!("speaks as {}", npc.key(content)),
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
    match &doc.owner {
        ScriptOwner::Inline { table, key, module } => {
            let response = ui
                .horizontal(|ui| {
                    ui.label(theme::code(format!("{table}/{key}")).size(13.0));
                    ui.label(theme::muted("holds it in its own row").size(13.0));
                })
                .response
                .interact(egui::Sense::click());
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                open = Some(Open {
                    table: table.clone(),
                    key: key.clone(),
                    module: Some(*module),
                });
            }
        }
        ScriptOwner::Shared { key } => {
            let Some(project) = &editor.project else {
                return;
            };
            let mut rows: Vec<NamedBy> = Vec::new();
            for (file, pointers) in project.usages.files_naming("script", key) {
                let row = project.files.json(&file);
                let Some((table, row_key)) = row_of(&file) else {
                    continue;
                };
                for pointer in pointers {
                    let at = ScriptRef::parse(&pointer);
                    let module_json = at
                        .module
                        .and_then(|module| row.as_ref()?.pointer(&module.pointer()).cloned());
                    let (name, what) = match at.node {
                        Some(node) => (node, "run node".to_owned()),
                        None => (
                            module_json
                                .as_ref()
                                .and_then(|module| module["name"].as_str())
                                .unwrap_or_default()
                                .to_owned(),
                            module_json
                                .as_ref()
                                .map(|module| summarise_values(&module["values"]))
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
                widgets::hint(ui, "Nothing names this script yet.");
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
                                    egui::Label::new(
                                        theme::code(format!("{}/{}", row.table, row.key))
                                            .size(13.0),
                                    )
                                    .truncate(),
                                );
                                ui.add_sized(
                                    egui::vec2(120.0, 20.0),
                                    egui::Label::new(theme::mono(&row.name).size(13.0)).truncate(),
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
                                module: row.module,
                            });
                        }
                        widgets::rule(ui);
                    }
                    if rows.len() > shown {
                        let all_areas = owners.iter().all(|(table, _)| *table == "area");
                        ui.label(
                            theme::muted(format!(
                                "… {} more in {} {}{} · click one to open its {}",
                                rows.len() - shown,
                                owners.len(),
                                if all_areas { "area" } else { "owner" },
                                widgets::plural(owners.len()),
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
        }
    }
    let Some(Open { table, key, module }) = open else {
        return;
    };
    editor.select(RowSelection::row(&table, &key));
    match module {
        Some(ScriptPath::Module(index)) if table == "area" => {
            editor.select(RowSelection::parts(
                &table,
                &key,
                vec![format!("/modules/{index}")],
            ));
            editor.open_tab(EditorTab::Area { key });
        }
        Some(module) => super::open_inline_or_row(editor, &table, &key, &module.pointer()),
        None => editor.open_tab(EditorTab::for_row(&table, &key)),
    }
}

pub fn used_by_count(editor: &Editor, owner: &ScriptOwner) -> usize {
    match owner {
        ScriptOwner::Inline { .. } => 1,
        ScriptOwner::Shared { key } => editor
            .project
            .as_ref()
            .map_or(0, |project| project.usages.of_row("script", key).len()),
    }
}

fn summarise_values(values: &Value) -> String {
    values
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(name, value)| {
                    let shown =
                        nodes::place_text(value).unwrap_or_else(|| condition::summary(value));
                    format!("{name} {shown}")
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default()
}

fn stat_name(stat: engine::systems::stat::StatKind) -> String {
    let name: &'static str = stat.into();
    name.to_lowercase()
}
