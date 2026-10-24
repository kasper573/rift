use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use engine::core::content::id::LeafKind;
use engine::core::content::{Content, ContentFile, fields_file, name_of};
use engine::core::math::{Pos, Size};
use engine::core::script::Expr;
use engine::core::units::Meters;
use engine::systems::GameSettings;
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, AreaModuleKind, ObjectShape};
use engine::systems::item::ItemDef;
use engine::systems::memory::MemoryDef;
use engine::systems::npc::{self, NpcDef, NpcId};
use engine::systems::quest::{Objective, QuestDef, QuestId};
use engine::systems::script::{
    Condition, OwnerRow, Reach, RiftQuery, RiftScripts, RunnerKind, ScriptDef, condition,
};
use engine::systems::shop::ShopDef;
use strum::VariantArray;

use crate::project::ProjectFiles;
use crate::tabs::shown_path;
use crate::usages::ContentUsages;

pub struct WorldAtlas {
    pub generation: u64,
    pub areas: BTreeMap<AreaId, AtlasArea>,
    ids: BTreeMap<String, AreaId>,
    pub warps: Vec<AtlasWarp>,
    pub owners: Vec<AtlasOwner>,
    pub initial_spawn: AreaId,
    pub distances: BTreeMap<AreaId, u32>,
    pub quests: Vec<AtlasQuest>,
    pub lines: Vec<QuestLine>,
    pub module_kinds: Vec<AreaModuleKind>,
    exits: BTreeMap<AreaId, Vec<AreaId>>,
    fought: BTreeSet<NpcId>,
}

pub struct AtlasArea {
    pub key: String,
    pub name: String,
    pub size: Size<Meters>,
    pub modules: HashSet<AreaModuleKind>,
    pub text: String,
    pub residents: Vec<PlacedNpc>,
    pub populations: Vec<PlacedNpc>,
    pub doors: Vec<AtlasDoor>,
    found: HashMap<RowName, Vec<AtlasSource>>,
}

#[derive(Clone)]
pub struct PlacedNpc {
    pub npc: NpcId,
    pub key: String,
    pub at: Pos<Meters>,
    pub count: u32,
}

pub struct AtlasDoor {
    pub at: Pos<Meters>,
    pub owner: usize,
}

pub struct AtlasWarp {
    pub from: AreaId,
    pub to: AreaId,
    pub at: Option<Pos<Meters>>,
    pub landing: Pos<Meters>,
    pub owner: usize,
}

