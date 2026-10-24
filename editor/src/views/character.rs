use std::sync::Arc;
use ui::chip;
use ui::egui;
use ui::form;
use ui::input;
use ui::menu;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::{Content, ContentFile, ContentId, FolderReader, fields_file};
use engine::core::math::Pos;
use engine::core::render::TILE;
use engine::core::script::ValueType;
use engine::core::time::Seconds;
use engine::core::units::{Meters, TilePos};
use engine::field;
use engine::systems::area::{self, AreaDef};
use engine::systems::class::{ClassDef, NodeKey, SkillTree};
use engine::systems::equipment::EquipmentSlot;
use engine::systems::item::{ItemDef, ItemKind, ItemStack};
use engine::systems::memory::MemoryDef;
use engine::systems::player::snapshot::{
    AtInitialSpawn, CharacterSnapshot, CharacterStart, MemoryValue, QuestState, SnapshotMemory,
    SnapshotQuest, SnapshotSkill, SnapshotSlot,
};
use engine::systems::quest::{
    Objective, Parties, Party, QuestDef, QuestId, QuestLog, QuestModule, QuestResult,
};
use engine::systems::script::condition as script_condition;
use engine::systems::script::{Place, RiftScripts};
use engine::systems::skill::{HOTBAR_SLOTS, HotbarEntry, HotbarSlot, Rank, SkillDef};
use engine::systems::stat::StatKind;
use serde::Deserialize;
use strum::{AsRefStr, VariantArray};
use ui::controls;
use ui::egui::{Align, Layout, Ui, Vec2};
use ui::glyph::{self, Mark};

use super::area::scene::{AreaSceneRequest, AreaSceneSpec};
use crate::app::{Editor, PlacePick};
use crate::forms::{self, FormAction, FormChange, FormCx};
use crate::project::ContentEdit;
use crate::shell::browser::BrowserAction;
use crate::shell::{row_form, widgets};
use crate::tabs::{EditorTab, file_title, shown_path};
use crate::views::area::AreaViews;
use crate::{picker, play, stage};
use ui::input::{InputField, NumberField};

const NAV: f32 = 150.0;
const COLUMN: f32 = 300.0;
const MAP: Vec2 = Vec2::new(272.0, 96.0);
const SLOT_WIDTH: f32 = 90.0;
const MEMORY_WIDTH: f32 = 140.0;
const HOTBAR_COLUMN: f32 = 440.0;

#[derive(Default)]
pub struct CharacterView {
    pub section: CharacterSection,
    pub more_quests: bool,
    pub more_memories: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize, VariantArray, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum CharacterSection {
    #[default]
    Identity,
    Stats,
    Bag,
    Equipment,
    Skills,
    Hotbar,
    Start,
    #[serde(rename = "areas")]
    #[strum(serialize = "Areas known")]
    AreasKnown,
    Quests,
    Memories,
    #[serde(rename = "lines")]
    #[strum(serialize = "Lines heard")]
    LinesHeard,
    Shops,
    History,
}

impl CharacterSection {
    fn page(self) -> (&'static [CharacterSection], &'static [CharacterSection]) {
        match self {
            CharacterSection::Identity
            | CharacterSection::Bag
            | CharacterSection::Equipment
            | CharacterSection::Start => (
                &[CharacterSection::Bag, CharacterSection::Equipment],
                &[CharacterSection::Identity, CharacterSection::Start],
            ),
            CharacterSection::Stats | CharacterSection::AreasKnown => {
                (&[CharacterSection::Stats], &[CharacterSection::AreasKnown])
            }
            CharacterSection::Skills | CharacterSection::Hotbar => {
                (&[CharacterSection::Skills], &[CharacterSection::Hotbar])
            }
            CharacterSection::Quests | CharacterSection::Memories => {
                (&[CharacterSection::Quests], &[CharacterSection::Memories])
            }
            CharacterSection::LinesHeard | CharacterSection::Shops | CharacterSection::History => (
                &[CharacterSection::LinesHeard, CharacterSection::Shops],
                &[CharacterSection::History],
            ),
        }
    }
}

/// What a section of the character editor reads beside the character itself.
struct Sheet<'a> {
    reader: &'a FolderReader,
    key: &'a str,
}

