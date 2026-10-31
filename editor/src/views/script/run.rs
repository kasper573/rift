use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;
use ui::egui;
use ui::form;
use ui::input;
use ui::menu;
use ui::panel;
use ui::sheet;
use ui::theme;

use bevy::ecs::message::{MessageCursor, Messages};
use bevy::prelude::{Entity, World};
use bevy::time::TimeUpdateStrategy;
use bevy_replicon::prelude::{ClientId as Sender, SendTargets, ToClients};
use engine::core::content::{Content, ContentFile, ContentScope, FolderReader};
use engine::core::math::Pos;
use engine::core::script::{FlowSteering, Pause, RunnerKindDef, Value as ScriptValue, ValueType};
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::Meters;
use engine::systems::actor::Name;
use engine::systems::area::{self, AreaId, AreaModule, AreaObject};
use engine::systems::class::{self, Class, ClassDef, ClassId};
use engine::systems::dialogue::{self, Conversation, ConversationRequest};
use engine::systems::input::map::InputMap;
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::memory::{self, Memory, MemoryDef};
use engine::systems::notification::NotificationSent;
use engine::systems::npc::{self, Npc, NpcId};
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::player::{AreaDrawn, ClientId, Xp};
use engine::systems::quest::{
    ActiveQuest, FinishedQuest, QuestDef, QuestId, QuestLog, QuestResult,
};
use engine::systems::script::{
    EventKind, FlowKey, FlowTrace, Node, Owner, Place, Reach, RiftInstances, RiftScripts,
    RunnerKind, TraceStep,
};
use engine::systems::{self, GameSettings, TICK_HZ};
use strum::VariantArray;
use ui::controls;
use ui::egui::{RichText, Ui};
use ui::glyph::{self, Mark};

use super::nodes::{self, NodeReads};
use super::panel::{ScriptBinding, bindings};
use super::{ScriptDoc, ScriptSessionId, inspect};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::RowSelection;
use crate::project::files::content_of;
use crate::shell::widgets;
use crate::stage::live;
use crate::tabs::EditorTab;
use crate::tabs::shown_path;
use crate::views::character::Standing;
use ui::input::{InputField, NumberField};

const CLIENT: u32 = 1;
const MAX_TICKS: usize = 2_000;
// Every run of a script plays out alike.
const RUN_SEED: u64 = 1;
const COUNT_WIDTH: f32 = 80.0;

#[derive(Clone, Debug)]
pub struct RunSetup {
    pub event: String,
    pub owner: usize,
    pub character: Option<String>,
    pub hour: f32,
    pub random: BTreeMap<String, usize>,
    pub step: bool,
}

