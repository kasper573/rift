use std::collections::{BTreeMap, HashMap};

use engine::core::content::{Content, ContentFile, fields_file, name_of, parent_of};
use engine::field;
use engine::systems::actor::ModelDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::attention::AttentionDef;
use engine::systems::class::{ClassDef, SkillNode};
use engine::systems::interface::{CursorDef, InterfaceIconDef};
use engine::systems::item::ItemDef;
use engine::systems::npc::{self, NpcDef, NpcId};
use engine::systems::quest::QuestDef;
use engine::systems::registry;
use engine::systems::script::{RiftScripts, RunnerKind, ScriptDef};
use engine::systems::skill::SkillDef;

use super::{Candidate, Category, Dragged, Pick, PickLayout, PickSource};
use crate::forms::{FormAction, FormCx};
use crate::icons::{self, KindIcon};
use crate::tabs::file_title;
use crate::tabs::shown_path;

const PICTURE_TABLES: [&str; 6] = [
    ModelDef::TABLE,
    ItemDef::TABLE,
    PropDef::TABLE,
    AttentionDef::TABLE,
    InterfaceIconDef::TABLE,
    CursorDef::TABLE,
];

/// Rows of one or more tables. Several tables become the popover's category chips; a single
/// script table offers the kinds of row a script runs on instead.
pub struct RowSource {
    tables: Vec<String>,
    path: String,
}

impl RowSource {
    pub fn table(table: &str, path: &str) -> RowSource {
        RowSource::tables(&[table], path)
    }

    pub fn tables(tables: &[&str], path: &str) -> RowSource {
        RowSource {
            tables: tables.iter().map(|table| (*table).to_string()).collect(),
            path: path.to_owned(),
        }
    }

    fn several(&self) -> bool {
        self.tables.len() > 1
    }

    fn spoken(&self) -> String {
        self.tables
            .iter()
            .map(|table| table.replace('_', " "))
            .collect::<Vec<_>>()
            .join(" or ")
    }

    fn only_scripts(&self) -> bool {
        self.tables.as_slice() == [ScriptDef::TABLE]
    }
}

impl PickSource for RowSource {
    fn placeholder(&self) -> String {
        self.spoken()
    }

    fn categories(&self, cx: &FormCx) -> Vec<Category> {
        if self.several() {
            return self
                .tables
                .iter()
                .map(|table| Category {
                    id: table.clone(),
                    label: table.replace('_', " "),
                })
                .collect();
        }
        if self.only_scripts() {
            let mut owners: Vec<String> = cx
                .files
                .paths_of(ScriptDef::TABLE)
                .iter()
                .map(|key| script_runner(cx, key))
                .filter(|owner| !owner.is_empty())
                .collect();
            owners.sort();
            owners.dedup();
            return owners
                .into_iter()
                .map(|owner| Category {
                    label: owner.clone(),
                    id: owner,
                })
                .collect();
        }
        Vec::new()
    }

    fn candidates(&self, cx: &FormCx) -> Vec<Candidate> {
        let places = self
            .tables
            .iter()
            .any(|table| table == NpcDef::TABLE)
            .then(|| npc_places(cx))
            .flatten();
        let mut listed = Vec::new();
        for table in &self.tables {
            for key in cx.files.paths_of(table) {
                let key = key.as_str();
                let category = if self.only_scripts() {
                    script_runner(cx, key)
                } else {
                    table.clone()
                };
                let (nearness, from_here) = cx.seen_from_here(key);
                listed.push(Candidate {
                    pick: Pick::new(table.as_str(), key),
                    category,
                    name: display_name(cx, table, key),
                    hint: match pick_hint(cx, places.as_ref(), table, key) {
                        Some(detail) => format!("{} · {detail}", shown_path(&from_here)),
                        None => shown_path(&from_here).to_owned(),
                    },
                    blocked: blocked(cx, table, &self.path, key),
                    nearness,
                });
            }
        }
        listed
    }

    fn accepts(&self, cx: &FormCx, dragged: &Dragged) -> Result<Pick, String> {
        let Dragged::Row { table, key } = dragged else {
            return Err(format!("this field takes {}, not a file", self.spoken()));
        };
        if !self.tables.iter().any(|listed| listed == table) {
            return Err(format!("this field takes {}, not {table}", self.spoken()));
        }
        if let Some(reason) = blocked(cx, table, &self.path, key) {
            return Err(format!("{}: {reason}", file_title(key)));
        }
        Ok(Pick::new(table.as_str(), key.as_str()))
    }

    fn shown(&self, pick: &Pick) -> (String, String) {
        let folder = parent_of(&pick.key);
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        (prefix, file_title(&pick.key).to_owned())
    }

    fn short(&self, pick: &Pick) -> String {
        name_of(&pick.key).to_owned()
    }

    fn icon(&self, pick: &Pick) -> KindIcon {
        icons::of_table(&pick.category)
    }

    fn describe(&self, cx: &FormCx, pick: &Pick) -> String {
        let uses = cx.usages.of_content(&pick.key).len();
        match icons::picture_of(cx.project, &pick.key) {
            Some(picture) => format!("{} · used by {uses}", picture.path),
            None => format!("used by {uses}"),
        }
    }