impl Sheet<'_> {
    fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> String {
        self.reader.path_of(id).unwrap_or_default().to_owned()
    }

    fn read<Row: ContentFile>(&self, cx: &FormCx, id: ContentId<Row>) -> Option<Arc<Row>> {
        cx.project.read::<Row>(self.reader.path_of(id)?).ok()
    }

    /// The id the content loaded last gave a file, as the files stand now.
    fn now<Row: ContentFile>(
        &self,
        loaded: &Content,
        id: ContentId<Row>,
    ) -> Option<ContentId<Row>> {
        self.reader.id_of(id.path(loaded))
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let (Some(character), Some(reader)) = (
        super::read_shown::<CharacterSnapshot>(editor, ui, key),
        editor.reader(),
    ) else {
        return;
    };
    let mut character = (*character).clone();
    let sheet = Sheet {
        reader: &reader,
        key,
    };
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(CharacterSnapshot::TABLE);
    let section = editor.views.character.section;
    let characters = reader.paths_of::<CharacterSnapshot>().to_vec();
    let mut switch: Option<String> = None;
    let mut changed = FormChange::default();
    let mut label = String::new();
    let mut actions = Vec::new();
    let mut pick: Option<PlacePick> = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(NAV);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    theme::faint(shown_path(key).to_uppercase())
                        .monospace()
                        .size(11.5),
                );
                menu::dropdown(ui, theme::faint("other…"), |ui| {
                    for other in &characters {
                        if ui
                            .selectable_label(
                                other == key,
                                theme::mono(shown_path(other)).size(12.5),
                            )
                            .clicked()
                        {
                            switch = Some(other.clone());
                            ui.close();
                        }
                    }
                });
            });
            ui.add_space(6.0);
            for &candidate in CharacterSection::VARIANTS {
                let selected = candidate == section;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(NAV, 26.0), egui::Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        selected,
                        candidate.as_ref(),
                    )
                });
                if selected {
                    paint::row_wash(ui.painter(), rect);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 4.0, theme::RAISED);
                }
                ui.painter().text(
                    rect.left_center() + egui::vec2(10.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    candidate.as_ref(),
                    egui::FontId::proportional(13.5),
                    if selected { theme::TEXT } else { theme::MUTED },
                );
                if response.clicked() {
                    editor.views.character.section = candidate;
                }
            }
        });
        ui.add_space(14.0);
        let preview_width = stage::preview_width(ui.available_width() / 3.0);
        let middle = ui.available_width() - preview_width - 16.0;
        let columns = if middle >= 2.0 * (COLUMN + 32.0) {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        ui.vertical(|ui| {
            ui.set_width(middle);
            let Editor {
                project,
                form,
                shapes,
                views,
                ..
            } = editor;
            let project = project.as_ref().expect("a project");
            let mut cx = FormCx::new(project, root, shapes, form, &mut actions)
                .editing(CharacterSnapshot::TABLE, Some(key));
            let character = &mut character;
            ui.push_id(("character", key), |ui| {
                let (left, right) = section.page();
                ui.with_layout(columns, |ui| {
                    for column in [left, right] {
                        ui.vertical(|ui| {
                            ui.set_width(COLUMN + 16.0);
                            ui.set_max_width(COLUMN + 16.0);
                            for card in column {
                                let result = match card {
                                    CharacterSection::Identity => {
                                        identity(ui, &mut cx, &sheet, character)
                                    }
                                    CharacterSection::Stats => section_of(
                                        ui,
                                        &mut cx,
                                        "Stats",
                                        &mut character.stats,
                                        field!(CharacterSnapshot, stats),
                                    ),
                                    CharacterSection::Bag => bag(ui, &mut cx, &sheet, character),
                                    CharacterSection::Equipment => {
                                        equipment(ui, &mut cx, &sheet, character)
                                    }
                                    CharacterSection::Skills => {
                                        skills(ui, &mut cx, &sheet, character)
                                    }
                                    CharacterSection::Hotbar => hotbar(ui, &mut cx, character),
                                    CharacterSection::Start => start(
                                        ui,
                                        &mut cx,
                                        &sheet,
                                        character,
                                        &mut views.area,
                                        &mut pick,
                                        project.generation,
                                    ),
                                    CharacterSection::AreasKnown => {
                                        areas_known(ui, &sheet, character)
                                    }
                                    CharacterSection::Quests => quests(
                                        ui,
                                        &mut cx,
                                        &sheet,
                                        character,
                                        &mut views.character,
                                        &mut label,
                                    ),
                                    CharacterSection::Memories => memories(
                                        ui,
                                        &mut cx,
                                        &sheet,
                                        character,
                                        &mut views.character,
                                    ),
                                    CharacterSection::LinesHeard => section_of(
                                        ui,
                                        &mut cx,
                                        "Lines heard",
                                        &mut character.lines_heard,
                                        field!(CharacterSnapshot, lines_heard),
                                    ),
                                    CharacterSection::Shops => section_of(
                                        ui,
                                        &mut cx,
                                        "Shops",
                                        &mut character.shops,
                                        field!(CharacterSnapshot, shops),
                                    ),
                                    CharacterSection::History => section_of(
                                        ui,
                                        &mut cx,
                                        "History",
                                        &mut character.history,
                                        field!(CharacterSnapshot, history),
                                    ),
                                };
                                changed |= result;
                                ui.add_space(8.0);
                            }
                        });
                        ui.add_space(16.0);
                    }
                });
            });
        });
        ui.vertical(|ui| {
            ui.set_width(preview_width);
            sheet::section(ui, "preview");
            row_form::preview::show(editor, ui, CharacterSnapshot::TABLE, key);
        });
    });
    editor.status.left.push(format!(
        "test character {} › {}",
        shown_path(key),
        section.as_ref().to_lowercase()
    ));
    editor.status.left.push(match section {
        CharacterSection::Quests => {
            "Set up for… fills bag, counts, memories and start in one step".to_owned()
        }
        CharacterSection::Start => {
            "the start is where Play puts the character; Play here overrides it once".to_owned()
        }
        CharacterSection::Bag => "counts stop at each item's stack size".to_owned(),
        CharacterSection::Skills => {
            "ranks must fit the points the class grants at this xp".to_owned()
        }
        CharacterSection::Hotbar => {
            "slots count from 0; a slot may hold a known skill or a held item".to_owned()
        }
        _ => "every section is what the server keeps for a player".to_owned(),
    });
    if let Some(request) = pick {
        if let Some(area) = request.area.clone() {
            editor.open_tab(EditorTab::Area { key: area });
        }
        editor.pick = Some(request);
    }
    if let Some(what) = changed.0 {
        if label.is_empty() {
            label = format!("Edit {} {}", shown_path(key), what.trim_start_matches('/'));
        }
        // A drag edits a field every frame; those frames are one step to undo.
        editor.write_content_as(ContentEdit::new(label).merging(), key, &character);
    }
    for action in actions {
        row_form::act(editor, action, CharacterSnapshot::TABLE, key);
    }
    if let Some(other) = switch {
        editor.open_row(CharacterSnapshot::TABLE, &other);
    }
}

fn identity(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
) -> FormChange {
    let mut changed = FormChange::default();
    let classes = sheet.reader.paths_of::<ClassDef>();
    let level = level_of(cx, sheet, character);
    card(ui, "Identity", |ui| {
        form::kv(ui, "name", |ui| {
            changed |= forms::of_type(ui, cx, &mut character.name, field!(CharacterSnapshot, name));
        });
        form::kv(ui, "class", |ui| {
            let current = sheet.path_of(character.class);
            let mut picked = current.clone();
            controls::combo("character-class")
                .width(ui.available_width() - 8.0)
                .truncate()
                .selected_text(theme::mono(shown_path(&current)).size(13.0))
                .show_ui(ui, |ui| {
                    for class in classes {
                        ui.selectable_value(&mut picked, class.clone(), shown_path(class));
                    }
                });
            if picked != current
                && let Some(class) = sheet.reader.id_of(&picked)
            {
                character.class = class;
                changed = FormChange::at(field!(CharacterSnapshot, class));
            }
        });
        form::kv(ui, "xp", |ui| {
            let level = format!("level {level}");
            if NumberField::new(&mut character.xp).unit(&level).show(ui) {
                changed = FormChange::at(field!(CharacterSnapshot, xp));
            }
        });
    });
    changed
}