impl Default for RunSetup {
    fn default() -> RunSetup {
        RunSetup {
            event: EventKind::Interacted.name().to_owned(),
            owner: 0,
            character: None,
            hour: 12.0,
            random: BTreeMap::new(),
            step: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptRunState {
    Running,
    Dialogue,
    Waiting { left: Duration },
    Paused { what: String },
    Stepping,
    Ended,
}

pub enum RunLogLine {
    Node { name: String, procedure: String },
    Note(String),
}

impl RunLogLine {
    fn node(&self) -> Option<&str> {
        match self {
            RunLogLine::Node { name, .. } => Some(name),
            RunLogLine::Note(_) => None,
        }
    }
}

pub struct ScriptRun {
    pub doc: ScriptSessionId,
    pub generation: u64,
    pub label: String,
    world: World,
    owner: Owner,
    subject: Entity,
    conn: Entity,
    cursor: MessageCursor<ToClients<NotificationSent>>,
    traced: usize,
    pub visited: Vec<String>,
    pub log: Vec<RunLogLine>,
    pub state: ScriptRunState,
    flow: Option<FlowKey>,
    inputs: InputMap,
    pub conversation: Option<Conversation>,
    area: AreaId,
    watched: Watched,
}

#[derive(Default)]
struct Watched {
    memories: Vec<String>,
    quests: Vec<String>,
    items: Vec<String>,
}

pub fn panel(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let bound = bindings(editor, doc);
    let row_key = doc.at.clone();
    let mut setup: RunSetup = editor.views.script.setup_of(&row_key);
    let events: Vec<String> = doc
        .nodes()
        .values()
        .filter_map(nodes::event_name)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !events.contains(&setup.event) {
        setup.event = events.first().cloned().unwrap_or_default();
    }
    let randoms: Vec<(String, usize)> = doc
        .nodes()
        .iter()
        .filter_map(|(name, node)| match node {
            Node::Random { weights, .. } => Some((name.clone(), weights.len())),
            _ => None,
        })
        .collect();
    let characters: Vec<(String, String)> = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
        .map(|content| {
            content
                .ids::<CharacterSnapshot>()
                .map(|id| (id.path(&content).to_owned(), id.get(&content).name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let running = editor.views.script.run_of(&doc.at).is_some();
    let mut start = editor.views.script.take_fire(&doc.at);
    let mut stop = false;
    let mut open_owner: Option<String> = None;
    let mut step = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let can_fire = !bound.is_empty() && !events.is_empty();
        if ui
            .add_enabled_ui(can_fire, |ui| {
                controls::marked_primary(ui, Mark::Play, "Fire")
            })
            .inner
            .clicked()
        {
            start = true;
        }
        controls::combo(("run-event", doc.at.slug()))
            .selected_text(theme::mono(&setup.event).size(13.0))
            .show_ui(ui, |ui| {
                for event in &events {
                    ui.selectable_value(&mut setup.event, event.clone(), event);
                }
            });
        ui.label(theme::muted("on").size(13.0));
        match bound.len() {
            0 => {
                ui.label(theme::muted("no owner").size(13.0));
            }
            1 => {
                let owner = &bound[0];
                let response = ui
                    .horizontal(|ui| {
                        panel::card(true)
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                widgets::badge(ui, owner.row.kind.name());
                                ui.label(theme::mono(&owner.label).size(13.0));
                                ui.label(RichText::new("→").size(12.0).color(theme::ACCENT));
                            });
                    })
                    .response
                    .interact(egui::Sense::click());
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    open_owner = Some(owner.file.clone());
                }
            }
            _ => {
                controls::combo(("run-owner", doc.at.slug()))
                    .selected_text(
                        theme::mono(
                            bound
                                .get(setup.owner)
                                .map(|b| b.label.clone())
                                .unwrap_or_default(),
                        )
                        .size(13.0),
                    )
                    .show_ui(ui, |ui| {
                        for (index, owner) in bound.iter().enumerate() {
                            ui.selectable_value(&mut setup.owner, index, &owner.label);
                        }
                    });
            }
        }
        ui.label(theme::muted("by").size(13.0));
        controls::combo(("run-character", doc.at.slug()))
            .selected_text(
                theme::mono(
                    setup
                        .character
                        .as_ref()
                        .and_then(|key| characters.iter().find(|(k, _)| k == key))
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "pretend character".to_owned()),
                )
                .size(13.0),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut setup.character, None, "pretend character · fresh");
                for (key, name) in &characters {
                    ui.selectable_value(
                        &mut setup.character,
                        Some(key.clone()),
                        format!("{name} · {}", shown_path(key)),
                    );
                }
            });
        if running {
            if controls::marked(ui, Mark::Reload, "Restart").clicked() {
                start = true;
            }
            if ui
                .button(("Step", Mark::CaretRight.text(12.0)))
                .on_hover_text("One tick at a time")
                .clicked()
            {
                step = true;
            }
            if ui
                .button((Mark::Stop.text(12.0), "Stop"))
                .on_hover_text(EditorKey::Cancel.name())
                .clicked()
            {
                stop = true;
            }
        }
        ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
            ui.set_min_width(220.0);
            ui.horizontal(|ui| {
                ui.label(theme::muted("clock"));
                NumberField::new(&mut setup.hour)
                    .range(0.0..=23.99)
                    .unit("h UTC")
                    .show(ui);
            });
            ui.checkbox(&mut setup.step, "pause at every node");
            for (name, outputs) in &randoms {
                ui.horizontal(|ui| {
                    ui.label(theme::muted(format!("{name} takes")));
                    let pick = setup.random.get(name).copied();
                    controls::combo(("run-random", doc.at.slug(), name))
                        .selected_text(pick.map_or("its weights".to_owned(), |index| {
                            format!("output {}", index + 1)
                        }))
                        .show_ui(ui, |ui| {
                            let mut picked = pick;
                            ui.selectable_value(&mut picked, None, "its weights");
                            for index in 0..*outputs {
                                ui.selectable_value(
                                    &mut picked,
                                    Some(index),
                                    format!("output {}", index + 1),
                                );
                            }
                            match picked {
                                Some(index) => {
                                    setup.random.insert(name.clone(), index);
                                }
                                None => {
                                    setup.random.remove(name);
                                }
                            }
                        });
                });
            }
        });
    });
    editor.views.script.set_setup(&row_key, setup.clone());
    if let Some(file) = open_owner
        && let Some((table, key)) = content_of(&file)
    {
        editor.open_row(table, key);
    }
    if stop {
        editor.views.script.run = None;
    }
    if start {
        let owner = bound.get(setup.owner).or(bound.first());
        if let Some(mut run) = owner.and_then(|owner| {
            start_run(editor, doc, owner, &setup)
                .map_err(|e| editor.toast(e))
                .ok()
        }) {
            run.apply_steering(&setup);
            if setup.step {
                run.tick();
                run.read();
            } else {
                run.go();
            }
            editor.views.script.run = Some(run);
        }
    }
    let cards: Vec<NodeCard> = match editor.project.as_ref() {
        Some(project) => doc
            .nodes()
            .iter()
            .map(|(name, node)| NodeCard {
                name: name.clone(),
                title: super::graph::card_title(name, node),
                detail: nodes::summary(node, &doc.text(project)),
            })
            .collect(),
        None => Vec::new(),
    };
    let owner_label = bound
        .get(setup.owner)
        .or(bound.first())
        .map(|b| b.label.clone())
        .unwrap_or_default();
    let Some(run) = editor.views.script.run_of_mut(&row_key) else {
        if bound.is_empty() {
            form::hint(
                ui,
                "Run needs an owner: name this script from an npc, object, item, quest, shop or area first.",
            );
        }
        return;
    };
    if step {
        run.set_step(true);
        run.tick();
        run.read();
    }
    ui.add_space(4.0);
    let state = match &run.state {
        ScriptRunState::Running => "running".to_owned(),
        ScriptRunState::Dialogue => "waiting for a choice".to_owned(),
        ScriptRunState::Waiting { left } => format!("waiting {:.1} s", left.as_secs_f64()),
        ScriptRunState::Paused { what } => format!("paused in {what}"),
        ScriptRunState::Stepping => "stepping".to_owned(),
        ScriptRunState::Ended => "ended".to_owned(),
    };
    let mut skip = false;
    let mut resume = false;
    egui::ScrollArea::vertical()
        .id_salt(("run-trace", doc.at.slug()))
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let mut row_number = 0;
            let total = run.log.len();
            for (index, line) in run.log.iter().enumerate() {
                let last = index + 1 == total;
                let (number, title, detail) = match line {
                    RunLogLine::Node { name, procedure } => {
                        row_number += 1;
                        let card = cards.iter().find(|card| card.name == *name);
                        let title = card.map_or_else(|| name.clone(), |card| card.title.clone());
                        let pieces = card.map(|card| card.detail.clone()).unwrap_or_default();
                        let mut parts: Vec<String> = Vec::new();
                        if title.starts_with("on ") {
                            parts.push(format!(
                                "{owner_label} · {pieces} · a new flow, subject the pretend character"
                            ));
                        } else if !pieces.is_empty() {
                            parts.push(pieces);
                        }
                        if let Some(branch) = branch_taken(doc, name, run.log.get(index + 1)) {
                            parts.push(format!("→ {branch}"));
                        }
                        if !procedure.is_empty() && !procedure.ends_with(&owner_label) {
                            parts.push(format!("in {procedure}"));
                        }
                        if last {
                            parts.push(match (&run.state, &run.conversation) {
                                (ScriptRunState::Dialogue, Some(c)) => format!(
                                    "waiting for a choice · {} shown, {} locked",
                                    c.choices.len(),
                                    c.choices.iter().filter(|c| c.locked).count()
                                ),
                                _ => state.clone(),
                            });
                        }
                        (row_number.to_string(), title, parts.join(" · "))
                    }
                    RunLogLine::Note(note) => (String::new(), String::new(), note.clone()),
                };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        egui::vec2(24.0, 18.0),
                        egui::Label::new(theme::faint(number).monospace().size(12.5)),
                    );
                    if !title.is_empty() {
                        ui.add_sized(
                            egui::vec2(120.0, 18.0),
                            egui::Label::new(theme::mono(&title).size(13.0)).truncate(),
                        );
                    }
                    ui.add(egui::Label::new(theme::muted(detail).monospace().size(12.5)).wrap());
                });
                form::rule(ui);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if matches!(run.state, ScriptRunState::Waiting { .. })
                    && controls::marked(ui, Mark::Skip, "Skip the wait").clicked()
                {
                    skip = true;
                }
                if matches!(run.state, ScriptRunState::Stepping | ScriptRunState::Paused { .. })
                    && ui.button((Mark::Play.text(12.0), "Run on")).clicked()
                {
                    resume = true;
                }
                if run.state == ScriptRunState::Ended {
                    ui.label(
                        theme::muted("The flow ended. Fire again or pick another event.")
                            .size(12.5),
                    );
                }
            });
        });
    if resume {
        run.set_step(false);
        run.go();
    }
    if skip {
        run.skip_wait();
        run.go();
    }
}