pub struct AtlasOwner {
    pub area: AreaId,
    pub kind: RunnerKind,
    pub name: String,
    pub label: String,
    pub at: Option<Pos<Meters>>,
    pub reaches: Vec<Reach>,
    pub opens: OpenTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpenTarget {
    Area { key: String, module: Option<String> },
    Row { table: &'static str, key: String },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AtlasSource {
    pub kind: AtlasSourceKind,
    pub name: String,
    pub opens: OpenTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AtlasSourceKind {
    Area,
    Warp,
    Placed,
    Spawns,
    Drop,
    Gives,
    Sells,
    Quest,
    Offers,
    Accepts,
    TurnsIn,
    Fails,
    Completes,
    Decides,
    Opens,
    Remembers,
    Runs,
    Named,
    Module,
    Mention,
}

impl AtlasSourceKind {
    pub fn tag(self, count: usize) -> String {
        let (one, many) = match self {
            AtlasSourceKind::Area => ("this area", "this area"),
            AtlasSourceKind::Warp => ("warp", "warps"),
            AtlasSourceKind::Placed => ("placed", "placed"),
            AtlasSourceKind::Spawns => ("spawn", "spawns"),
            AtlasSourceKind::Drop => ("drop", "drops"),
            AtlasSourceKind::Gives => ("gift", "gifts"),
            AtlasSourceKind::Sells | AtlasSourceKind::Opens => ("shop", "shops"),
            AtlasSourceKind::Quest => ("quest", "quests"),
            AtlasSourceKind::Offers => ("offer", "offers"),
            AtlasSourceKind::Accepts => ("start", "starts"),
            AtlasSourceKind::TurnsIn => ("turn-in", "turn-ins"),
            AtlasSourceKind::Fails => ("failure", "failures"),
            AtlasSourceKind::Completes => ("task", "tasks"),
            AtlasSourceKind::Decides => ("decision", "decisions"),
            AtlasSourceKind::Remembers => ("remembered", "remembered"),
            AtlasSourceKind::Runs => ("script", "scripts"),
            AtlasSourceKind::Named => ("named", "named"),
            AtlasSourceKind::Module => ("module", "modules"),
            AtlasSourceKind::Mention => ("mention", "mentions"),
        };
        match count {
            1 => one.to_owned(),
            _ => format!("{count} {many}"),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            AtlasSourceKind::Area => "The area itself",
            AtlasSourceKind::Warp => "Warped to from",
            AtlasSourceKind::Placed => "Placed in",
            AtlasSourceKind::Spawns => "Spawned by",
            AtlasSourceKind::Drop => "Dropped by",
            AtlasSourceKind::Gives => "Given by",
            AtlasSourceKind::Sells => "Sold in",
            AtlasSourceKind::Quest => "Rewarded by",
            AtlasSourceKind::Offers => "Offered by",
            AtlasSourceKind::Accepts => "Started by",
            AtlasSourceKind::TurnsIn => "Turned in at",
            AtlasSourceKind::Fails => "Failed by",
            AtlasSourceKind::Completes => "Tasks done at",
            AtlasSourceKind::Decides => "Decided at",
            AtlasSourceKind::Opens => "Opened by",
            AtlasSourceKind::Remembers => "Remembered by",
            AtlasSourceKind::Runs => "Run by",
            AtlasSourceKind::Named => "Named in",
            AtlasSourceKind::Module => "Has the module",
            AtlasSourceKind::Mention => "Mentioned in",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            _ if self.per_area() => "area",
            AtlasSourceKind::Drop | AtlasSourceKind::Runs => "npc",
            AtlasSourceKind::Sells => "shop",
            AtlasSourceKind::Quest => "quest",
            _ => "owner",
        }
    }

    pub fn per_area(self) -> bool {
        matches!(
            self,
            AtlasSourceKind::Area
                | AtlasSourceKind::Placed
                | AtlasSourceKind::Named
                | AtlasSourceKind::Module
                | AtlasSourceKind::Mention
        )
    }
}

pub struct AtlasQuest {
    pub key: String,
    pub category: String,
    pub steps: Vec<AtlasQuestStep>,
    pub after: Vec<QuestLink>,
    pub requires: Option<String>,
    pub giver: Option<String>,
}

pub struct AtlasQuestStep {
    pub kind: QuestStepKind,
    pub areas: Vec<AreaId>,
    pub who: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestStepKind {
    Offered,
    Goal(String),
    TurnedIn,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestLink {
    pub quest: String,
    pub how: QuestLinkHow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestLinkHow {
    Done,
    During,
    Memory(String),
    Failed,
}

pub struct QuestLine {
    pub category: String,
    pub quests: Vec<String>,
}

pub struct QuestLineStep {
    pub badge: String,
    pub label: String,
    pub areas: Vec<AreaId>,
    pub branch: bool,
}

impl WorldAtlas {
    pub fn build(
        content: &Content,
        files: &ProjectFiles,
        usages: &ContentUsages,
        generation: u64,
    ) -> WorldAtlas {
        let mut atlas = WorldAtlas {
            generation,
            areas: BTreeMap::new(),
            ids: BTreeMap::new(),
            warps: Vec::new(),
            owners: Vec::new(),
            initial_spawn: content.built::<GameSettings>().initial_spawn.area,
            distances: BTreeMap::new(),
            quests: Vec::new(),
            lines: Vec::new(),
            module_kinds: Vec::new(),
            exits: BTreeMap::new(),
            fought: BTreeSet::new(),
        };
        for id in content.ids::<AreaDef>() {
            let info = area_info(content, files, id);
            atlas.ids.insert(info.key.clone(), id);
            atlas.areas.insert(id, info);
        }
        if content.has_built::<RiftScripts>() {
            let scripts = content.built::<RiftScripts>();
            for id in content.ids::<AreaDef>() {
                atlas.add_owners(content, scripts, id);
            }
            atlas.fought = content
                .ids::<NpcDef>()
                .filter(|&id| npc::can_be_fought(scripts, id))
                .collect();
        }
        for warp in &atlas.warps {
            atlas.exits.entry(warp.from).or_default().push(warp.to);
        }
        for exits in atlas.exits.values_mut() {
            exits.sort();
            exits.dedup();
        }
        atlas.module_kinds = AreaModuleKind::VARIANTS
            .iter()
            .copied()
            .filter(|kind| atlas.areas.values().any(|info| info.modules.contains(kind)))
            .collect();
        for id in content.ids::<AreaDef>() {
            atlas.find_sources(content, usages, id);
        }
        atlas.distances = atlas.distances_from(atlas.initial_spawn);
        atlas.build_quests(content);
        atlas
    }

    pub fn id(&self, key: &str) -> Option<AreaId> {
        self.ids.get(key).copied()
    }

    pub fn key(&self, id: AreaId) -> &str {
        &self.areas[&id].key
    }

    /// What a card is called on the map: its file's name, without the folders holding it.
    pub fn file_name(&self, id: AreaId) -> &str {
        name_of(self.key(id))
    }

    pub fn size(&self, id: AreaId) -> Size<Meters> {
        self.areas[&id].size
    }

    pub fn exits(&self, from: AreaId) -> &[AreaId] {
        self.exits.get(&from).map_or(&[], Vec::as_slice)
    }

    pub fn distances_from(&self, start: AreaId) -> BTreeMap<AreaId, u32> {
        let mut distances = BTreeMap::new();
        if !self.areas.contains_key(&start) {
            return distances;
        }
        distances.insert(start, 0);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let here = distances[&current];
            for &next in self.exits(current) {
                if let Entry::Vacant(slot) = distances.entry(next) {
                    slot.insert(here + 1);
                    queue.push_back(next);
                }
            }
        }
        distances
    }

    pub fn route(&self, from: AreaId, to: AreaId) -> Option<Vec<&AtlasWarp>> {
        let mut previous: HashMap<AreaId, AreaId> = HashMap::new();
        let mut queue = VecDeque::from([from]);
        while let Some(current) = queue.pop_front() {
            if current == to {
                let mut hops = Vec::new();
                let mut at = to;
                while at != from {
                    let back = previous[&at];
                    hops.push(self.warp_between(back, at)?);
                    at = back;
                }
                hops.reverse();
                return Some(hops);
            }
            for &next in self.exits(current) {
                if next != from && !previous.contains_key(&next) {
                    previous.insert(next, current);
                    queue.push_back(next);
                }
            }
        }
        None
    }

    pub fn warps_between(&self, from: AreaId, to: AreaId) -> impl Iterator<Item = &AtlasWarp> {
        self.warps
            .iter()
            .filter(move |warp| warp.from == from && warp.to == to)
    }

    pub fn both_ways(&self, warp: &AtlasWarp) -> bool {
        self.exits(warp.to).contains(&warp.from)
    }

    pub fn hostile(&self, npc: NpcId) -> bool {
        self.fought.contains(&npc)
    }

    pub fn givers(&self, area: AreaId) -> impl Iterator<Item = &AtlasOwner> {
        self.owners.iter().filter(move |owner| {
            owner.area == area
                && owner
                    .reaches
                    .iter()
                    .any(|reach| matches!(reach, Reach::Offers(_) | Reach::Accepts(_)))
        })
    }

    pub fn sources(&self, area: AreaId, table: &str, key: &str) -> &[AtlasSource] {
        self.areas
            .get(&area)
            .and_then(|info| info.found.get(&(table.to_owned(), key.to_owned())))
            .map_or(&[], Vec::as_slice)
    }

    pub fn line_steps(&self, category: &str) -> Vec<QuestLineStep> {
        let Some(line) = self.lines.iter().find(|line| line.category == category) else {
            return Vec::new();
        };
        let infos: Vec<&AtlasQuest> = line
            .quests
            .iter()
            .filter_map(|key| self.quests.iter().find(|quest| quest.key == *key))
            .collect();
        let done_required: HashSet<&String> = infos
            .iter()
            .flat_map(|quest| &quest.after)
            .filter(|link| link.how == QuestLinkHow::Done)
            .map(|link| &link.quest)
            .collect();
        let mut out = Vec::new();
        let mut number = 0;
        let mut letter = b'A';
        for (index, info) in infos.iter().enumerate() {
            let giver = info.giver.as_deref().unwrap_or("someone");
            let branch = index > 0
                && !info.after.is_empty()
                && info.after.iter().all(|link| link.how != QuestLinkHow::Done)
                && !done_required.contains(&info.key);
            if branch {
                letter += 1;
                let link = &info.after[0];
                let label = match &link.how {
                    QuestLinkHow::During => {
                        format!(
                            "while {} is open, {giver} offers {}",
                            name_of(&link.quest),
                            name_of(&info.key)
                        )
                    }
                    QuestLinkHow::Memory(memory) => {
                        format!(
                            "after {}, {giver} offers {}",
                            name_of(memory),
                            name_of(&info.key)
                        )
                    }
                    QuestLinkHow::Failed => {
                        format!(
                            "failing {}, {giver} offers {}",
                            name_of(&link.quest),
                            name_of(&info.key)
                        )
                    }
                    QuestLinkHow::Done => format!("{giver} offers {}", name_of(&info.key)),
                };
                let offered = info
                    .steps
                    .first()
                    .filter(|step| step.kind == QuestStepKind::Offered);
                out.push(QuestLineStep {
                    badge: (letter as char).to_string(),
                    label,
                    areas: offered.map(|step| step.areas.clone()).unwrap_or_default(),
                    branch: true,
                });
                continue;
            }
            for step in info
                .steps
                .iter()
                .filter(|step| step.kind != QuestStepKind::Failed)
            {
                number += 1;
                let label = match &step.kind {
                    QuestStepKind::Offered => match &info.requires {
                        Some(requires) => {
                            format!("{giver} offers {} · {requires}", name_of(&info.key))
                        }
                        None => format!("{giver} offers {}", name_of(&info.key)),
                    },
                    QuestStepKind::TurnedIn => {
                        format!("hand in to {}", step.who.as_deref().unwrap_or(giver))
                    }
                    QuestStepKind::Goal(goal) => goal.clone(),
                    QuestStepKind::Failed => continue,
                };
                out.push(QuestLineStep {
                    badge: number.to_string(),
                    label,
                    areas: step.areas.clone(),
                    branch: false,
                });
            }
        }
        out
    }

    pub fn line_areas(&self, category: &str) -> BTreeSet<AreaId> {
        self.line_steps(category)
            .into_iter()
            .flat_map(|step| step.areas)
            .collect()
    }

    fn warp_between(&self, from: AreaId, to: AreaId) -> Option<&AtlasWarp> {
        self.warps_between(from, to)
            .min_by(|a, b| self.owners[a.owner].label.cmp(&self.owners[b.owner].label))
    }

    fn add_owners(&mut self, content: &Content, scripts: &RiftScripts, id: AreaId) {
        let key = id.path(content);
        let placed = area::placed(content, id);
        let mut seeds = vec![AtlasOwner {
            area: id,
            kind: RunnerKind::Area,
            name: name_of(key).to_owned(),
            label: shown_path(key).to_owned(),
            at: None,
            reaches: Vec::new(),
            opens: OpenTarget::Area {
                key: key.to_owned(),
                module: None,
            },
        }];
        let mut rows = vec![OwnerRow {
            kind: RunnerKind::Area,
            row: id.raw(),
            object: None,
        }];
        for (index, object) in area::objects(placed) {
            let name = object
                .name
                .clone()
                .or_else(|| object.title.clone())
                .unwrap_or_else(|| format!("object {index}"));
            seeds.push(AtlasOwner {
                area: id,
                kind: RunnerKind::Object,
                label: format!("{} › {name}", name_of(key)),
                name,
                at: Some(match object.shape {
                    ObjectShape::Point(at) => at,
                    ObjectShape::Rect(rect) => rect.center(),
                }),
                reaches: Vec::new(),
                opens: OpenTarget::Area {
                    key: key.to_owned(),
                    module: Some(placed[index as usize].origin.clone()),
                },
            });
            rows.push(OwnerRow {
                kind: RunnerKind::Object,
                row: id.raw(),
                object: Some(index),
            });
        }
        for (ordinal, placement) in area::placements(content, id).iter().enumerate() {
            if placement.scripts.is_empty() {
                continue;
            }
            let blueprint = placement.blueprint.path(content);
            seeds.push(AtlasOwner {
                area: id,
                kind: RunnerKind::Blueprint,
                name: name_of(blueprint).to_owned(),
                label: format!("{} in {} {ordinal}", name_of(blueprint), name_of(key)),
                at: Some(placement.at),
                reaches: Vec::new(),
                opens: OpenTarget::Area {
                    key: key.to_owned(),
                    module: Some(placement.origin.clone()),
                },
            });
            rows.push(OwnerRow {
                kind: RunnerKind::Blueprint,
                row: id.raw(),
                object: Some(ordinal as u32),
            });
        }
        let mut seen: HashSet<NpcId> = HashSet::new();
        for flattened in placed {
            let (npc, at) = match &flattened.module {
                AreaModule::Resident(resident) => (resident.npc, Some(resident.at)),
                AreaModule::Population(population) => {
                    (population.npc, population.roams.map(|roams| roams.center()))
                }
                _ => continue,
            };
            if !seen.insert(npc) {
                continue;
            }
            let npc_key = npc.path(content);
            seeds.push(AtlasOwner {
                area: id,
                kind: RunnerKind::Npc,
                name: npc.get(content).name.clone(),
                label: shown_path(npc_key).to_owned(),
                at,
                reaches: Vec::new(),
                opens: OpenTarget::Row {
                    table: NpcDef::TABLE,
                    key: npc_key.to_owned(),
                },
            });
            rows.push(OwnerRow {
                kind: RunnerKind::Npc,
                row: npc.raw(),
                object: None,
            });
        }
        for (mut owner, row) in seeds.into_iter().zip(rows) {
            owner.reaches = scripts.may_do(row).reaches;
            let index = self.owners.len();
            for reach in &owner.reaches {
                let Reach::Warp(place) = reach else {
                    continue;
                };
                self.warps.push(AtlasWarp {
                    from: id,
                    to: place.area,
                    at: owner.at,
                    landing: place.at,
                    owner: index,
                });
                if let (Some(at), Some(info)) = (owner.at, self.areas.get_mut(&id)) {
                    info.doors.push(AtlasDoor { at, owner: index });
                }
            }
            self.owners.push(owner);
        }
    }

    fn find_sources(&mut self, content: &Content, usages: &ContentUsages, id: AreaId) {
        let mut found: Vec<(&'static str, String, AtlasSource)> = Vec::new();
        let Some(info) = self.areas.get(&id) else {
            return;
        };
        let here = OpenTarget::Area {
            key: info.key.clone(),
            module: None,
        };
        let source = |kind: AtlasSourceKind, name: &str, opens: &OpenTarget| AtlasSource {
            kind,
            name: name.to_owned(),
            opens: opens.clone(),
        };
        found.push((
            AreaDef::TABLE,
            info.key.clone(),
            source(AtlasSourceKind::Area, &info.key, &here),
        ));
        let mut placed: BTreeMap<NpcId, (String, u32)> = BTreeMap::new();
        for npc in info.residents.iter().chain(&info.populations) {
            placed.entry(npc.npc).or_insert((npc.key.clone(), 0)).1 += npc.count;
        }
        for (npc, (key, count)) in &placed {
            let opens = OpenTarget::Row {
                table: NpcDef::TABLE,
                key: key.clone(),
            };
            found.push((
                NpcDef::TABLE,
                key.clone(),
                source(AtlasSourceKind::Placed, &format!("×{count}"), &here),
            ));
            for loot in npc
                .get(content)
                .loot()
                .into_iter()
                .flat_map(|loot| &loot.items)
            {
                found.push((
                    ItemDef::TABLE,
                    loot.item.path(content).to_owned(),
                    source(AtlasSourceKind::Drop, key, &opens),
                ));
            }
            for quest in content.ids::<QuestDef>() {
                for drop in quest.get(content).drops().filter(|drop| drop.from == *npc) {
                    found.push((
                        ItemDef::TABLE,
                        drop.item.path(content).to_owned(),
                        source(AtlasSourceKind::Drop, key, &opens),
                    ));
                }
            }
            for usage in usages.in_file(&fields_file(key)) {
                if usage.kind == LeafKind::Row(ScriptDef::TABLE) {
                    found.push((
                        ScriptDef::TABLE,
                        usage.value.clone(),
                        source(AtlasSourceKind::Runs, key, &opens),
                    ));
                }
            }
        }
        for owner in self.owners.iter().filter(|owner| owner.area == id) {
            let by = |kind: AtlasSourceKind| source(kind, &owner.name, &owner.opens);
            for reach in &owner.reaches {
                match reach {
                    Reach::Warp(place) => found.push((
                        AreaDef::TABLE,
                        place.area.path(content).to_owned(),
                        by(AtlasSourceKind::Warp),
                    )),
                    Reach::Spawns(npc) => found.push((
                        NpcDef::TABLE,
                        npc.path(content).to_owned(),
                        by(AtlasSourceKind::Spawns),
                    )),
                    Reach::Gives(item) | Reach::Drops(item) => found.push((
                        ItemDef::TABLE,
                        item.path(content).to_owned(),
                        by(AtlasSourceKind::Gives),
                    )),
                    Reach::OpensShop(shop) => {
                        let shop_key = shop.path(content);
                        let opens = OpenTarget::Row {
                            table: ShopDef::TABLE,
                            key: shop_key.to_owned(),
                        };
                        for offer in shop.get(content).sells() {
                            found.push((
                                ItemDef::TABLE,
                                offer.item.path(content).to_owned(),
                                source(AtlasSourceKind::Sells, shop_key, &opens),
                            ));
                        }
                        found.push((
                            ShopDef::TABLE,
                            shop_key.to_owned(),
                            by(AtlasSourceKind::Opens),
                        ));
                    }
                    Reach::Remembers(memory) => found.push((
                        MemoryDef::TABLE,
                        memory.path(content).to_owned(),
                        by(AtlasSourceKind::Remembers),
                    )),
                    _ => {}
                }
                let (kind, quest) = match reach {
                    Reach::Offers(quest) => (AtlasSourceKind::Offers, quest),
                    Reach::Accepts(quest) => (AtlasSourceKind::Accepts, quest),
                    Reach::TurnsIn(quest) => (AtlasSourceKind::TurnsIn, quest),
                    Reach::Fails(quest) => (AtlasSourceKind::Fails, quest),
                    Reach::Completes(quest) => (AtlasSourceKind::Completes, quest),
                    Reach::Decides(quest) => (AtlasSourceKind::Decides, quest),
                    _ => continue,
                };
                let quest_key = quest.path(content);
                found.push((QuestDef::TABLE, quest_key.to_owned(), by(kind)));
                if matches!(kind, AtlasSourceKind::Offers | AtlasSourceKind::TurnsIn) {
                    let def = quest.get(content);
                    let opens = OpenTarget::Row {
                        table: QuestDef::TABLE,
                        key: quest_key.to_owned(),
                    };
                    for stack in def
                        .grants()
                        .iter()
                        .chain(def.rewards())
                        .chain(def.pick_one())
                    {
                        found.push((
                            ItemDef::TABLE,
                            stack.item.path(content).to_owned(),
                            source(AtlasSourceKind::Quest, quest_key, &opens),
                        ));
                    }
                }
            }
        }
        let own_rows = [
            AreaDef::TABLE,
            NpcDef::TABLE,
            ItemDef::TABLE,
            QuestDef::TABLE,
            ShopDef::TABLE,
        ];
        let mut files = vec![fields_file(&info.key)];
        files.extend(
            area::placements(content, id)
                .iter()
                .map(|placement| fields_file(placement.blueprint.path(content))),
        );
        for file in &files {
            for usage in usages.in_file(file) {
                if let LeafKind::Row(table) = usage.kind
                    && !own_rows.contains(&table)
                {
                    found.push((
                        table,
                        usage.value.clone(),
                        source(AtlasSourceKind::Named, &info.key, &here),
                    ));
                }
            }
        }
        let Some(info) = self.areas.get_mut(&id) else {
            return;
        };
        for (table, key, source) in found {
            let sources = info.found.entry((table.to_owned(), key)).or_default();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        for sources in info.found.values_mut() {
            sources.sort();
        }
    }

    fn build_quests(&mut self, content: &Content) {
        let mut quests: Vec<AtlasQuest> = Vec::new();
        for id in content.ids::<QuestDef>() {
            let key = id.path(content).to_owned();
            let def = id.get(content);
            let mut steps = Vec::new();
            let offering = |reach: &Reach| matches!(reach, Reach::Offers(quest) | Reach::Accepts(quest) if *quest == id);
            let giver = self.who_does(&offering);
            let offered = self.areas_where(&offering);
            if !offered.is_empty() {
                steps.push(AtlasQuestStep {
                    kind: QuestStepKind::Offered,
                    areas: offered,
                    who: giver.clone(),
                });
            }
            for objective in def.objectives() {
                let (goal, areas) = match objective {
                    Objective::Defeat { npc, count } => {
                        let areas = self
                            .areas
                            .iter()
                            .filter(|(_, info)| {
                                info.residents
                                    .iter()
                                    .chain(&info.populations)
                                    .any(|placed| placed.npc == *npc)
                            })
                            .map(|(area, _)| *area)
                            .collect();
                        (counted("defeat", *count, npc.path(content)), areas)
                    }
                    Objective::Hold(stack) => (
                        counted("hold", stack.count, stack.item.path(content)),
                        self.areas_finding(ItemDef::TABLE, stack.item.path(content)),
                    ),
                    Objective::Deliver { item } => (
                        counted("deliver", 1, item.path(content)),
                        self.areas_finding(ItemDef::TABLE, item.path(content)),
                    ),
                    Objective::Task { label, .. } => (
                        label.clone(),
                        self.areas_where(
                            &|reach| matches!(reach, Reach::Completes(quest) if *quest == id),
                        ),
                    ),
                    Objective::Decide { label, .. } => (
                        label.clone(),
                        self.areas_where(
                            &|reach| matches!(reach, Reach::Decides(quest) if *quest == id),
                        ),
                    ),
                };
                steps.push(AtlasQuestStep {
                    kind: QuestStepKind::Goal(goal),
                    areas,
                    who: None,
                });
            }
            let turning_in = |reach: &Reach| matches!(reach, Reach::TurnsIn(quest) if *quest == id);
            let turn_in = self.areas_where(&turning_in);
            if !turn_in.is_empty() {
                steps.push(AtlasQuestStep {
                    kind: QuestStepKind::TurnedIn,
                    areas: turn_in,
                    who: self.who_does(&turning_in),
                });
            }
            let failed =
                self.areas_where(&|reach| matches!(reach, Reach::Fails(quest) if *quest == id));
            if !failed.is_empty() {
                steps.push(AtlasQuestStep {
                    kind: QuestStepKind::Failed,
                    areas: failed,
                    who: None,
                });
            }
            let requires: Vec<String> = [def.shown(), def.requires()]
                .into_iter()
                .flatten()
                .map(|condition| condition::describe(content, condition))
                .collect();
            quests.push(AtlasQuest {
                key,
                category: def.category.clone(),
                steps,
                after: self.links_of(content, id),
                requires: (!requires.is_empty()).then(|| requires.join(" · ")),
                giver,
            });
        }
        let categories: BTreeSet<&String> = quests.iter().map(|quest| &quest.category).collect();
        let mut lines = Vec::new();
        for category in categories {
            let mut ordered: Vec<String> = Vec::new();
            let mut pending: Vec<&AtlasQuest> = quests
                .iter()
                .filter(|quest| quest.category == *category)
                .collect();
            while !pending.is_empty() {
                let next = pending
                    .iter()
                    .position(|quest| {
                        quest.after.iter().all(|link| {
                            ordered.contains(&link.quest)
                                || !pending.iter().any(|other| other.key == link.quest)
                        })
                    })
                    .unwrap_or(0);
                ordered.push(pending.remove(next).key.clone());
            }
            lines.push(QuestLine {
                category: category.clone(),
                quests: ordered,
            });
        }
        self.lines = lines;
        self.quests = quests;
    }

    fn links_of(&self, content: &Content, id: QuestId) -> Vec<QuestLink> {
        let def = id.get(content);
        let mut queries = Vec::new();
        for condition in [def.shown(), def.requires()].into_iter().flatten() {
            positive_queries(condition, &mut queries);
        }
        let mut after: Vec<QuestLink> = Vec::new();
        let mut link = |quest: QuestId, how: QuestLinkHow| {
            let quest = quest.path(content).to_owned();
            if !after.iter().any(|known| known.quest == quest) {
                after.push(QuestLink { quest, how });
            }
        };
        for query in queries {
            match query {
                RiftQuery::OnQuest { quest, .. } if *quest != id => {
                    link(*quest, QuestLinkHow::During)
                }
                RiftQuery::QuestDone { quest, .. } if *quest != id => {
                    link(*quest, QuestLinkHow::Done)
                }
                RiftQuery::Memory { memory, .. } | RiftQuery::SetWithin { memory, .. } => {
                    for owner in self
                        .owners
                        .iter()
                        .filter(|owner| owner.reaches.contains(&Reach::Remembers(*memory)))
                    {
                        for reach in &owner.reaches {
                            if let Reach::Offers(other)
                            | Reach::Accepts(other)
                            | Reach::TurnsIn(other)
                            | Reach::Completes(other)
                            | Reach::Decides(other) = reach
                                && *other != id
                            {
                                link(
                                    *other,
                                    QuestLinkHow::Memory(memory.path(content).to_owned()),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for owner in self.owners.iter().filter(|owner| {
            owner
                .reaches
                .iter()
                .any(|reach| matches!(reach, Reach::Offers(quest) | Reach::Accepts(quest) if *quest == id))
        }) {
            for reach in &owner.reaches {
                if let Reach::Fails(other) = reach
                    && *other != id
                {
                    link(*other, QuestLinkHow::Failed);
                }
            }
        }
        after
    }

    fn areas_where(&self, pick: &dyn Fn(&Reach) -> bool) -> Vec<AreaId> {
        let areas: BTreeSet<AreaId> = self
            .owners
            .iter()
            .filter(|owner| owner.reaches.iter().any(pick))
            .map(|owner| owner.area)
            .collect();
        areas.into_iter().collect()
    }

    fn who_does(&self, pick: &dyn Fn(&Reach) -> bool) -> Option<String> {
        self.owners
            .iter()
            .find(|owner| owner.reaches.iter().any(pick))
            .map(|owner| owner.name.clone())
    }

    fn areas_finding(&self, table: &str, key: &str) -> Vec<AreaId> {
        self.areas
            .keys()
            .copied()
            .filter(|area| {
                self.sources(*area, table, key).iter().any(|source| {
                    matches!(source.kind, AtlasSourceKind::Drop | AtlasSourceKind::Gives)
                })
            })
            .collect()
    }
}

impl AtlasArea {
    pub fn rows_found<'a>(&'a self, table: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.found
            .keys()
            .filter(move |(found, _)| found == table)
            .map(|(_, key)| key.as_str())
    }
}

type RowName = (String, String);

fn area_info(content: &Content, files: &ProjectFiles, id: AreaId) -> AtlasArea {
    let key = id.path(content).to_owned();
    let def = id.get(content);
    let loaded = area::load(content, id);
    let placed = area::placed(content, id);
    let mut text = files
        .json(&fields_file(&key))
        .map(|row| row.to_string().to_lowercase())
        .unwrap_or_default();
    for placement in area::placements(content, id) {
        if let Some(row) = files.json(&fields_file(placement.blueprint.path(content))) {
            text.push(' ');
            text.push_str(&row.to_string().to_lowercase());
        }
    }
    let mut residents = Vec::new();
    let mut populations = Vec::new();
    for flattened in placed {
        match &flattened.module {
            AreaModule::Resident(resident) => residents.push(PlacedNpc {
                npc: resident.npc,
                key: resident.npc.path(content).to_owned(),
                at: resident.at,
                count: 1,
            }),
            AreaModule::Population(population) => populations.push(PlacedNpc {
                npc: population.npc,
                key: population.npc.path(content).to_owned(),
                at: population
                    .roams
                    .map_or_else(|| loaded.center(), |roams| roams.center()),
                count: population.count,
            }),
            _ => {}
        }
    }
    AtlasArea {
        key,
        name: def.name.clone(),
        size: loaded.size,
        modules: placed
            .iter()
            .map(|flattened| flattened.module.kind())
            .collect(),
        text,
        residents,
        populations,
        doors: Vec::new(),
        found: HashMap::new(),
    }
}

fn positive_queries<'a>(condition: &'a Condition, out: &mut Vec<&'a RiftQuery>) {
    match condition {
        Expr::Not { .. } => {}
        Expr::Query(query) => out.push(query),
        other => {
            for child in other.children() {
                positive_queries(child, out);
            }
        }
    }
}

fn counted(verb: &str, count: u32, what: &str) -> String {
    match count {
        1 => format!("{verb} {}", name_of(what)),
        _ => format!("{verb} {count} {}", name_of(what)),
    }
}