/// A card of the form of one part of the character, drawn from its type.
fn section_of<T>(ui: &mut Ui, cx: &mut FormCx, title: &str, part: &mut T, path: &str) -> FormChange
where
    T: schemars::JsonSchema + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    let mut changed = FormChange::default();
    card(ui, title, |ui| {
        changed = forms::of_type(ui, cx, part, path);
    });
    changed
}

fn bag(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
) -> FormChange {
    let mut changed = FormChange::default();
    let first_item = sheet
        .reader
        .paths_of::<ItemDef>()
        .first()
        .and_then(|item| sheet.reader.id_of::<ItemDef>(item));
    let mut remove = None;
    card(ui, "Bag", |ui| {
        for (index, stack) in character.bag.iter_mut().enumerate() {
            let path = format!("{}/{index}", field!(CharacterSnapshot, bag));
            ui.horizontal(|ui| {
                if controls::remove(ui, "Remove").clicked() {
                    remove = Some(index);
                }
                let mut picked = false;
                ui.scope(|ui| {
                    ui.set_width(200.0);
                    let item = format!("{path}{}", field!(ItemStack, item));
                    picked = picker::content(ui, cx, &mut stack.item, &item);
                });
                let cap = sheet
                    .read(cx, stack.item)
                    .map_or(1, |item| item.stack_max());
                if picked {
                    stack.count = stack.count.clamp(1, cap);
                    changed = FormChange::at(&path);
                }
                let of = format!("/{cap}");
                if NumberField::new(&mut stack.count)
                    .range(1..=cap)
                    .unit(&of)
                    .show(ui)
                {
                    changed = FormChange::at(&path);
                }
            });
        }
        if character.bag.is_empty() {
            ui.label(theme::muted("empty").size(13.0));
        }
        if ui.button("+ item").clicked()
            && let Some(item) = first_item
        {
            character.bag.push(ItemStack { item, count: 1 });
            changed = FormChange::at(field!(CharacterSnapshot, bag));
        }
    });
    if let Some(index) = remove {
        character.bag.remove(index);
        changed = FormChange::at(field!(CharacterSnapshot, bag));
    }
    changed
}

fn equipment(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
) -> FormChange {
    let mut changed = FormChange::default();
    let fits = |slot: EquipmentSlot| -> Vec<String> {
        sheet
            .reader
            .paths_of::<ItemDef>()
            .iter()
            .filter(|item| {
                cx.project.read::<ItemDef>(item).is_ok_and(|item| {
                    matches!(item.kind, ItemKind::Equipment { slot: fits, .. } if fits == slot)
                })
            })
            .cloned()
            .collect()
    };
    let fitting: Vec<(EquipmentSlot, Vec<String>)> = EquipmentSlot::VARIANTS
        .iter()
        .map(|&slot| (slot, fits(slot)))
        .collect();
    card(ui, "Equipment", |ui| {
        for (slot, fitting) in &fitting {
            let current = character
                .equipment
                .get(slot)
                .map(|&item| sheet.path_of(item));
            form::kv(ui, slot.label(), |ui| {
                let mut picked = current.clone();
                controls::combo(("equipment", slot.label()))
                    .width(ui.available_width() - 8.0)
                    .selected_text(match &current {
                        Some(item) => theme::mono(shown_path(item)).size(13.0),
                        None => theme::muted("empty").size(13.0),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut picked, None, "empty");
                        for item in fitting {
                            ui.selectable_value(&mut picked, Some(item.clone()), shown_path(item));
                        }
                    });
                if picked != current {
                    match picked.and_then(|item| sheet.reader.id_of::<ItemDef>(&item)) {
                        Some(item) => character.equipment.insert(*slot, item),
                        None => character.equipment.remove(slot),
                    };
                    changed = FormChange::at(field!(CharacterSnapshot, equipment));
                }
            });
        }
        ui.label(theme::faint("each slot offers only the items made for it").size(12.0));
    });
    changed
}

fn skills(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
) -> FormChange {
    let mut changed = FormChange::default();
    let class = sheet.read(cx, character.class);
    let tree = class
        .as_ref()
        .map_or_else(SkillTree::default, |class| class.tree.clone());
    let granted = class
        .as_ref()
        .map_or(0, |class| class.granted_points(character.xp).0);
    let mut remove = None;
    let mut spent = 0;
    card(ui, "Skills", |ui| {
        for (index, learned) in character.skills.iter_mut().enumerate() {
            let path = format!("{}/{index}", field!(CharacterSnapshot, skills));
            let skill = tree
                .node(learned.node.0.as_str())
                .and_then(|node| sheet.read(cx, node.skill));
            let ranks = skill.as_ref().map_or(1, |skill| skill.rank_count());
            spent += skill
                .as_ref()
                .map_or(0, |skill| skill.cost_to(learned.rank).0);
            ui.horizontal(|ui| {
                if controls::remove(ui, "Forget").clicked() {
                    remove = Some(index);
                }
                let mut picked = learned.node.clone();
                controls::combo(("learned-node", index))
                    .width(170.0)
                    .selected_text(theme::mono(learned.node.as_str()).size(13.0))
                    .show_ui(ui, |ui| {
                        for node in tree.0.keys() {
                            ui.selectable_value(&mut picked, node.clone(), node.as_str());
                        }
                    });
                if picked != learned.node {
                    learned.node = picked;
                    changed = FormChange::at(&path);
                }
                let mut draft = learned.rank.0.min(ranks);
                let of = format!("of {ranks}");
                let typed = NumberField::new(&mut draft)
                    .range(1..=ranks)
                    .unit(&of)
                    .show(ui);
                if draft != learned.rank.0 && (typed || draft < learned.rank.0) {
                    learned.rank = Rank(draft);
                    changed = FormChange::at(&path);
                }
            });
        }
        if character.skills.is_empty() {
            ui.label(theme::muted("nothing learned").size(13.0));
        }
        if tree.is_empty() {
            ui.label(theme::faint("the class has no tree to learn from").size(12.0));
        } else if ui.button("+ skill").clicked()
            && let Some(node) = tree.0.keys().next()
        {
            character.skills.push(SnapshotSkill {
                node: NodeKey(node.0.clone()),
                rank: Rank::FIRST,
            });
            changed = FormChange::at(field!(CharacterSnapshot, skills));
        }
        let line = format!("{spent} of {granted} points spent");
        if spent > granted {
            ui.label(theme::warn(format!(
                "{line} · more than the class grants at this xp"
            )));
        } else {
            ui.label(theme::faint(line).size(12.0));
        }
    });
    if let Some(index) = remove {
        character.skills.remove(index);
        changed = FormChange::at(field!(CharacterSnapshot, skills));
    }
    changed
}