struct NodeCard {
    name: String,
    title: String,
    detail: String,
}

pub fn conversation_box(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let owner = doc.at.clone();
    let Some(run) = editor.views.script.run_of_mut(&owner) else {
        return;
    };
    let Some(conversation) = run.conversation.clone() else {
        return;
    };
    let inputs = run.inputs.clone();
    let mut pick: Option<usize> = None;
    let mut leave = false;
    if !ui.ctx().text_edit_focused() {
        if let Some(index) = ui.input(|i| EditorKey::Pick.digit(i))
            && index < conversation.choices.len()
            && !conversation.choices[index].locked
        {
            pick = Some(index);
        }
        if ui.input(|i| EditorKey::Cancel.pressed(i)) {
            leave = true;
        }
    }
    let rect = ui.max_rect();
    let width = 360.0_f32.min(rect.width() - 32.0);
    egui::Area::new(ui.id().with(("conversation-box", owner.slug())))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(rect.max.x - width - 16.0, rect.max.y - 16.0))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .show(ui.ctx(), |ui| {
            ui::slip::show(ui, |ui| {
                ui.set_width(width);
                ui.label(
                    RichText::new("CONVERSATION · the client's box")
                        .size(10.5)
                        .color(theme::FAINT),
                );
                for line in &conversation.lines {
                    let who = dialogue::speaker_name(run.world(), run.subject, line.by);
                    let text = line.text.words(&inputs);
                    let Some(who) = who else {
                        ui.add(
                            egui::Label::new(
                                RichText::new(text).size(13.5).color(theme::TEXT).italics(),
                            )
                            .wrap(),
                        );
                        continue;
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{who}: {text}"))
                                .size(13.5)
                                .color(theme::TEXT),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
                for (index, choice) in conversation.choices.iter().enumerate() {
                    let mut text = format!("{}  {}", index + 1, choice.label.words(&inputs));
                    if let Some(warn) = &choice.warn {
                        text.push_str(&format!("  ({warn})"));
                    }
                    if choice.locked {
                        text.push_str("  locked");
                    }
                    if ui
                        .add_enabled(
                            !choice.locked,
                            egui::Button::new(RichText::new(text).size(13.0)).frame(false),
                        )
                        .clicked()
                    {
                        pick = Some(index);
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(theme::muted("Leave").size(12.5)).frame(false))
                        .clicked()
                    {
                        leave = true;
                    }
                    ui.label(
                        theme::faint(format!(
                            "{} pick · {} leaves",
                            EditorKey::Pick.name(),
                            EditorKey::Cancel.name()
                        ))
                        .size(11.5)
                        .monospace(),
                    );
                });
            });
        });
    if let Some(index) = pick {
        run.pick(index);
        run.go();
    }
    if leave {
        run.leave();
        run.go();
    }
}

