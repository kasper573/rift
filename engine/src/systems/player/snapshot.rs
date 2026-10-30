use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentProblem, Loading, NamedContent, Nearby, RowCheck, id, serial,
};
use crate::core::script::{Value, ValueType};
use crate::core::time::{Seconds, WallClock};
use crate::systems::GameSettings;
use crate::systems::actor::{ActorModel, Appearance, Name, appearance};
use crate::systems::area::{self, AreaId, transition::Discovered};
use crate::systems::class::{Class, ClassId, NodeKey};
use crate::systems::dialogue::Heard;
use crate::systems::equipment::{self, Equipment};
use crate::systems::history::{History, HistoryEntry};
use crate::systems::item::{INVENTORY_MAX, Inventory, ItemDef, ItemId, ItemStack};
use crate::systems::memory::{Memory, MemoryId};
use crate::systems::player::{CharacterState, PlayerDef, SpawnPoint, Xp, def};
use crate::systems::quest::{ActiveQuest, FinishedQuest, Parties, QuestId, QuestLog, QuestResult};
use crate::systems::script::Place;
use crate::systems::shop::{Sale, ShopId, ShopLedger};
use crate::systems::skill::learn::{PlanRefusal, plan};
use crate::systems::skill::{
    self, BasicAttack, HOTBAR_SLOTS, Hotbar, HotbarEntry, HotbarSlot, Learned, Rank, SkillId,
};
use crate::systems::stat::Stats;

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterSnapshot {
    pub name: String,
    pub class: ClassId,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub xp: u32,
    #[schemars(extend("x-usage" = "learned ranks, by node of the class tree"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SnapshotSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hotbar: Vec<SnapshotSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_attack: Option<SkillId>,
    #[serde(default, skip_serializing_if = "Stats::is_empty")]
    pub stats: Stats,
    #[schemars(extend("x-usage" = "from the bag"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bag: Vec<ItemStack>,
    /// Put on in order, each where equipping puts it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipped: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_areas: Vec<AreaId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quests: Vec<SnapshotQuest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<SnapshotMemory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines_heard: Vec<HeardLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shops: Vec<SnapshotShop>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<SnapshotRecord>,
    pub start: CharacterStart,
}

impl NamedContent for CharacterSnapshot {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for CharacterSnapshot {
    const TABLE: &'static str = "test_character";
    const PICKED_FROM_TABLE: bool = true;

    fn fresh(near: &Nearby) -> Result<CharacterSnapshot, String> {
        Ok(CharacterSnapshot {
            name: near.name(),
            class: near.file()?,
            appearance: Appearance::of(near.file()?),
            xp: 0,
            skills: Vec::new(),
            hotbar: Vec::new(),
            basic_attack: None,
            stats: Stats::default(),
            bag: Vec::new(),
            equipped: Vec::new(),
            known_areas: Vec::new(),
            quests: Vec::new(),
            memories: Vec::new(),
            lines_heard: Vec::new(),
            shops: Vec::new(),
            history: Vec::new(),
            start: CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn),
        })
    }
}