fn hotbar(ui: &mut Ui, cx: &mut FormCx, character: &mut CharacterSnapshot) -> FormChange {
    let mut changed = FormChange::default();
    let mut remove = None;
    panel::card(false).show(ui, |ui| {
        ui.set_width(HOTBAR_COLUMN);
        panel::title(ui, "Hotbar");
        form::kv(ui, "basic attack", |ui| {
            let path = field!(CharacterSnapshot, basic_attack);
            if picker::maybe_content(ui, cx, &mut character.basic_attack, path) {
                changed = FormChange::at(path);
            }
        });
        ui.label(theme::faint("empty, a worn weapon that grants one fills it").size(12.0));
        sheet::section(ui, "slots");
        for (index, entry) in character.hotbar.iter_mut().enumerate() {
            let path = format!("{}/{index}", field!(CharacterSnapshot, hotbar));
            ui.horizontal(|ui| {
                if controls::remove(ui, "Empty the slot").clicked() {
                    remove = Some(index);
                }
                let mut draft = entry.slot.0;
                let typed = NumberField::new(&mut draft)
                    .range(0..=HOTBAR_SLOTS - 1)
                    .unit("slot")
                    .width(SLOT_WIDTH)
                    .show(ui);
                if typed && draft != entry.slot.0 {
                    entry.slot = HotbarSlot(draft);
                    changed = FormChange::at(&path);
                }
                ui.scope(|ui| {
                    ui.set_width(ui.available_width());
                    let at = format!("{path}{}", field!(SnapshotSlot, entry));
                    if hotbar_entry(ui, cx, &mut entry.entry, &at) {
                        changed = FormChange::at(&path);
                    }
                });
            });
        }
        if character.hotbar.is_empty() {
            ui.label(theme::muted("empty").size(13.0));
        }
        if ui.button("+ slot").clicked() {
            let next = character
                .hotbar
                .iter()
                .map(|slot| slot.slot.0 + 1)
                .max()
                .unwrap_or(0);
            character.hotbar.push(SnapshotSlot {
                slot: HotbarSlot(next),
                entry: HotbarEntry::Empty,
            });
            changed = FormChange::at(field!(CharacterSnapshot, hotbar));
        }
    });
    if let Some(index) = remove {
        character.hotbar.remove(index);
        changed = FormChange::at(field!(CharacterSnapshot, hotbar));
    }
    changed
}

/// A field picking what a hotbar slot holds: a skill or an item, either of the files there.
fn hotbar_entry(ui: &mut Ui, cx: &mut FormCx, entry: &mut HotbarEntry, path: &str) -> bool {
    let reader = cx.project.reader();
    let (mut table, mut key) = match *entry {
        HotbarEntry::Empty => (SkillDef::TABLE.to_owned(), String::new()),
        HotbarEntry::Skill(skill) => (
            SkillDef::TABLE.to_owned(),
            reader.path_of(skill).unwrap_or_default().to_owned(),
        ),
        HotbarEntry::Item(item) => (
            ItemDef::TABLE.to_owned(),
            reader.path_of(item).unwrap_or_default().to_owned(),
        ),
    };
    if !picker::row_among(
        ui,
        cx,
        &[SkillDef::TABLE, ItemDef::TABLE],
        &mut table,
        &mut key,
        path,
    ) {
        return false;
    }
    let picked = match table.as_str() {
        SkillDef::TABLE => reader.id_of(&key).map(HotbarEntry::Skill),
        ItemDef::TABLE => reader.id_of(&key).map(HotbarEntry::Item),
        _ => None,
    };
    *entry = picked.unwrap_or(HotbarEntry::Empty);
    true
}

fn start(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
    area: &mut AreaViews,
    pick: &mut Option<PlacePick>,
    generation: u64,
) -> FormChange {
    let mut changed = FormChange::default();
    let scene_key = format!("character:{}", sheet.key);
    let texture = area
        .scenes
        .by_path
        .get(&scene_key)
        .map(|scene| (scene.texture, scene.uv()));
    let spawn = cx.project.settings().ok().map(|settings| {
        (
            sheet.path_of(settings.initial_spawn.area),
            settings.initial_spawn.at,
        )
    });
    let path = field!(CharacterSnapshot, start);
    let (at_spawn, area_key, at) = match character.start {
        CharacterStart::InitialSpawn(_) => {
            let (area, at) = spawn.clone().unwrap_or_default();
            (true, area, at)
        }
        CharacterStart::At(place) => (false, sheet.path_of(place.area), place.at),
    };
    card(ui, "Start", |ui| {
        form::kv(ui, "area", |ui| {
            if at_spawn {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::faint("· the initial spawn").size(12.0));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(theme::mono(shown_path(&area_key)).size(13.0))
                                .truncate(),
                        );
                    });
                });
            } else if let CharacterStart::At(place) = &mut character.start {
                let area = format!("{path}{}", field!(Place, area));
                if picker::content(ui, cx, &mut place.area, &area) {
                    changed = FormChange::at(path);
                }
            }
        });
        form::kv(ui, "at", |ui| {
            if at_spawn {
                ui.label(theme::mono(format!("{}, {}", at.x, at.y)).size(13.0));
                ui.label(theme::faint("m").size(12.0));
            } else if let CharacterStart::At(place) = &mut character.start {
                let (mut x, mut y) = (at.x, at.y);
                if NumberField::of([&mut x, &mut y]).unit("m").show(ui) {
                    place.at = Pos::new(x, y);
                    changed = FormChange::at(path);
                }
            }
        });
        if let Some(content) = cx.content.clone()
            && let Some(area_id) = content.by_path::<AreaDef>(&area_key)
        {
            let size = area::load(&content, area_id).size;
            let (rect, _) = ui.allocate_exact_size(MAP, egui::Sense::hover());
            let ppp = ui.ctx().pixels_per_point();
            let px_per_tile = 8.0;
            let half = MAP / px_per_tile / 2.0;
            let center = Pos::<Meters>::new(
                kept_in_view(at.x, half.x, size.width),
                kept_in_view(at.y, half.y, size.height),
            );
            area.requests.push(AreaSceneRequest {
                key: scene_key.clone(),
                spec: AreaSceneSpec::area(&area_key),
                width: (MAP.x * ppp) as u32,
                height: (MAP.y * ppp) as u32,
                center,
                zoom: px_per_tile / TILE.0 * ppp,
                generation,
                live: true,
            });
            ui.painter().rect_filled(rect, 4.0, theme::INPUT);
            if let Some((texture, uv)) = texture {
                ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            }
            let marker = rect.center()
                + egui::vec2(
                    (at.x - center.x) * px_per_tile,
                    (at.y - center.y) * px_per_tile,
                );
            ui.painter().circle(
                marker,
                4.0,
                theme::WARN,
                egui::Stroke::new(1.5, egui::Color32::BLACK),
            );
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Outside,
            );
        }
        ui.horizontal(|ui| {
            if controls::marked(ui, Mark::Pin, "Pick on the map").clicked() {
                if at_spawn && let Some(area) = sheet.reader.id_of::<AreaDef>(&area_key) {
                    character.start = CharacterStart::At(Place { area, at });
                    changed = FormChange::at(path);
                }
                *pick = Some(PlacePick {
                    file: fields_file(sheet.key),
                    pointer: path.to_owned(),
                    area: Some(area_key.clone()),
                });
            }
            if ui
                .add_enabled(!at_spawn, egui::Button::new("Use the initial spawn"))
                .clicked()
            {
                character.start = CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn);
                changed = FormChange::at(path);
            }
        });
    });
    changed
}