pub fn inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) -> bool {
    let row_key = doc.at.clone();
    if editor.views.script.run_of(&row_key).is_none() {
        return false;
    }
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return false;
    };
    let bound = bindings(editor, doc);
    let setup = editor.views.script.setup_of(&row_key);
    let reaches: Vec<Reach> = bound
        .get(setup.owner)
        .or(bound.first())
        .map(|b| content.built::<RiftScripts>().may_do(b.row).reaches)
        .unwrap_or_default();
    let read_memories = NodeReads::of(&doc.reader, doc.nodes()).memories;
    let state_names = inspect::names_of(&doc.script.state);
    let mut open: Option<EditorTab> = None;
    let Some(run) = editor.views.script.run_of_mut(&row_key) else {
        return false;
    };
    let label = run.label.split(" · ").next().unwrap_or("").to_owned();
    let flows = run
        .world()
        .resource::<RiftInstances>()
        .live_flows(run.owner);
    let waiting = match &run.state {
        ScriptRunState::Dialogue => run
            .visited
            .last()
            .map(|node| format!("waiting at {node}"))
            .unwrap_or_default(),
        ScriptRunState::Waiting { left } => format!("waiting {:.0} s", left.as_secs_f64()),
        ScriptRunState::Ended => "ended".to_owned(),
        _ => run
            .visited
            .last()
            .map(|node| format!("at {node}"))
            .unwrap_or_default(),
    };
    sheet::section(ui, "Owner");
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, &label);
        form::kv_text(ui, "state", state_names);
        form::kv_text(
            ui,
            "flows",
            if flows == 0 {
                "none".to_owned()
            } else {
                format!("{flows} · {waiting}")
            },
        );
    });
    sheet::section(ui, "Pretend subject");
    character_card(ui, run, &content, &mut open);
    memories_card(ui, run, &content, &read_memories);
    quests_card(ui, run, &content, &reaches, &mut open);
    bag_card(ui, run, &content, &reaches, &mut open);
    if let Some(tab) = open {
        if let Some((table, key)) = tab.row() {
            editor.select(RowSelection::row(&table, &key));
        }
        editor.open_tab(tab);
    }
    true
}

fn character_card(
    ui: &mut Ui,
    run: &mut ScriptRun,
    content: &Content,
    open: &mut Option<EditorTab>,
) {
    let subject = run.subject;
    let name = run
        .world()
        .get::<Name>(subject)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "character".to_owned());
    let class = run.world().get::<Class>(subject).map(|class| class.def);
    let level = class::level(run.world(), subject);
    let classes: Vec<(ClassId, String)> = content
        .table::<ClassDef>()
        .ids()
        .map(|id| (id, id.path(content).to_owned()))
        .collect();
    let tiers = class
        .map(|class| class.get(content).levels.len())
        .unwrap_or(0);
    let area_key = run.area.path(content).to_owned();
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, &name);
        form::kv(ui, "level", |ui| {
            let mut picked = level;
            controls::combo("run-level")
                .width(ui.available_width() - 8.0)
                .selected_text(theme::mono(level.to_string()))
                .show_ui(ui, |ui| {
                    for tier in 1..=tiers.max(1) as u32 {
                        ui.selectable_value(&mut picked, tier, tier.to_string());
                    }
                });
            if picked != level {
                let xp = class
                    .and_then(|class| class.get(content).levels.get(picked as usize - 1))
                    .map_or(0, |tier| tier.xp);
                if let Some(mut held) = run.world_mut().get_mut::<Xp>(subject) {
                    held.amount = xp;
                }
            }
        });
        form::kv(ui, "class", |ui| {
            let current = class
                .map(|class| class.path(content).to_owned())
                .unwrap_or_else(|| "none".to_owned());
            let mut picked = class;
            controls::combo("run-class")
                .width(ui.available_width() - 8.0)
                .selected_text(theme::mono(&current))
                .show_ui(ui, |ui| {
                    for (id, key) in &classes {
                        ui.selectable_value(&mut picked, Some(*id), key);
                    }
                });
            if picked != class
                && let (Some(def), Some(mut held)) =
                    (picked, run.world_mut().get_mut::<Class>(subject))
            {
                held.def = def;
            }
        });
        form::kv(ui, "area", |ui| {
            if controls::keyed(ui, &format!("area/ {area_key}"), None, true)
                .on_hover_text("Open the area the run plays in")
                .clicked()
            {
                *open = Some(EditorTab::Area {
                    key: area_key.clone(),
                });
            }
        });
    });
}