    fn layout(&self) -> PickLayout {
        let table = self.tables[0].as_str();
        if !self.several() && PICTURE_TABLES.contains(&table) && icons::of_table(table).pictured {
            PickLayout::Grid
        } else {
            PickLayout::List
        }
    }

    fn opens(&self, pick: &Pick) -> Option<FormAction> {
        Some(FormAction::OpenRow {
            table: pick.category.clone(),
            key: pick.key.clone(),
        })
    }

    fn creates(&self, cx: &FormCx) -> Vec<(String, FormAction)> {
        if self.several() || cx.shapes.is_closed(&self.tables[0]) {
            return Vec::new();
        }
        let table = &self.tables[0];
        let spoken = table.replace('_', " ");
        vec![(
            format!("New {spoken} beside it"),
            FormAction::NewRow {
                table: table.clone(),
                path: self.path.clone(),
            },
        )]
    }
}

fn blocked(cx: &FormCx, table: &str, path: &str, key: &str) -> Option<String> {
    match table {
        ScriptDef::TABLE => {
            let kind = match cx.runner {
                Some(kind) => kind,
                None => runner_kind(cx.row_table.as_deref()?),
            };
            let owner = script_runner(cx, key);
            (runner_kind(&owner) != kind).then(|| format!("runs on {owner}"))
        }
        SkillDef::TABLE if path.ends_with(field!(NpcDef, basic_attack)) => {
            let skill = cx.project.read::<SkillDef>(key).ok()?;
            (!skill.basic_attack).then(|| "not flagged as a basic attack".to_owned())
        }
        SkillDef::TABLE if cx.row_table.as_deref() == Some(ClassDef::TABLE) => {
            let node = path
                .strip_prefix(&format!("{}/", field!(ClassDef, tree)))?
                .strip_suffix(field!(SkillNode, skill))?;
            let class = cx.project.read::<ClassDef>(cx.row_key.as_deref()?).ok()?;
            let reader = cx.project.reader();
            let (holder, _) = class.tree.0.iter().find(|(other, held)| {
                other.as_str() != node && reader.path_of(held.skill) == Some(key)
            })?;
            Some(format!("already the node {holder}"))
        }
        NpcDef::TABLE
            if cx.row_table.as_deref() == Some(QuestDef::TABLE) && path.ends_with("/npc") =>
        {
            let content = cx.content.as_ref()?;
            let scripts = content
                .has_built::<RiftScripts>()
                .then(|| content.built::<RiftScripts>())?;
            let id = content.by_path::<NpcDef>(key)?;
            (!npc::can_be_fought(scripts, id)).then(|| "no script sets it hostile".to_owned())
        }
        BlueprintDef::TABLE => {
            crate::views::area::placement::holds_itself(cx.project, cx.row_key.as_deref()?, key)
        }
        _ => None,
    }
}

fn runner_kind(owner: &str) -> RunnerKind {
    owner.parse().unwrap_or(RunnerKind::Any)
}

fn script_runner(cx: &FormCx, key: &str) -> String {
    cx.project.read::<ScriptDef>(key).map_or_else(
        |_| String::new(),
        |script| <&str>::from(script.runs_on).to_owned(),
    )
}

/// What sets the file `key` of `table` apart in the picker, beside its path, for the types that
/// say more about a file than how often it's used.
fn pick_hint(cx: &FormCx, places: Option<&NpcPlaces>, table: &str, key: &str) -> Option<String> {
    match table {
        NpcDef::TABLE => {
            let places = places?;
            let id = places.content.by_path::<NpcDef>(key)?;
            Some(match places.of.get(&id) {
                Some(places) => places.join(" · "),
                None => "placed nowhere".to_owned(),
            })
        }
        ScriptDef::TABLE => Some(script_runner(cx, key)).filter(|runner| !runner.is_empty()),
        _ => None,
    }
}

/// Where each npc stands in the world: every area that places it, and how many.
struct NpcPlaces {
    content: Content,
    of: HashMap<NpcId, Vec<String>>,
}

fn npc_places(cx: &FormCx) -> Option<NpcPlaces> {
    let content = cx.content.clone()?;
    let mut of: HashMap<NpcId, Vec<String>> = HashMap::new();
    for area in content.ids::<AreaDef>() {
        let mut counts: BTreeMap<NpcId, u32> = BTreeMap::new();
        for placed in area::placed(&content, area) {
            match &placed.module {
                AreaModule::Resident(resident) => *counts.entry(resident.npc).or_default() += 1,
                AreaModule::Population(population) => {
                    *counts.entry(population.npc).or_default() += population.count
                }
                _ => {}
            }
        }
        for (npc, count) in counts.into_iter().filter(|(_, count)| *count > 0) {
            of.entry(npc)
                .or_default()
                .push(format!("{} × {count}", shown_path(area.path(&content))));
        }
    }
    Some(NpcPlaces { content, of })
}

fn display_name(cx: &FormCx, table: &str, key: &str) -> String {
    cx.files
        .json(&fields_file(key))
        .and_then(|row| registry().shown_name(table, &row))
        .unwrap_or_default()
}