fn areas_known(ui: &mut Ui, sheet: &Sheet, character: &mut CharacterSnapshot) -> FormChange {
    let mut changed = FormChange::default();
    let unknown: Vec<&String> = sheet
        .reader
        .paths_of::<AreaDef>()
        .iter()
        .filter(|area| {
            sheet
                .reader
                .id_of::<AreaDef>(area)
                .is_some_and(|area| !character.known_areas.contains(&area))
        })
        .collect();
    let mut remove = None;
    let mut add = None;
    card(ui, "Areas known", |ui| {
        ui.horizontal_wrapped(|ui| {
            for (index, &area) in character.known_areas.iter().enumerate() {
                let (_, closed) = chip::closable(ui, shown_path(&sheet.path_of(area)));
                if closed {
                    remove = Some(index);
                }
            }
            if character.known_areas.is_empty() {
                ui.label(
                    theme::muted("only the start's area · the map shows nothing else").size(13.0),
                );
            }
        });
        let response = controls::dropdown(ui, "+ area");
        egui::Popup::from_toggle_button_response(&response).show(|ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for area in &unknown {
                        if ui
                            .selectable_label(false, theme::mono(shown_path(area)).size(12.5))
                            .clicked()
                        {
                            add = sheet.reader.id_of::<AreaDef>(area);
                        }
                    }
                });
        });
    });
    if let Some(index) = remove {
        character.known_areas.remove(index);
        changed = FormChange::at(field!(CharacterSnapshot, known_areas));
    }
    if let Some(area) = add {
        character.known_areas.push(area);
        changed = FormChange::at(field!(CharacterSnapshot, known_areas));
    }
    changed
}

#[derive(Clone, Copy, PartialEq, Eq, VariantArray)]
pub enum Standing {
    NotTaken,
    Active,
    Completed,
    Failed,
}

impl Standing {
    pub fn label(self) -> &'static str {
        match self {
            Standing::NotTaken => "not taken",
            Standing::Active => "in progress",
            Standing::Completed => "completed",
            Standing::Failed => "failed",
        }
    }

    pub fn of_log(log: &QuestLog, quest: QuestId) -> Standing {
        if log.active(quest).is_some() {
            return Standing::Active;
        }
        match log.finished(quest).map(|finished| finished.result) {
            Some(QuestResult::Completed) => Standing::Completed,
            Some(QuestResult::Failed) => Standing::Failed,
            None => Standing::NotTaken,
        }
    }

    fn of(state: Option<QuestState>) -> Standing {
        match state {
            Some(QuestState::Active) => Standing::Active,
            Some(QuestState::Completed) => Standing::Completed,
            Some(QuestState::Failed) => Standing::Failed,
            None => Standing::NotTaken,
        }
    }

    fn state(self) -> Option<QuestState> {
        match self {
            Standing::NotTaken => None,
            Standing::Active => Some(QuestState::Active),
            Standing::Completed => Some(QuestState::Completed),
            Standing::Failed => Some(QuestState::Failed),
        }
    }
}

struct ObjectiveLine {
    label: String,
    subject: String,
    held: Option<ContentId<ItemDef>>,
    max: u32,
}

fn objectives_of(cx: &FormCx, sheet: &Sheet, quest: QuestId) -> Vec<ObjectiveLine> {
    let Some(def) = sheet.read(cx, quest) else {
        return Vec::new();
    };
    def.objectives()
        .map(|objective| match objective {
            Objective::Hold(stack) => {
                let item = sheet.path_of(stack.item);
                ObjectiveLine {
                    label: format!("hold {}", shown_path(&item)),
                    subject: shown_path(&item).to_owned(),
                    held: Some(stack.item),
                    max: stack.count,
                }
            }
            Objective::Defeat { npc, count } => {
                let npc = sheet.path_of(*npc);
                ObjectiveLine {
                    label: format!("defeat {}", shown_path(&npc)),
                    subject: shown_path(&npc).to_owned(),
                    held: None,
                    max: *count,
                }
            }
            Objective::Task { label, .. } | Objective::Decide { label, .. } => ObjectiveLine {
                label: label.clone(),
                subject: label.clone(),
                held: None,
                max: 1,
            },
            Objective::Deliver { item } => {
                let item = sheet.path_of(*item);
                ObjectiveLine {
                    label: format!("deliver {}", shown_path(&item)),
                    subject: shown_path(&item).to_owned(),
                    held: None,
                    max: 1,
                }
            }
        })
        .collect()
}