fn memories_card(ui: &mut Ui, run: &mut ScriptRun, content: &Content, read: &[String]) {
    let subject = run.subject;
    let set: Vec<String> = run
        .world()
        .get::<Memory>(subject)
        .map(|memory| {
            memory
                .keys()
                .map(|id| id.path(content).to_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut shown: Vec<String> = read.to_vec();
    for key in set.iter().chain(&run.watched.memories) {
        if !shown.contains(key) {
            shown.push(key.clone());
        }
    }
    let all: Vec<String> = content.table::<MemoryDef>().paths().to_vec();
    let mut add: Option<String> = None;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "memories");
        if shown.is_empty() {
            ui.label(theme::muted("none set · unset reads as zero").size(13.0));
        }
        for key in &shown {
            let Some(id) = content.by_path::<MemoryDef>(key) else {
                continue;
            };
            let ty = id.get(content).ty;
            let is_set = set.contains(key);
            let value = memory::get(run.world(), subject, id);
            form::kv(ui, shown_path(key), |ui| {
                let mut changed = None;
                match (ty, value) {
                    (ValueType::Boolean, value) => {
                        let mut on = value.truthy();
                        if !is_set {
                            ui.label(theme::muted("unset").size(13.0));
                        }
                        if ui.checkbox(&mut on, "").changed() {
                            changed = Some(ScriptValue::Bool(on));
                        }
                    }
                    (ValueType::Text, value) => {
                        let text = match value {
                            ScriptValue::Text(text) => text,
                            _ => String::new(),
                        };
                        let draft_id = ui.id().with(("run-memory", key));
                        if let Some(typed) = input::drafted(ui, draft_id, &text, |ui, draft| {
                            let field = InputField::new(draft);
                            if is_set { field } else { field.hint("unset") }.show(ui)
                        }) {
                            changed = Some(ScriptValue::Text(typed));
                        }
                    }
                    (ValueType::Integer, value) => {
                        let mut count = match value {
                            ScriptValue::Int(int) => int,
                            ScriptValue::Num(num) => num as i64,
                            _ => 0,
                        };
                        if !is_set {
                            ui.label(theme::muted("unset").size(13.0));
                        }
                        if NumberField::new(&mut count).show(ui) {
                            changed = Some(ScriptValue::Int(count));
                        }
                    }
                    (_, value) => {
                        let mut number = match value {
                            ScriptValue::Int(int) => int as f64,
                            ScriptValue::Num(num) => num,
                            _ => 0.0,
                        };
                        if !is_set {
                            ui.label(theme::muted("unset").size(13.0));
                        }
                        if NumberField::new(&mut number).show(ui) {
                            changed = Some(ScriptValue::Num(number));
                        }
                    }
                }
                if let Some(value) = changed {
                    memory::set(run.world_mut(), subject, id, value);
                }
                if is_set
                    && glyph::button(ui, Mark::Cross, 12.0, theme::FAINT, "Remove")
                        .on_hover_text("Forget it · reads as unset")
                        .clicked()
                    && let Some(mut memory) = run.world_mut().get_mut::<Memory>(subject)
                {
                    memory.forget(id);
                }
            });
        }
        add = add_popup(ui, "+ memory", &all, &shown);
    });
    if let Some(key) = add {
        run.watched.memories.push(key);
    }
}