impl CharacterSnapshot {
    /// A character as the player row makes it: of its class, looking as the class's model does,
    /// with nothing on.
    pub fn of_player(content: &Content, name: &str, def: &PlayerDef) -> CharacterSnapshot {
        CharacterSnapshot {
            name: name.to_owned(),
            class: def.class,
            appearance: Appearance::of(def.class.get(content).model),
            xp: 0,
            skills: Vec::new(),
            hotbar: Vec::new(),
            basic_attack: None,
            stats: Stats::default(),
            bag: Vec::new(),
            equipped: Vec::new(),
            known_areas: Vec::new(),
            quests: Vec::new(),
            memories: Vec::new(),
            lines_heard: Vec::new(),
            shops: Vec::new(),
            history: Vec::new(),
            start: CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn),
        }
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CharacterStart {
    InitialSpawn(AtInitialSpawn),
    At(Place),
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtInitialSpawn {
    InitialSpawn,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSkill {
    pub node: NodeKey,
    pub rank: Rank,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSlot {
    pub slot: HotbarSlot,
    pub entry: HotbarEntry,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotQuest {
    pub quest: QuestId,
    pub state: QuestState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Seconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided: Option<String>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestState {
    Active,
    Completed,
    Failed,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotMemory {
    pub memory: MemoryId,
    pub value: MemoryValue,
    pub ago: Seconds,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MemoryValue {
    Bool(bool),
    Int(i64),
    Num(f64),
    Text(String),
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeardLine {
    pub script: String,
    pub node: String,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotShop {
    pub shop: ShopId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stock: Vec<ShopStock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sales: Vec<Sale>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShopStock {
    pub offer: u32,
    pub left: u32,
    pub restocks_in: Seconds,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRecord {
    pub ago: Seconds,
    pub entry: HistoryEntry,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct JoinAs {
    pub snapshot: CharacterSnapshot,
    pub start: Place,
}

pub fn start_of(content: &Content, snapshot: &CharacterSnapshot) -> Place {
    match snapshot.start {
        CharacterStart::InitialSpawn(_) => content.built::<GameSettings>().initial_spawn,
        CharacterStart::At(place) => place,
    }
}

pub fn materialize(
    content: &Content,
    snapshot: &CharacterSnapshot,
    start: Place,
    clock: WallClock,
) -> CharacterState {
    let mut stats = def(content).stats.clone();
    stats.0.extend(&snapshot.stats.0);
    let mut discovered = Discovered::starting_in(start.area);
    for &known in &snapshot.known_areas {
        discovered.discover(known);
    }
    let mut memory = Memory::default();
    for remembered in &snapshot.memories {
        memory.set(
            content,
            remembered.memory,
            remembered.value.clone().into(),
            WallClock {
                now: clock.now.before(remembered.ago),
                reset: clock.reset,
            },
        );
    }
    let mut heard = Heard::default();
    for line in &snapshot.lines_heard {
        heard.hear(&line.script, &line.node);
    }
    let mut ledger = ShopLedger::default();
    for visited in &snapshot.shops {
        for stock in &visited.stock {
            ledger.stocked(
                visited.shop,
                stock.offer,
                stock.left,
                clock.now.after(stock.restocks_in),
            );
        }
        for sale in &visited.sales {
            ledger.record_sale(visited.shop, sale.clone());
        }
    }
    CharacterState {
        name: Name {
            name: snapshot.name.clone(),
        },
        appearance: snapshot.appearance.clone(),
        spawn: SpawnPoint(start.at),
        discovered,
        stats,
        inventory: Inventory {
            slots: snapshot.bag.clone(),
            max: INVENTORY_MAX,
        },
        xp: Xp {
            amount: snapshot.xp,
        },
        equipment: Equipment {
            slots: equipment::put_on(
                content,
                ActorModel::get(content, snapshot.appearance.model),
                &snapshot.equipped,
            )
            .slots,
        },
        class: Class {
            def: snapshot.class,
        },
        learned: learned(snapshot),
        hotbar: hotbar(snapshot),
        basic_attack: BasicAttack(snapshot.basic_attack),
        memory,
        heard,
        ledger,
        quests: quest_log(content, snapshot, clock),
        history: History::replayed(
            snapshot
                .history
                .iter()
                .map(|record| (clock.now.before(record.ago), record.entry.clone())),
        ),
    }
}

pub async fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, snapshot) in loading.rows::<CharacterSnapshot>(&content) {
        let mut check = RowCheck::default();
        check_start(&content, snapshot, &mut check);
        check_bag(&content, snapshot, &mut check);
        check_equipment(&content, snapshot, &mut check);
        check_skills(&content, snapshot, &mut check);
        check_hotbar(&content, snapshot, &mut check);
        check_quests(&content, snapshot, &mut check);
        check_memories(&content, snapshot, &mut check);
        check_shops(&content, snapshot, &mut check);
        loading.row_errors(id, check);
    }
}

impl From<MemoryValue> for Value {
    fn from(value: MemoryValue) -> Value {
        match value {
            MemoryValue::Bool(flag) => Value::Bool(flag),
            MemoryValue::Int(int) => Value::Int(int),
            MemoryValue::Num(num) => Value::Num(num),
            MemoryValue::Text(text) => Value::Text(text),
        }
    }
}

impl MemoryValue {
    fn fits(&self, ty: ValueType) -> bool {
        matches!(
            (self, ty),
            (MemoryValue::Bool(_), ValueType::Boolean)
                | (MemoryValue::Int(_), ValueType::Integer | ValueType::Number)
                | (MemoryValue::Num(_), ValueType::Number)
                | (MemoryValue::Text(_), ValueType::Text)
        )
    }

    fn describe(&self) -> String {
        match self {
            MemoryValue::Bool(flag) => flag.to_string(),
            MemoryValue::Int(int) => int.to_string(),
            MemoryValue::Num(num) => num.to_string(),
            MemoryValue::Text(text) => format!("{text:?}"),
        }
    }
}

fn learned(snapshot: &CharacterSnapshot) -> Learned {
    Learned(
        snapshot
            .skills
            .iter()
            .map(|learned| (learned.node.clone(), learned.rank))
            .collect(),
    )
}

fn hotbar(snapshot: &CharacterSnapshot) -> Hotbar {
    let mut hotbar = Hotbar::default();
    for held in &snapshot.hotbar {
        hotbar.set(held.slot, held.entry);
    }
    hotbar
}

fn quest_log(content: &Content, snapshot: &CharacterSnapshot, clock: WallClock) -> QuestLog {
    let mut log = QuestLog::default();
    for entry in &snapshot.quests {
        let result = match entry.state {
            QuestState::Active => {
                let mut active = ActiveQuest::fresh(content, entry.quest);
                for (count, given) in active.counts.iter_mut().zip(&entry.progress) {
                    *count = *given;
                }
                active.deadline = entry.left.map(|left| clock.now.after(left));
                active.decided = entry.decided.clone();
                log.active.push(active);
                log.tracked.push(entry.quest);
                continue;
            }
            QuestState::Completed => QuestResult::Completed,
            QuestState::Failed => QuestResult::Failed,
        };
        log.finished
            .push(FinishedQuest::new(entry.quest, result, clock.day()));
    }
    log
}

fn check_start(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    let start = start_of(content, snapshot);
    if let CharacterStart::At(place) = snapshot.start
        && let Err(problem) = area::check_place(content, place)
    {
        check.problem("/start", problem);
    }
    if !snapshot.known_areas.is_empty() && !snapshot.known_areas.contains(&start.area) {
        check.error(
            "/known_areas",
            "start not known",
            start.area.path(content),
            "the known areas to include the start's area",
        );
    }
}

fn check_bag(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    if snapshot.bag.len() > INVENTORY_MAX as usize {
        check.error(
            "/bag",
            "bag too full",
            format!("{} slots", snapshot.bag.len()),
            format!("at most {INVENTORY_MAX} slots"),
        );
    }
    for (index, stack) in snapshot.bag.iter().enumerate() {
        let Some(item) = content.table::<ItemDef>().get(stack.item) else {
            continue;
        };
        let max = item.stack_max();
        if stack.count == 0 {
            check.error(
                format!("/bag/{index}/count"),
                "empty stack",
                0,
                "at least 1",
            );
        } else if stack.count > max {
            check.error(
                format!("/bag/{index}/count"),
                "bag too full",
                format!("{} {} in one slot", stack.count, stack.item.path(content)),
                format!("a stack of at most {max}"),
            );
        }
    }
}

fn check_equipment(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    let Some(model) = ActorModel::loaded(content, snapshot.appearance.model) else {
        return;
    };
    appearance::check(content, &snapshot.appearance, "/appearance", check);
    equipment::check_put_on(content, model, &snapshot.equipped, "/equipped", check);
}

fn check_skills(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    let class = snapshot.class.get(content);
    for (index, learned) in snapshot.skills.iter().enumerate() {
        if snapshot.skills[..index]
            .iter()
            .any(|earlier| earlier.node == learned.node)
        {
            check.error(
                format!("/skills/{index}/node"),
                "node listed twice",
                &learned.node,
                "each node once",
            );
        }
    }
    let picks: Vec<(NodeKey, Rank)> = snapshot
        .skills
        .iter()
        .map(|learned| (learned.node.clone(), learned.rank))
        .collect();
    let level = class.level_at(snapshot.xp);
    let planned = match plan(content, class, &Learned::default(), &picks, level, |_| true) {
        Ok(planned) => planned,
        Err(refusal) => {
            let pick = refusal.pick();
            let (key, rank) = &picks[pick];
            match &refusal {
                PlanRefusal::NotInTree { .. } => check.problem(
                    format!("/skills/{pick}/node"),
                    ContentProblem::new(
                        "unknown node",
                        key,
                        format!("a node of the {} tree", class.name),
                    )
                    .near(id::nearest(
                        class.tree.0.keys().map(|key| &key.0),
                        key.as_str(),
                    )),
                ),
                PlanRefusal::OutOfRange { .. } | PlanRefusal::NotAbove { .. } => {
                    if let Some(node) = class.tree.node(key) {
                        skill::check_rank(
                            check,
                            format!("/skills/{pick}/rank"),
                            content,
                            node.skill,
                            *rank,
                        );
                    }
                }
                PlanRefusal::Unmet { requirement, .. } => check.error(
                    format!("/skills/{pick}"),
                    "not unlocked",
                    refusal.sentence(content, class, &picks),
                    format!(
                        "its requirement met: {}",
                        requirement.describe(content, &class.tree)
                    ),
                ),
            }
            return;
        }
    };
    let granted = class.granted_points(snapshot.xp);
    if planned.cost > granted {
        check.error(
            "/skills",
            "too many ranks",
            format!("{} points spent", planned.cost.0),
            format!("at most {}, granted by level {level}", granted.0),
        );
    }
}

fn check_hotbar(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    let class = snapshot.class.get(content);
    let known = skill::known_from(
        content,
        Some(&class.tree),
        Some(&learned(snapshot)),
        snapshot.equipped.iter().copied(),
        None,
    );
    let held = |item: ItemId| {
        snapshot.bag.iter().any(|stack| stack.item == item) || snapshot.equipped.contains(&item)
    };
    for (index, slot) in snapshot.hotbar.iter().enumerate() {
        if slot.slot.0 >= HOTBAR_SLOTS {
            check.error(
                format!("/hotbar/{index}/slot"),
                "out of range",
                slot.slot.0,
                format!("0 to {}", HOTBAR_SLOTS - 1),
            );
        }
        if snapshot.hotbar[..index]
            .iter()
            .any(|earlier| earlier.slot == slot.slot)
        {
            check.error(
                format!("/hotbar/{index}/slot"),
                "slot listed twice",
                slot.slot.0,
                "each slot once",
            );
        }
        match slot.entry {
            HotbarEntry::Skill(skill) if !known.contains_key(&skill) => check.error(
                format!("/hotbar/{index}/entry"),
                "skill not known",
                skill.path(content),
                "a skill learned in the tree or granted by worn gear",
            ),
            HotbarEntry::Item(item) if !held(item) => check.error(
                format!("/hotbar/{index}/entry"),
                "item not held",
                item.path(content),
                "an item in the bag or worn",
            ),
            _ => {}
        }
    }
    if let Some(skill) = snapshot.basic_attack {
        if !known.contains_key(&skill) {
            check.error(
                "/basic_attack",
                "skill not known",
                skill.path(content),
                "a skill learned in the tree or granted by worn gear",
            );
        } else if !skill.get(content).basic_attack {
            check.error(
                "/basic_attack",
                "not a basic attack",
                skill.path(content),
                "a skill flagged basic_attack",
            );
        }
    }
}

fn check_quests(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    let parties = content.built::<Parties>();
    for (index, entry) in snapshot.quests.iter().enumerate() {
        if snapshot.quests[..index]
            .iter()
            .any(|earlier| earlier.quest == entry.quest)
        {
            check.error(
                format!("/quests/{index}/quest"),
                "quest listed twice",
                entry.quest.path(content),
                "each quest once",
            );
        }
        if entry.state != QuestState::Active || entry.progress.is_empty() {
            continue;
        }
        let def = entry.quest.get(content);
        let objectives = def.objectives().count();
        if entry.progress.len() != objectives {
            check.error(
                format!("/quests/{index}/progress"),
                "progress mismatch",
                format!("{} counts", entry.progress.len()),
                format!("{objectives}, one per objective"),
            );
            continue;
        }
        let needs = def.progress(
            content,
            parties,
            &ActiveQuest::fresh(content, entry.quest),
            None,
        );
        for (objective, (&count, progress)) in entry.progress.iter().zip(needs).enumerate() {
            if count > progress.need {
                check.error(
                    format!("/quests/{index}/progress/{objective}"),
                    "progress out of range",
                    count,
                    format!("0 to {}", progress.need),
                );
            }
        }
    }
}

fn check_memories(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    for (index, remembered) in snapshot.memories.iter().enumerate() {
        let ty = remembered.memory.get(content).ty;
        if !remembered.value.fits(ty) {
            check.error(
                format!("/memories/{index}/value"),
                "memory type",
                remembered.value.describe(),
                format!(
                    "{}, the type of {}",
                    ty.name(),
                    remembered.memory.path(content)
                ),
            );
        }
    }
}

fn check_shops(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    for (index, visited) in snapshot.shops.iter().enumerate() {
        let offers = visited.shop.get(content).sells().count() as u32;
        for (entry, stock) in visited.stock.iter().enumerate() {
            if stock.offer >= offers {
                check.error(
                    format!("/shops/{index}/stock/{entry}/offer"),
                    "unknown offer",
                    stock.offer,
                    format!(
                        "0 to {}, an offer of {}",
                        offers.saturating_sub(1),
                        visited.shop.path(content)
                    ),
                );
            }
        }
    }
}