fn quests(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
    view: &mut CharacterView,
    label: &mut String,
) -> FormChange {
    let mut changed = FormChange::default();
    let all: Vec<QuestId> = sheet
        .reader
        .paths_of::<QuestDef>()
        .iter()
        .filter_map(|quest| sheet.reader.id_of(quest))
        .collect();
    let in_log: Vec<QuestId> = character.quests.iter().map(|entry| entry.quest).collect();
    let parties = cx
        .content
        .as_ref()
        .filter(|content| content.has_built::<RiftScripts>())
        .map(|content| {
            (
                content.clone(),
                Parties::of(content, content.built::<RiftScripts>()),
            )
        });
    let mut order = in_log.clone();
    let rest: Vec<QuestId> = all
        .iter()
        .filter(|quest| !in_log.contains(quest))
        .copied()
        .collect();
    let hidden = if view.more_quests {
        0
    } else {
        rest.len().saturating_sub(3)
    };
    order.extend(rest.iter().take(rest.len() - hidden).copied());
    let mut setup: Option<(QuestId, Moment)> = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(COLUMN);
            form::section_count(ui, "Quests", all.len());
            for &quest in &order {
                let path = sheet.path_of(quest);
                let entry = character
                    .quests
                    .iter()
                    .position(|entry| entry.quest == quest);
                let standing = Standing::of(entry.map(|index| character.quests[index].state));
                let giver =
                    giver_of(sheet, parties.as_ref(), quest).map(|party| party.name.clone());
                let requires = sheet.read(cx, quest).and_then(|def| {
                    def.modules.iter().find_map(|module| match module {
                        QuestModule::Requires { condition } => {
                            Some(script_condition::describe(cx.project, condition))
                        }
                        _ => None,
                    })
                });
                let subtitle = match standing {
                    Standing::NotTaken => requires
                        .map(|requires| format!("needs {requires}"))
                        .or_else(|| giver.clone().map(|giver| format!("offered by {giver}")))
                        .unwrap_or_default(),
                    _ => giver
                        .clone()
                        .map(|giver| format!("from {giver}"))
                        .unwrap_or_default(),
                };
                panel::card(standing == Standing::Active).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(theme::mono(shown_path(&path)).size(13.0));
                            if !subtitle.is_empty() {
                                ui.label(theme::faint(subtitle).size(12.0));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let mut picked = standing;
                            controls::combo(("quest-state", &path))
                                .selected_text(theme::mono(standing.label()).size(12.5))
                                .show_ui(ui, |ui| {
                                    for &option in Standing::VARIANTS {
                                        ui.selectable_value(&mut picked, option, option.label());
                                    }
                                });
                            if picked != standing {
                                set_standing(character, quest, picked);
                                changed = FormChange::at(field!(CharacterSnapshot, quests));
                            }
                        });
                    });
                    if standing == Standing::Active
                        && let Some(index) = entry
                        && let Some(entry) = character.quests.get_mut(index)
                    {
                        let objectives = objectives_of(cx, sheet, quest);
                        for (i, objective) in objectives.iter().enumerate() {
                            form::kv(ui, &objective.label, |ui| {
                                let held = entry.progress.get(i).copied().unwrap_or(0);
                                let mut draft = held;
                                let of = format!("/ {}", objective.max);
                                if NumberField::new(&mut draft)
                                    .range(0..=objective.max)
                                    .unit(&of)
                                    .show(ui)
                                {
                                    entry.progress.resize(objectives.len(), 0);
                                    entry.progress[i] = draft;
                                    changed = FormChange::at(field!(CharacterSnapshot, quests));
                                }
                            });
                        }
                    }
                });
            }
            if hidden > 0
                && ui
                    .add(
                        egui::Label::new(
                            theme::muted(format!("… {hidden} more not taken")).size(12.5),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
            {
                view.more_quests = true;
            }
            ui.add_space(6.0);
            let response = controls::dropdown(ui, "Set up for…");
            egui::Popup::from_toggle_button_response(&response)
                .width(300.0)
                .show(|ui| {
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            let subjects: Vec<QuestId> = if in_log.is_empty() {
                                all.iter().take(3).copied().collect()
                            } else {
                                in_log.clone()
                            };
                            for &quest in &subjects {
                                sheet::section(
                                    ui,
                                    &format!("Set up for {}", shown_path(&sheet.path_of(quest))),
                                );
                                for moment in Moment::all(cx, sheet, quest) {
                                    let hint = moment.hint(
                                        cx,
                                        sheet,
                                        quest,
                                        moment.party(sheet, parties.as_ref(), quest),
                                    );
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 36.0),
                                        egui::Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            moment.label(),
                                        )
                                    });
                                    if response.hovered() {
                                        paint::row_wash(ui.painter(), rect);
                                    }
                                    ui.painter().text(
                                        rect.left_top() + egui::vec2(8.0, 4.0),
                                        egui::Align2::LEFT_TOP,
                                        moment.label(),
                                        egui::FontId::proportional(13.0),
                                        theme::TEXT,
                                    );
                                    ui.painter().text(
                                        rect.left_bottom() + egui::vec2(8.0, -4.0),
                                        egui::Align2::LEFT_BOTTOM,
                                        &hint,
                                        egui::FontId::proportional(11.5),
                                        theme::FAINT,
                                    );
                                    if response.clicked() {
                                        setup = Some((quest, moment));
                                        ui.close();
                                    }
                                }
                            }
                        });
                });
        });
    });
    if let Some((quest, moment)) = setup {
        *label = format!(
            "Set up {} for {} · {}",
            shown_path(sheet.key),
            shown_path(&sheet.path_of(quest)),
            moment.label()
        );
        apply_moment(cx, sheet, character, quest, moment, parties.as_ref());
        changed = FormChange::at(field!(CharacterSnapshot, quests));
    }
    changed
}

/// Who offers `quest`, as the content loaded last knows it.
fn giver_of<'p>(
    sheet: &Sheet,
    parties: Option<&'p (Content, Parties)>,
    quest: QuestId,
) -> Option<&'p Party> {
    let (content, parties) = parties?;
    parties.giver(content.by_path::<QuestDef>(&sheet.path_of(quest))?)
}