fn quests_card(
    ui: &mut Ui,
    run: &mut ScriptRun,
    content: &Content,
    reaches: &[Reach],
    open: &mut Option<EditorTab>,
) {
    let subject = run.subject;
    let log = run
        .world()
        .get::<QuestLog>(subject)
        .cloned()
        .unwrap_or_default();
    let mut shown: Vec<String> = reaches
        .iter()
        .filter_map(|reach| match reach {
            Reach::Offers(q)
            | Reach::Accepts(q)
            | Reach::TurnsIn(q)
            | Reach::Completes(q)
            | Reach::Decides(q)
            | Reach::Fails(q) => Some(q.path(content).to_owned()),
            _ => None,
        })
        .collect();
    shown.dedup();
    for key in log
        .active
        .iter()
        .map(|a| a.quest.path(content).to_owned())
        .chain(
            log.finished
                .iter()
                .map(|f| f.quest.path(content).to_owned()),
        )
        .chain(run.watched.quests.iter().cloned())
    {
        if !shown.contains(&key) {
            shown.push(key);
        }
    }
    let all: Vec<String> = content.table::<QuestDef>().paths().to_vec();
    let mut add: Option<String> = None;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "quests");
        if shown.is_empty() {
            ui.label(theme::muted("none taken").size(13.0));
        }
        for key in &shown {
            let Some(id) = content.by_path::<QuestDef>(key) else {
                continue;
            };
            let standing = Standing::of_log(&log, id);
            ui.horizontal(|ui| {
                if controls::keyed(ui, shown_path(key), None, true).clicked() {
                    *open = Some(EditorTab::for_row(QuestDef::TABLE, key));
                }
                let mut picked = standing;
                controls::combo(("run-quest", key))
                    .selected_text(theme::mono(run_label(standing)))
                    .show_ui(ui, |ui| {
                        for &option in Standing::VARIANTS {
                            ui.selectable_value(&mut picked, option, run_label(option));
                        }
                    });
                if picked != standing {
                    set_quest_standing(run.world_mut(), subject, content, id, picked);
                }
            });
        }
        add = add_popup(ui, "+ quest", &all, &shown);
    });
    if let Some(key) = add {
        run.watched.quests.push(key);
    }
}

fn bag_card(
    ui: &mut Ui,
    run: &mut ScriptRun,
    content: &Content,
    reaches: &[Reach],
    open: &mut Option<EditorTab>,
) {
    let subject = run.subject;
    let held: Vec<ItemStack> = run
        .world()
        .get::<Inventory>(subject)
        .map(|bag| bag.slots.clone())
        .unwrap_or_default();
    let mut shown: Vec<String> = reaches
        .iter()
        .filter_map(|reach| match reach {
            Reach::Takes(i) | Reach::Gives(i) | Reach::Drops(i) => Some(i.path(content).to_owned()),
            _ => None,
        })
        .collect();
    shown.dedup();
    for key in held
        .iter()
        .map(|stack| stack.item.path(content).to_owned())
        .chain(run.watched.items.iter().cloned())
    {
        if !shown.contains(&key) {
            shown.push(key);
        }
    }
    let all: Vec<String> = content.table::<ItemDef>().paths().to_vec();
    let mut add: Option<String> = None;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "bag");
        if shown.is_empty() {
            ui.label(theme::muted("empty").size(13.0));
        }
        for key in &shown {
            let Some(id) = content.by_path::<ItemDef>(key) else {
                continue;
            };
            let count = held
                .iter()
                .filter(|stack| stack.item == id)
                .map(|stack| stack.count)
                .sum::<u32>();
            ui.horizontal(|ui| {
                if controls::keyed(ui, shown_path(key), None, true).clicked() {
                    *open = Some(EditorTab::for_row(ItemDef::TABLE, key));
                }
                let mut picked = count;
                if NumberField::new(&mut picked)
                    .range(0..=999)
                    .width(COUNT_WIDTH)
                    .show(ui)
                    && let Some(mut bag) = run.world_mut().get_mut::<Inventory>(subject)
                {
                    bag.set_count(id, picked);
                }
            });
        }
        add = add_popup(ui, "+ item", &all, &shown);
    });
    if let Some(key) = add {
        run.watched.items.push(key);
    }
}

fn add_popup(ui: &mut Ui, label: &str, all: &[String], shown: &[String]) -> Option<String> {
    let mut add = None;
    menu::dropdown(ui, theme::muted(label).size(12.5), |ui| {
        ui.set_min_width(180.0);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                let mut any = false;
                for key in all.iter().filter(|key| !shown.contains(key)) {
                    any = true;
                    if ui
                        .selectable_label(false, theme::mono(shown_path(key)).size(12.5))
                        .clicked()
                    {
                        add = Some(key.clone());
                    }
                }
                if !any {
                    ui.label(theme::faint("everything is listed").size(12.0));
                }
            });
    });
    add
}

fn run_label(standing: Standing) -> &'static str {
    match standing {
        Standing::Active => "active",
        other => other.label(),
    }
}

fn set_quest_standing(
    world: &mut World,
    subject: Entity,
    content: &Content,
    quest: QuestId,
    standing: Standing,
) {
    let day = world.resource::<WallClock>().day();
    let Some(mut log) = world.get_mut::<QuestLog>(subject) else {
        return;
    };
    log.active.retain(|active| active.quest != quest);
    log.finished.retain(|finished| finished.quest != quest);
    log.tracked.retain(|tracked| *tracked != quest);
    match standing {
        Standing::NotTaken => {}
        Standing::Active => log.active.push(ActiveQuest::fresh(content, quest)),
        Standing::Completed => {
            log.finished
                .push(FinishedQuest::new(quest, QuestResult::Completed, day))
        }
        Standing::Failed => log
            .finished
            .push(FinishedQuest::new(quest, QuestResult::Failed, day)),
    }
}

fn branch_taken(doc: &ScriptDoc, node: &str, next: Option<&RunLogLine>) -> Option<String> {
    let next = next?.node()?;
    let source = doc.nodes().get(node)?;
    match source {
        Node::If { .. } | Node::Random { .. } => source
            .exits()
            .into_iter()
            .find(|(_, arrow)| arrow.to() == Some(next))
            .map(|(exit, _)| nodes::exit_label(exit)),
        _ => None,
    }
}

fn start_run(
    editor: &mut Editor,
    doc: &ScriptDoc,
    owner: &ScriptBinding,
    setup: &RunSetup,
) -> Result<ScriptRun, String> {
    let doc_session = editor.views.script.session(&doc.at);
    let project = editor.project.as_ref().ok_or("no project")?;
    let content = project.content.clone().ok_or("the folder does not load")?;
    let generation = project.generation;
    let (area_id, near) = area_for(&content, owner).ok_or("no area to run in")?;
    let clock = WallClock {
        now: UnixMillis((setup.hour * 3600.0) as u64 * 1000),
        reset: UtcHour::MIDNIGHT,
    };
    let mut world = live::sandbox(&content, area_id, RUN_SEED, clock);
    world.insert_resource(FlowTrace::default());
    world.insert_resource(FlowSteering::default());
    let snapshot = snapshot_for(&content, setup.character.as_deref(), &project.reader())?;
    let area = area::load(&content, area_id);
    let at = area
        .mesh()
        .nearest(near)
        .unwrap_or_else(|| area.fallback_spawn());
    let joined = live::join(
        &mut world,
        snapshot,
        Place { area: area_id, at },
        ClientId(CLIENT),
    )?;
    let (conn, subject) = (joined.conn, joined.actor);
    systems::from_client(&mut world, conn, AreaDrawn);
    live::tick(&mut world);
    let world_owner = locate_owner(&mut world, owner, area_id, at)?;
    let inputs = InputMap::new(&content);
    let mut run = ScriptRun {
        doc: doc_session,
        generation,
        label: format!("{} · {}", owner.label, setup.event),
        world,
        owner: world_owner,
        subject,
        conn,
        cursor: MessageCursor::default(),
        traced: 0,
        visited: Vec::new(),
        log: Vec::new(),
        state: ScriptRunState::Running,
        flow: None,
        inputs,
        conversation: None,
        area: area_id,
        watched: Watched::default(),
    };
    run.read();
    run.traced = run.world().resource::<FlowTrace>().steps.len();
    run.visited.clear();
    run.log.clear();
    let event =
        EventKind::from_str(&setup.event).map_err(|_| format!("unknown event {}", setup.event))?;
    let subject = (event != EventKind::Started).then_some(run.subject);
    let raised_on = run.owner;
    run.world_mut().resource_mut::<RiftInstances>().raise(
        raised_on,
        event,
        subject,
        Vec::new(),
        |_| true,
    );
    run.log.push(RunLogLine::Note(format!(
        "{} raised on {}",
        setup.event, owner.label
    )));
    Ok(run)
}

fn area_for(content: &Content, owner: &ScriptBinding) -> Option<(AreaId, Pos<Meters>)> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    match owner.row.kind {
        RunnerKind::Area | RunnerKind::Object | RunnerKind::Blueprint => {
            let area: AreaId = owner.row.id();
            let placed = area::placed(content, area);
            let near = match (owner.row.kind, owner.row.object) {
                (RunnerKind::Object, Some(index)) => match &placed.get(index as usize)?.module {
                    AreaModule::Object(object) => object.shape.bounds().center(),
                    _ => area::load(content, area).center(),
                },
                (RunnerKind::Blueprint, Some(ordinal)) => {
                    area::placements(content, area).get(ordinal as usize)?.at
                }
                _ => area::load(content, area).center(),
            };
            Some((area, near))
        }
        RunnerKind::Npc => {
            let home = area::residence(content, owner.row.id()).unwrap_or(spawn);
            Some((home.area, home.at))
        }
        _ => Some((spawn.area, spawn.at)),
    }
}

fn locate_owner(
    world: &mut World,
    owner: &ScriptBinding,
    area_id: AreaId,
    at: Pos<Meters>,
) -> Result<Owner, String> {
    Ok(match owner.row.kind {
        RunnerKind::Area => Owner::Row(RunnerKind::Area, area_id.raw()),
        RunnerKind::Object => {
            let index = owner.row.object.ok_or("an object without an index")?;
            let entity = world
                .query::<(Entity, &AreaObject)>()
                .iter(world)
                .find(|(_, object)| object.index == index)
                .map(|(entity, _)| entity)
                .ok_or("the object is not in its area")?;
            Owner::Entity(RunnerKind::Object, entity)
        }
        RunnerKind::Npc => {
            let def: NpcId = owner.row.id();
            let found = world
                .query::<(Entity, &Npc)>()
                .iter(world)
                .find(|(_, npc)| npc.def == def)
                .map(|(entity, _)| entity);
            let entity = match found {
                Some(entity) => entity,
                None => npc::spawn(world, def, at, area_id, None),
            };
            Owner::Entity(RunnerKind::Npc, entity)
        }
        RunnerKind::Blueprint => {
            Owner::Row(RunnerKind::Blueprint, owner.row.object.unwrap_or_default())
        }
        kind => Owner::Row(kind, owner.row.row),
    })
}