fn set_standing(character: &mut CharacterSnapshot, quest: QuestId, standing: Standing) {
    character.quests.retain(|entry| entry.quest != quest);
    if let Some(state) = standing.state() {
        character.quests.push(SnapshotQuest {
            quest,
            state,
            progress: Vec::new(),
            left: None,
            decided: None,
        });
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Moment {
    JustOffered,
    ObjectivesMet,
    Decided(String),
    ReadyToHandIn,
    Completed,
    Failed,
}

impl Moment {
    fn all(cx: &FormCx, sheet: &Sheet, quest: QuestId) -> Vec<Moment> {
        let mut moments = vec![Moment::JustOffered, Moment::ObjectivesMet];
        if let Some(def) = sheet.read(cx, quest) {
            for objective in def.objectives() {
                if let Objective::Decide { paths, .. } = objective {
                    moments.extend(
                        paths
                            .iter()
                            .map(|path| Moment::Decided(path.choice.clone())),
                    );
                }
            }
        }
        moments.extend([Moment::ReadyToHandIn, Moment::Completed, Moment::Failed]);
        moments
    }

    fn label(&self) -> String {
        match self {
            Moment::JustOffered => "just offered".to_owned(),
            Moment::ObjectivesMet => "objectives met".to_owned(),
            Moment::Decided(choice) => format!("decided: {choice}"),
            Moment::ReadyToHandIn => "ready to hand in".to_owned(),
            Moment::Completed => "completed".to_owned(),
            Moment::Failed => "failed".to_owned(),
        }
    }

    /// Whom the character stands beside at this moment: who offers the quest before it is taken,
    /// who takes it once it is ready to hand in.
    fn party<'p>(
        &self,
        sheet: &Sheet,
        parties: Option<&'p (Content, Parties)>,
        quest: QuestId,
    ) -> Option<&'p Party> {
        let (content, parties) = parties?;
        let quest = content.by_path::<QuestDef>(&sheet.path_of(quest))?;
        match self {
            Moment::JustOffered => parties.giver(quest),
            Moment::ReadyToHandIn | Moment::Decided(_) => parties.turn_in(quest),
            Moment::ObjectivesMet | Moment::Completed | Moment::Failed => None,
        }
    }

    fn hint(&self, cx: &FormCx, sheet: &Sheet, quest: QuestId, party: Option<&Party>) -> String {
        let by = party
            .map(|party| format!("standing by {}", party.name))
            .unwrap_or_else(|| "at its start".to_owned());
        let done: Vec<String> = objectives_of(cx, sheet, quest)
            .iter()
            .filter(|objective| objective.max > 0)
            .map(|objective| format!("{} {}", objective.max, objective.subject))
            .collect();
        match self {
            Moment::JustOffered => format!("{by}, quest not taken"),
            Moment::ObjectivesMet => {
                if done.is_empty() {
                    "taken, every count full".to_owned()
                } else {
                    format!("taken, {} done", done.join(", "))
                }
            }
            Moment::Decided(choice) => format!("every count full, {choice} chosen"),
            Moment::ReadyToHandIn => format!("every count full, {by}"),
            Moment::Completed => "done and rewarded".to_owned(),
            Moment::Failed => "failed, in the log as such".to_owned(),
        }
    }
}

fn apply_moment(
    cx: &FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
    quest: QuestId,
    moment: Moment,
    parties: Option<&(Content, Parties)>,
) {
    let objectives = objectives_of(cx, sheet, quest);
    let beside_party = moment.party(sheet, parties, quest).and_then(|party| {
        let (content, _) = parties?;
        let home = area::residence(content, party.npc?)?;
        let beside = area::load(content, home.area)
            .grid
            .nearest_walkable_where(home.at, |cell| cell != home.at.cell())?;
        Some(Place {
            area: sheet.now(content, home.area)?,
            at: beside,
        })
    });
    match &moment {
        Moment::JustOffered => set_standing(character, quest, Standing::NotTaken),
        Moment::ObjectivesMet | Moment::ReadyToHandIn | Moment::Decided(_) => {
            set_standing(character, quest, Standing::Active);
            if let Some(entry) = character
                .quests
                .iter_mut()
                .find(|entry| entry.quest == quest)
            {
                entry.progress = objectives.iter().map(|objective| objective.max).collect();
                if let Moment::Decided(choice) = &moment {
                    entry.decided = Some(choice.clone());
                }
            }
            for objective in &objectives {
                if let Some(item) = objective.held {
                    give(character, item, objective.max);
                }
            }
        }
        Moment::Completed => set_standing(character, quest, Standing::Completed),
        Moment::Failed => set_standing(character, quest, Standing::Failed),
    }
    if let Some(place) = beside_party {
        character.start = CharacterStart::At(place);
    }
}

fn give(character: &mut CharacterSnapshot, item: ContentId<ItemDef>, count: u32) {
    match character.bag.iter_mut().find(|stack| stack.item == item) {
        Some(stack) => stack.count = stack.count.max(count),
        None => character.bag.push(ItemStack { item, count }),
    }
}