fn snapshot_for(
    content: &Content,
    key: Option<&str>,
    reader: &FolderReader,
) -> Result<CharacterSnapshot, String> {
    let Some(key) = key else {
        let settings = content.built::<GameSettings>();
        return Ok(CharacterSnapshot::of_player(
            content,
            "Tester",
            settings.player.get(content),
        ));
    };
    reader
        .read_in(key, &ContentScope::of(content))
        .map_err(|_| format!("test character {} does not read", shown_path(key)))
}

impl ScriptRun {
    pub fn world(&self) -> &World {
        &self.world
    }

    fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn apply_steering(&mut self, setup: &RunSetup) {
        let mut steering = self.world_mut().resource_mut::<FlowSteering>();
        steering.step = setup.step;
        steering.random.clear();
        for (name, index) in &setup.random {
            steering.random.insert(name.clone(), *index);
        }
    }

    fn set_step(&mut self, step: bool) {
        self.world_mut().resource_mut::<FlowSteering>().step = step;
    }

    fn tick(&mut self) {
        live::tick(&mut self.world);
    }

    pub fn go(&mut self) {
        for _ in 0..MAX_TICKS {
            self.tick();
            self.read();
            match self.state {
                ScriptRunState::Running => {}
                _ => return,
            }
        }
        self.state = ScriptRunState::Paused {
            what: "a run that never pauses".to_owned(),
        };
    }

    fn pick(&mut self, choice: usize) {
        if let Some(conversation) = &self.conversation {
            let step = conversation.step;
            let conn = self.conn;
            systems::from_client(
                &mut self.world,
                conn,
                ConversationRequest::Pick {
                    step,
                    choice: choice as u32,
                },
            );
        }
    }

    fn leave(&mut self) {
        if let Some(conversation) = &self.conversation {
            let step = conversation.step;
            let conn = self.conn;
            systems::from_client(&mut self.world, conn, ConversationRequest::Leave { step });
        }
    }

    fn skip_wait(&mut self) {
        if let ScriptRunState::Waiting { left } = self.state {
            let jump = left + Duration::from_millis(50);
            self.world_mut()
                .insert_resource(TimeUpdateStrategy::ManualDuration(jump));
            self.tick();
            self.world_mut()
                .insert_resource(TimeUpdateStrategy::ManualDuration(TICK_HZ.period()));
            self.log.push(RunLogLine::Note(format!(
                "skipped {:.1} s",
                left.as_secs_f64()
            )));
        }
    }

    fn read(&mut self) {
        let steps: Vec<TraceStep> = {
            let trace = self.world().resource::<FlowTrace>();
            trace.steps[self.traced.min(trace.steps.len())..].to_vec()
        };
        self.traced += steps.len();
        for step in steps.iter().filter(|step| step.key.owner == self.owner) {
            self.flow = Some(step.key);
            self.visited.push(step.node.to_owned());
            self.log.push(RunLogLine::Node {
                name: step.node.to_owned(),
                procedure: step.procedure.clone(),
            });
        }
        let notices: Vec<RunLogLine> = {
            let messages = self
                .world
                .resource::<Messages<ToClients<NotificationSent>>>();
            let (conn, inputs) = (self.conn, &self.inputs);
            self.cursor
                .read(messages)
                .filter(|message| {
                    matches!(message.targets, SendTargets::Single(Sender::Client(sent)) if sent == conn)
                })
                .map(|message| {
                    let text = message.message.notification.text.words(inputs);
                    RunLogLine::Note(format!("notice: {text}"))
                })
                .collect()
        };
        self.log.extend(notices);
        self.conversation = self.world().get::<Conversation>(self.subject).cloned();
        let now = self.world().resource::<bevy::time::Time>().elapsed();
        let instances = self.world().resource::<RiftInstances>();
        let live = instances.live_flows(self.owner);
        let pause = self.flow.and_then(|key| instances.pause_of(key));
        self.state = if self.conversation.is_some() {
            ScriptRunState::Dialogue
        } else {
            match pause {
                Some(Pause::Wait { until }) => ScriptRunState::Waiting {
                    left: until.saturating_sub(now),
                },
                Some(Pause::NextTick { .. }) => ScriptRunState::Running,
                Some(Pause::Step { .. }) => ScriptRunState::Stepping,
                Some(Pause::Dialogue) => ScriptRunState::Dialogue,
                Some(Pause::Effect { .. }) => ScriptRunState::Paused {
                    what: self.visited.last().cloned().unwrap_or_default(),
                },
                None if live == 0 => ScriptRunState::Ended,
                None => ScriptRunState::Running,
            }
        };
    }
}