fn memories(
    ui: &mut Ui,
    cx: &mut FormCx,
    sheet: &Sheet,
    character: &mut CharacterSnapshot,
    view: &mut CharacterView,
) -> FormChange {
    let mut changed = FormChange::default();
    let defs: Vec<(String, Arc<MemoryDef>)> = sheet
        .reader
        .paths_of::<MemoryDef>()
        .iter()
        .filter_map(|key| Some((key.clone(), cx.project.read::<MemoryDef>(key).ok()?)))
        .collect();
    let held = |key: &str| {
        let id = sheet.reader.id_of::<MemoryDef>(key)?;
        character.memories.iter().find(|memory| memory.memory == id)
    };
    let mut order: Vec<&(String, Arc<MemoryDef>)> =
        defs.iter().filter(|(key, _)| held(key).is_some()).collect();
    order.extend(defs.iter().filter(|(key, _)| held(key).is_none()));
    let mut updates: Vec<(String, Option<MemoryValue>)> = Vec::new();
    form::section_count(ui, "Memories", defs.len());
    let hidden = if view.more_memories {
        0
    } else {
        order.len().saturating_sub(6)
    };
    egui::Grid::new("memories")
        .num_columns(4)
        .spacing([10.0, 6.0])
        .min_col_width(10.0)
        .show(ui, |ui| {
            ui.label(theme::faint("MEMORY").size(11.0).monospace());
            ui.label("");
            ui.label(theme::faint("LAST SET").size(11.0).monospace());
            ui.end_row();
            for (key, def) in order.iter().take(order.len() - hidden) {
                let entry = held(key);
                ui.label(theme::mono(shown_path(key)).size(13.0))
                    .on_hover_text(&def.label);
                let value = entry.map(|entry| &entry.value);
                match def.ty {
                    ValueType::Boolean => {
                        let mut on = matches!(value, Some(MemoryValue::Bool(true)));
                        if ui.checkbox(&mut on, "").changed() {
                            updates.push((key.clone(), Some(MemoryValue::Bool(on))));
                        }
                    }
                    ValueType::Text => {
                        let text = match value {
                            Some(MemoryValue::Text(text)) => text.as_str(),
                            _ => "",
                        };
                        let id = ui.id().with(("memory", key));
                        if let Some(typed) = input::drafted(ui, id, text, |ui, draft| {
                            InputField::new(draft).width(MEMORY_WIDTH).show(ui)
                        }) {
                            updates.push((key.clone(), Some(MemoryValue::Text(typed))));
                        }
                    }
                    ValueType::Integer => {
                        let mut count = match value {
                            Some(MemoryValue::Int(count)) => *count,
                            Some(MemoryValue::Num(number)) => *number as i64,
                            _ => 0,
                        };
                        if NumberField::new(&mut count)
                            .unit("times")
                            .width(MEMORY_WIDTH)
                            .show(ui)
                        {
                            updates.push((key.clone(), Some(MemoryValue::Int(count))));
                        }
                    }
                    _ => {
                        let mut number = match value {
                            Some(MemoryValue::Int(count)) => *count as f64,
                            Some(MemoryValue::Num(number)) => *number,
                            _ => 0.0,
                        };
                        if NumberField::new(&mut number)
                            .unit("times")
                            .width(MEMORY_WIDTH)
                            .show(ui)
                        {
                            updates.push((key.clone(), Some(MemoryValue::Num(number))));
                        }
                    }
                }
                ui.label(
                    theme::muted(
                        entry.map_or_else(|| "never".to_owned(), |entry| widgets::ago(entry.ago)),
                    )
                    .size(12.5)
                    .monospace(),
                );
                if entry.is_some()
                    && glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                        .on_hover_text("Unset")
                        .clicked()
                {
                    updates.push((key.clone(), None));
                }
                ui.end_row();
            }
        });
    if hidden > 0
        && ui
            .add(
                egui::Label::new(theme::muted(format!("… {hidden} more")).size(12.5))
                    .sense(egui::Sense::click()),
            )
            .clicked()
    {
        view.more_memories = true;
    }
    for (key, update) in updates {
        let Some(memory) = sheet.reader.id_of::<MemoryDef>(&key) else {
            continue;
        };
        character.memories.retain(|held| held.memory != memory);
        if let Some(value) = update {
            character.memories.push(SnapshotMemory {
                memory,
                value,
                ago: Seconds(0.0),
            });
        }
        changed = FormChange::at(field!(CharacterSnapshot, memories));
    }
    changed
}

fn card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    panel::card(false).show(ui, |ui| {
        ui.set_width(COLUMN);
        panel::title(ui, title);
        body(ui);
    });
}

fn level_of(cx: &FormCx, sheet: &Sheet, character: &CharacterSnapshot) -> u32 {
    sheet
        .read(cx, character.class)
        .map_or(0, |class| class.level_at(character.xp))
}

pub fn preview_extras(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let (Some(Ok(character)), Some(reader), Some(project)) = (
        editor.read::<CharacterSnapshot>(key),
        editor.reader(),
        editor.project.as_ref(),
    ) else {
        return;
    };
    let Ok(settings) = project.settings() else {
        return;
    };
    let read = |path: Option<&str>| project.read::<ClassDef>(path?).ok();
    let class = read(reader.path_of(character.class));
    let xp = character.xp;
    let tiers = class
        .as_ref()
        .map_or(&[][..], |class| class.levels.as_slice());
    let level = class.as_ref().map_or(0, |class| class.level_at(xp));
    let next = tiers
        .iter()
        .find(|tier| tier.xp > xp)
        .map(|tier| format!(" · {xp} of {} xp to {}", tier.xp, level + 1))
        .unwrap_or_else(|| format!(" · {xp} xp · top tier"));
    let base = reader
        .path_of(settings.player)
        .and_then(|player| {
            project
                .read::<engine::systems::player::PlayerDef>(player)
                .ok()
        })
        .map_or(0.0, |player| player.stats.get(StatKind::MaxHealth));
    let from_tiers: f32 = tiers
        .iter()
        .filter(|tier| tier.xp <= xp)
        .map(|tier| tier.stats.get(StatKind::MaxHealth))
        .sum();
    let health = character
        .stats
        .0
        .get(&StatKind::MaxHealth)
        .copied()
        .unwrap_or(base + from_tiers);
    let path_of = |area| shown_path(reader.path_of(area).unwrap_or_default()).to_owned();
    let starts = match character.start {
        CharacterStart::At(place) => {
            format!("{} · {}, {}", path_of(place.area), place.at.x, place.at.y)
        }
        CharacterStart::InitialSpawn(_) => format!(
            "{} · the initial spawn",
            path_of(settings.initial_spawn.area)
        ),
    };
    form::kv_text(ui, "level", format!("{level}{next}"));
    let granted = class.as_ref().map_or(0, |class| class.granted_points(xp).0);
    let spent: u32 = character
        .skills
        .iter()
        .filter_map(|learned| {
            let node = class.as_ref()?.tree.node(learned.node.0.as_str())?;
            let skill = project.read::<SkillDef>(reader.path_of(node.skill)?).ok()?;
            Some(skill.cost_to(learned.rank).0)
        })
        .sum();
    form::kv_text(
        ui,
        "skill points",
        format!(
            "{spent} spent of {granted} · {} left",
            granted.saturating_sub(spent)
        ),
    );
    form::kv_text(ui, "health", format!("{} of {}", health, health));
    form::kv_text(ui, "starts", starts);
    form::kv_code(ui, "file", shown_path(key));
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if controls::marked_primary(ui, Mark::Play, format!("Play as {}", file_title(key)))
            .clicked()
        {
            editor.play.character = Some(key.to_owned());
            editor.settings.last_play = Some(key.to_owned());
            play::start(editor, None);
        }
        if controls::marked(ui, Mark::Duplicate, "Duplicate").clicked() {
            row_form::act(
                editor,
                FormAction::RowAction(BrowserAction::Duplicate),
                CharacterSnapshot::TABLE,
                key,
            );
        }
    });
}

/// Where along an area `extent` tiles long a view reaching `half` tiles each way centres on `at`
/// without showing past the area's edges, or the area's middle when the view is the longer.
fn kept_in_view(at: f32, half: f32, extent: f32) -> f32 {
    if extent <= 2.0 * half {
        extent / 2.0
    } else {
        at.clamp(half, extent - half)
    }
}
