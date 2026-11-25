use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::content::{
    Content, ContentFile, ContentProblem, ContentScope, Loading, NamedContent, Nearby, RowCheck,
    SavedList, SetAside, id, serial, with_content,
};
use crate::core::math::Pos;
use crate::core::script::{Value, ValueType};
use crate::core::time::{GameDay, UnixMillis, WallClock};
use crate::core::units::Seconds;
use crate::systems::account::AccountId;
use crate::systems::actor::{ActorModel, Appearance, Name, appearance};
use crate::systems::area::Place;
use crate::systems::area::{self, AreaId, transition::Discovered};
use crate::systems::area_instance::AreaInstanceWorld;
use crate::systems::area_instance::InstanceCooldowns;
use crate::systems::class::{Class, ClassId, NodeKey};
use crate::systems::command::{CommandCooldowns, TerminalCommandId};
use crate::systems::crafting::{self, RecipeKey, RecipeMasteries};
use crate::systems::dialogue::Heard;
use crate::systems::equipment::{self, Equipment};
use crate::systems::history::{History, HistoryEntry};
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::memory::{Memory, MemoryId};
use crate::systems::movement::Position;
use crate::systems::player::{
    CharacterAccount, CharacterDef, CharacterState, SpawnPoint, StandIn, Xp, def,
};
use crate::systems::quest::{
    ActiveQuest, FinishedQuest, Objective, QuestId, QuestLog, QuestResult,
};
use crate::systems::shop::{Sale, ShopId, ShopLedger};
use crate::systems::skill::learn::{PlanRefusal, plan};
use crate::systems::skill::{
    self, BasicAttack, Cooldowns, Hotbar, HotbarEntry, HotbarSlot, Learned, Rank, SkillId,
};
use crate::systems::stat::{self, Stat, Stats};
use crate::systems::status::{
    CarriedStatus, CarriedStatuses, StatusBeat, StatusEffectId, StatusInstance, Statuses,
};
use crate::systems::{GameSettings, WorldArea};

/// A character, whole: what a test character is staged from, and what a played one is saved as.
/// It holds every moment as time ago or time left, counted from when it was taken.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterSnapshot {
    pub name: String,
    pub class: ClassId,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub xp: u32,
    #[schemars(transform = SchemaMark::Usage.with("learned ranks, by node of the class tree"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SnapshotSkill>,
    /// The recipes mastered for good, besides those the skills give.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub recipe_masteries: SavedList<RecipeKey>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub hotbar: SavedList<SnapshotSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_attack: Option<SkillId>,
    #[serde(default, skip_serializing_if = "Stats::is_empty")]
    pub stats: Stats,
    #[schemars(transform = SchemaMark::Usage.with("from the bag"))]
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub bag: SavedList<BagEntry>,
    /// Put on in order, each where equipping puts it.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub equipped: SavedList<ItemId>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub known_areas: SavedList<AreaId>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub quests: SavedList<SnapshotQuest>,
    /// The quests the tracker shows, in its order. Without it, every active quest, as listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_quests: Option<SavedList<QuestId>>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub memories: SavedList<SnapshotMemory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines_heard: Vec<HeardLine>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub shops: SavedList<SnapshotShop>,
    /// What the character's history holds: it crosses into another world of the server with the
    /// character, and is never saved.
    #[serde(skip)]
    pub history: Vec<SnapshotRecord>,
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub statuses: SavedList<SnapshotStatus>,
    /// The skills still cooling down.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub cooldowns: SavedList<SnapshotCooldown>,
    /// The commands still cooling down.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub command_cooldowns: SavedList<SnapshotCommandCooldown>,
    /// The instanced areas they started too lately to start again yet.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub instance_cooldowns: SavedList<SnapshotInstanceCooldown>,
    /// The spot they rise on after a fall, in the area they start in. Without it, where they
    /// start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rises_at: Option<SnapshotSpot>,
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
            recipe_masteries: SavedList::default(),
            hotbar: SavedList::default(),
            basic_attack: None,
            stats: Stats::default(),
            bag: SavedList::default(),
            equipped: SavedList::default(),
            known_areas: SavedList::default(),
            quests: SavedList::default(),
            tracked_quests: None,
            memories: SavedList::default(),
            lines_heard: Vec::new(),
            shops: SavedList::default(),
            history: Vec::new(),
            statuses: SavedList::default(),
            cooldowns: SavedList::default(),
            command_cooldowns: SavedList::default(),
            instance_cooldowns: SavedList::default(),
            rises_at: None,
            start: CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn),
        })
    }
}

impl CharacterSnapshot {
    /// A character as the player row makes it: of its class, looking as the class's model does,
    /// with nothing on.
    pub fn of_def(content: &Content, name: &str, def: &CharacterDef) -> CharacterSnapshot {
        CharacterSnapshot {
            name: name.to_owned(),
            class: def.class,
            appearance: Appearance::of(def.class.get(content).model),
            xp: 0,
            skills: Vec::new(),
            recipe_masteries: SavedList::default(),
            hotbar: SavedList::default(),
            basic_attack: None,
            stats: Stats::default(),
            bag: SavedList::default(),
            equipped: SavedList::default(),
            known_areas: SavedList::default(),
            quests: SavedList::default(),
            tracked_quests: None,
            memories: SavedList::default(),
            lines_heard: Vec::new(),
            shops: SavedList::default(),
            history: Vec::new(),
            statuses: SavedList::default(),
            cooldowns: SavedList::default(),
            command_cooldowns: SavedList::default(),
            instance_cooldowns: SavedList::default(),
            rises_at: None,
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
    /// An active quest's kills since one last gave a drop that grows rarer or likelier with
    /// them.
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub dry_kills: u32,
    /// Whether the player was already told an active quest is ready to hand in.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub told_ready: bool,
    /// How many days ago a finished quest was finished.
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub days_ago: u32,
    /// How long until a finished quest that opens again does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_in: Option<Seconds>,
    /// Whether a finished quest was offered again since.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub offered_again: bool,
}

impl SnapshotQuest {
    /// The quest in `state`, with nothing done on it yet.
    pub fn of(quest: QuestId, state: QuestState) -> SnapshotQuest {
        SnapshotQuest {
            quest,
            state,
            progress: Vec::new(),
            left: None,
            decided: None,
            dry_kills: 0,
            told_ready: false,
            days_ago: 0,
            resets_in: None,
            offered_again: false,
        }
    }
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
    pub ware: u32,
    pub left: u32,
    pub restocks_in: Seconds,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotRecord {
    pub ago: Seconds,
    pub entry: HistoryEntry,
}

/// A status a character carries.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotStatus {
    pub effect: StatusEffectId,
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub power: f32,
    /// How long ago it was applied.
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub ago: Seconds,
    /// How long it still holds. Without it, until it is taken off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Seconds>,
    /// What it had under way when the snapshot was taken. With it the status is taken up where
    /// it was. Without it the status is applied anew.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<RunningStatus>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunningStatus {
    /// When each interval of its script next fires.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beats: Vec<SnapshotBeat>,
    /// The stat modifiers its script holds on the character.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boosts: Vec<Stat>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBeat {
    pub every: Seconds,
    pub next_in: Seconds,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCommandCooldown {
    pub command: TerminalCommandId,
    pub left: Seconds,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotInstanceCooldown {
    pub area: AreaId,
    pub left: Seconds,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCooldown {
    pub skill: SkillId,
    pub left: Seconds,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSpot {
    pub x: f32,
    pub y: f32,
}

/// An account's own character as it is saved: the snapshot, the moment it was taken, and what
/// its player has been told the game changing did to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCharacter {
    pub account: AccountId,
    pub saved_at: UnixMillis,
    pub character: CharacterSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub told: Vec<GameChange>,
}

impl SavedCharacter {
    /// `character` as it stands, when it is an account's own and no stand-in for it.
    pub fn of_live(world: &World, character: Entity) -> Option<SavedCharacter> {
        if world.get::<StandIn>(character).is_some() {
            return None;
        }
        Some(SavedCharacter {
            account: world.get::<CharacterAccount>(character)?.0.clone(),
            saved_at: world.resource::<WallClock>().now,
            character: CharacterSnapshot::of_live(world, character)?,
            told: world
                .get::<KeptAside>(character)
                .map(|kept| kept.told.clone())
                .unwrap_or_default(),
        })
    }
}

/// A stack of the bag as it is saved: the slot it is in, and what it holds.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BagEntry {
    pub slot: u32,
    pub item: ItemId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub count: u32,
}

impl BagEntry {
    pub fn of(slot: u32, stack: ItemStack) -> BagEntry {
        BagEntry {
            slot,
            item: stack.item,
            count: stack.count,
        }
    }

    pub fn stack(self) -> ItemStack {
        ItemStack::new(self.item, self.count)
    }
}

impl CharacterSnapshot {
    /// Puts `stack` in the bag's first slot no other stack is in.
    pub fn carry(&mut self, stack: ItemStack) {
        let slot = (0..)
            .find(|slot| self.bag.iter().all(|entry| entry.slot != *slot))
            .expect("a bag leaves some slot number free");
        self.bag.push(BagEntry::of(slot, stack));
    }
}

/// A bag of `size` slots holding `entries`, each in its slot. One whose slot is past the bag or
/// already taken goes to the first empty slot, as long as there is one.
fn bag_of(size: u32, entries: &[BagEntry]) -> Inventory {
    let mut bag = Inventory::sized(size);
    let mut misplaced = Vec::new();
    for entry in entries {
        match bag.slots.get_mut(entry.slot as usize) {
            Some(slot) if slot.is_none() => *slot = Some(entry.stack()),
            _ => misplaced.push(entry.stack()),
        }
    }
    for stack in misplaced {
        if let Some(slot) = bag.slots.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(stack);
        }
    }
    bag
}

/// Something a player comes back to that is not as they left it, because the game changed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum GameChange {
    /// Something they had names a file the game no longer has, and is kept aside as it was saved:
    /// the type of the file, and the name it went by.
    Gone { table: String, name: String },
    /// Their bag holds fewer stacks than they carried, and these are kept aside.
    DidNotFit { slots: u32, stacks: u32 },
    /// Their class's skills no longer allow what they had learned, so every point is theirs to
    /// spend again.
    SkillsReset { class: String, points: u32 },
}

/// What a character was saved with that the game it came back to has no place for, kept as it
/// was saved so every save writes it back, and what its player has been told of that.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct KeptAside {
    lists: Vec<(&'static str, Vec<SetAside>)>,
    pub told: Vec<GameChange>,
}

/// Every list of `$snapshot` that sets entries aside, by its field, read through `$of`.
macro_rules! saved_lists {
    ($snapshot:expr, $of:ident) => {
        [
            ("recipe_masteries", $snapshot.recipe_masteries.$of()),
            ("hotbar", $snapshot.hotbar.$of()),
            (BAG, $snapshot.bag.$of()),
            ("equipped", $snapshot.equipped.$of()),
            ("known_areas", $snapshot.known_areas.$of()),
            ("quests", $snapshot.quests.$of()),
            ("memories", $snapshot.memories.$of()),
            ("shops", $snapshot.shops.$of()),
            ("statuses", $snapshot.statuses.$of()),
            ("cooldowns", $snapshot.cooldowns.$of()),
            ("command_cooldowns", $snapshot.command_cooldowns.$of()),
            ("instance_cooldowns", $snapshot.instance_cooldowns.$of()),
        ]
    };
}

impl KeptAside {
    /// Whether it keeps stacks of the bag that name items the game has, and only found no room.
    pub fn holds_overflow(&self) -> bool {
        self.bag().any(|aside| aside.missing.is_empty())
    }

    /// Takes out the stacks the bag had no room for, to be handed over some other way.
    pub fn take_overflow(&mut self, content: &Content) -> Vec<ItemStack> {
        let Some((_, bag)) = self.lists.iter_mut().find(|(list, _)| *list == BAG) else {
            return Vec::new();
        };
        let scope = ContentScope::of(content);
        let mut over = Vec::new();
        bag.retain(|aside| {
            if !aside.missing.is_empty() {
                return true;
            }
            let (entry, _) = with_content(scope.clone(), || {
                serde_json::from_value::<BagEntry>(aside.entry.clone())
            });
            match entry {
                Ok(entry) => {
                    over.push(entry.stack());
                    false
                }
                Err(_) => true,
            }
        });
        self.lists.retain(|(_, aside)| !aside.is_empty());
        over
    }

    fn bag(&self) -> impl Iterator<Item = &SetAside> {
        self.lists
            .iter()
            .filter(|(list, _)| *list == BAG)
            .flat_map(|(_, aside)| aside)
    }
}

impl CharacterSnapshot {
    /// What its lists hold set aside.
    fn kept_aside(&self) -> KeptAside {
        let tracked = self
            .tracked_quests
            .as_ref()
            .map(|tracked| (TRACKED_QUESTS, tracked.set_aside()));
        KeptAside {
            lists: saved_lists!(self, set_aside)
                .into_iter()
                .chain(tracked)
                .filter(|(_, aside)| !aside.is_empty())
                .map(|(list, aside)| (list, aside.to_vec()))
                .collect(),
            told: Vec::new(),
        }
    }

    /// Puts back what `kept` holds of its lists.
    fn keep_aside(&mut self, kept: &KeptAside) {
        let tracked = self
            .tracked_quests
            .as_mut()
            .map(|tracked| (TRACKED_QUESTS, tracked.set_aside_mut()));
        for (list, aside) in saved_lists!(self, set_aside_mut).into_iter().chain(tracked) {
            if let Some((_, held)) = kept.lists.iter().find(|(of, _)| *of == list) {
                aside.clone_from(held);
            }
        }
    }
}

const TRACKED_QUESTS: &str = "tracked_quests";
const BAG: &str = "bag";

/// The character a connection joins as, where it starts, and where the character is from.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct JoinAs {
    pub snapshot: CharacterSnapshot,
    pub start: Place,
    pub origin: JoinOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinOrigin {
    /// Staged from the game's content: it stands in for the account's own character, its
    /// snapshot counts from the moment it joins, and it is never saved.
    Staged,
    /// Staged from the game's content to be the account's own character from then on: its
    /// snapshot counts from the moment it joins, and it is saved as the account's own.
    Kept,
    /// The account's own, as it was saved `at` that moment, its player `told` this much of what
    /// the game changing did to it.
    Saved {
        at: UnixMillis,
        told: Vec<GameChange>,
    },
}

pub fn start_of(content: &Content, snapshot: &CharacterSnapshot) -> Place {
    match snapshot.start {
        CharacterStart::InitialSpawn(_) => content.built::<GameSettings>().initial_spawn,
        CharacterStart::At(place) => place,
    }
}

/// The character `snapshot` holds, starting at `start`. What runs on the clock is counted from
/// `taken`, the moment the snapshot was taken: cooldowns, a quest's deadline, a shop's restock,
/// how long ago something was remembered or recorded. What acts on the character is counted from
/// now, so it waited while the character was not in a world: its statuses.
pub fn materialize(
    content: &Content,
    snapshot: &CharacterSnapshot,
    start: Place,
    clock: WallClock,
    taken: UnixMillis,
) -> CharacterState {
    let then = WallClock {
        now: taken,
        reset: clock.reset,
    };
    let mut stats = def(content).stats.clone();
    stats.0.extend(&snapshot.stats.0);
    let mut discovered = Discovered::starting_in(start.area);
    for &known in snapshot.known_areas.iter() {
        discovered.discover(known);
    }
    let mut memory = Memory::default();
    for remembered in snapshot.memories.iter() {
        memory.set(
            content,
            remembered.memory,
            remembered.value.clone().into(),
            WallClock {
                now: taken.before(remembered.ago),
                reset: clock.reset,
            },
        );
    }
    let mut heard = Heard::default();
    for line in &snapshot.lines_heard {
        heard.hear(&line.script, &line.node);
    }
    let mut ledger = ShopLedger::default();
    for visited in snapshot.shops.iter() {
        for stock in &visited.stock {
            ledger.stocked(
                visited.shop,
                stock.ware,
                stock.left,
                taken.after(stock.restocks_in),
            );
        }
        for sale in &visited.sales {
            ledger.record_sale(
                visited.shop,
                sale.clone(),
                visited.shop.get(content).buys_back,
            );
        }
    }
    let (statuses, carried) = statuses(snapshot, clock.now);
    CharacterState {
        name: Name {
            name: snapshot.name.clone(),
        },
        appearance: snapshot.appearance.clone(),
        spawn: SpawnPoint(
            snapshot
                .rises_at
                .map_or(start.at, |spot| Pos::new(spot.x, spot.y)),
        ),
        discovered,
        stats,
        inventory: bag_of(content.built::<GameSettings>().slots.bag, &snapshot.bag),
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
        masteries: RecipeMasteries(snapshot.recipe_masteries.iter().cloned().collect()),
        hotbar: hotbar(snapshot),
        basic_attack: BasicAttack(snapshot.basic_attack),
        memory,
        heard,
        ledger,
        quests: quest_log(content, snapshot, then),
        history: History::replayed(
            snapshot
                .history
                .iter()
                .map(|record| (taken.before(record.ago), record.entry.clone())),
            content.built::<GameSettings>().history,
        ),
        cooldowns: Cooldowns(
            snapshot
                .cooldowns
                .iter()
                .map(|cooling| (cooling.skill, taken.after(cooling.left)))
                .collect(),
        ),
        command_cooldowns: CommandCooldowns(
            snapshot
                .command_cooldowns
                .iter()
                .map(|cooling| (cooling.command, taken.after(cooling.left)))
                .collect(),
        ),
        instance_cooldowns: InstanceCooldowns(
            snapshot
                .instance_cooldowns
                .iter()
                .map(|cooling| (cooling.area, taken.after(cooling.left)))
                .collect(),
        ),
        statuses,
        carried,
        kept_aside: snapshot.kept_aside(),
    }
}

impl CharacterSnapshot {
    /// A live character, as it stands: every part `CharacterState` lists, with each moment on
    /// the clock written as time ago or time left.
    pub fn of_live(world: &World, player: Entity) -> Option<CharacterSnapshot> {
        let now = world.resource::<WallClock>().now;
        let day = world.resource::<WallClock>().day();
        let area = world.resource::<WorldArea>().0;
        let at = world.get::<Position>(player)?.pos;
        let state = CharacterState::of(world, player, world.get::<SpawnPoint>(player)?.0)?;
        // An instance is a run, gone with the server: whoever is saved inside one comes back
        // where leaving it leads.
        let (start, rises_at) = match world.get_resource::<AreaInstanceWorld>() {
            Some(run) => (run.return_to, run.return_to.at),
            None => (Place { area, at }, state.spawn.0),
        };
        let log = &state.quests;
        let active = log.active.iter().map(|active| SnapshotQuest {
            quest: active.quest,
            state: QuestState::Active,
            progress: active.counts.clone(),
            left: active.deadline.map(|deadline| deadline.since(now)),
            decided: active.decided.clone(),
            dry_kills: active.dry_kills,
            told_ready: log.told_ready.contains(&active.quest),
            days_ago: 0,
            resets_in: None,
            offered_again: false,
        });
        let finished = log.finished.iter().map(|finished| SnapshotQuest {
            quest: finished.quest,
            state: match finished.result {
                QuestResult::Completed => QuestState::Completed,
                QuestResult::Failed => QuestState::Failed,
            },
            progress: Vec::new(),
            left: None,
            decided: None,
            dry_kills: 0,
            told_ready: false,
            days_ago: u32::try_from(day.0.saturating_sub(finished.day.0)).unwrap_or(u32::MAX),
            resets_in: finished.resets_at.map(|at| at.since(now)),
            offered_again: finished.offered_again,
        });
        let mut shops: Vec<SnapshotShop> = Vec::new();
        let shop = |shops: &mut Vec<SnapshotShop>, id: ShopId| -> usize {
            shops
                .iter()
                .position(|visited| visited.shop == id)
                .unwrap_or_else(|| {
                    shops.push(SnapshotShop {
                        shop: id,
                        stock: Vec::new(),
                        sales: Vec::new(),
                    });
                    shops.len() - 1
                })
        };
        let mut stock: Vec<_> = state.ledger.stock().collect();
        stock.sort_by_key(|&(shop, ware, ..)| (shop, ware));
        for (id, ware, left, full_at) in stock {
            let visited = shop(&mut shops, id);
            shops[visited].stock.push(ShopStock {
                ware,
                left,
                restocks_in: full_at.since(now),
            });
        }
        let mut sales: Vec<_> = state.ledger.sales().collect();
        sales.sort_by_key(|(shop, _)| *shop);
        for (id, sold) in sales {
            let visited = shop(&mut shops, id);
            shops[visited].sales.extend(sold.cloned());
        }
        let mut equipped: Vec<ItemId> = Vec::new();
        for &item in state.equipment.slots.values() {
            if !equipped.contains(&item) {
                equipped.push(item);
            }
        }
        let mut snapshot = CharacterSnapshot {
            name: state.name.name.clone(),
            class: state.class.def,
            appearance: state.appearance.clone(),
            xp: state.xp.amount,
            skills: state
                .learned
                .0
                .iter()
                .map(|(node, &rank)| SnapshotSkill {
                    node: node.clone(),
                    rank,
                })
                .collect(),
            recipe_masteries: state.masteries.0.iter().cloned().collect(),
            hotbar: state
                .hotbar
                .0
                .iter()
                .zip(0..)
                .filter(|(entry, _)| **entry != HotbarEntry::Empty)
                .map(|(&entry, slot)| SnapshotSlot {
                    slot: HotbarSlot(slot),
                    entry,
                })
                .collect(),
            basic_attack: state.basic_attack.0,
            stats: stat::snapshot(world, player),
            bag: state
                .inventory
                .stacks()
                .map(|(slot, stack)| BagEntry::of(slot as u32, stack))
                .collect(),
            equipped: equipped.into(),
            known_areas: state.discovered.areas().collect(),
            quests: active.chain(finished).collect(),
            tracked_quests: Some(log.tracked.iter().copied().collect()),
            memories: state
                .memory
                .recollections()
                .filter_map(|(memory, value, at)| {
                    Some(SnapshotMemory {
                        memory,
                        value: MemoryValue::of(value)?,
                        ago: now.since(at),
                    })
                })
                .collect(),
            lines_heard: state
                .heard
                .lines()
                .map(|(script, node)| HeardLine {
                    script: script.to_owned(),
                    node: node.to_owned(),
                })
                .collect(),
            shops: shops.into(),
            history: state
                .history
                .records()
                .map(|record| SnapshotRecord {
                    ago: now.since(record.at),
                    entry: record.entry.clone(),
                })
                .collect(),
            statuses: state
                .statuses
                .0
                .iter()
                .map(|held| SnapshotStatus {
                    effect: held.effect,
                    power: held.power,
                    ago: now.since(held.since),
                    left: held.until.map(|until| until.since(now)),
                    running: state
                        .carried
                        .0
                        .iter()
                        .find(|carried| {
                            carried.effect == held.effect && carried.since == held.since
                        })
                        .map(|carried| RunningStatus {
                            beats: carried
                                .beats
                                .iter()
                                .map(|beat| SnapshotBeat {
                                    every: beat.every,
                                    next_in: beat.next.since(now),
                                })
                                .collect(),
                            boosts: carried.boosts.clone(),
                        }),
                })
                .collect(),
            cooldowns: state
                .cooldowns
                .0
                .iter()
                .filter(|&(_, &ready)| ready > now)
                .map(|(&skill, &ready)| SnapshotCooldown {
                    skill,
                    left: ready.since(now),
                })
                .collect(),
            command_cooldowns: state
                .command_cooldowns
                .0
                .iter()
                .filter(|&(_, &ready)| ready > now)
                .map(|(&command, &ready)| SnapshotCommandCooldown {
                    command,
                    left: ready.since(now),
                })
                .collect(),
            instance_cooldowns: state
                .instance_cooldowns
                .0
                .iter()
                .filter(|&(_, &ready)| ready > now)
                .map(|(&area, &ready)| SnapshotInstanceCooldown {
                    area,
                    left: ready.since(now),
                })
                .collect(),
            rises_at: Some(SnapshotSpot {
                x: rises_at.x,
                y: rises_at.y,
            }),
            start: CharacterStart::At(start),
        };
        snapshot.keep_aside(&state.kept_aside);
        Some(snapshot)
    }
}

/// The checks that rest on the game's settings are left until the settings load whole.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let thorough = loading.thorough();
    let settings = content.try_built::<GameSettings>();
    for (id, snapshot) in loading.rows::<CharacterSnapshot>(&content) {
        let mut check = RowCheck::default();
        check_start(&content, settings, snapshot, thorough, &mut check);
        check_bag(&content, settings, snapshot, &mut check);
        check_equipment(&content, snapshot, &mut check);
        check_skills(&content, snapshot, &mut check);
        check_masteries(&content, snapshot, &mut check);
        check_hotbar(&content, settings, snapshot, &mut check);
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
    /// What a memory holds, when it is of a type a memory is declared as.
    fn of(value: &Value) -> Option<MemoryValue> {
        match value {
            Value::Bool(flag) => Some(MemoryValue::Bool(*flag)),
            Value::Int(int) => Some(MemoryValue::Int(*int)),
            Value::Num(num) => Some(MemoryValue::Num(*num)),
            Value::Text(text) => Some(MemoryValue::Text(text.clone())),
            _ => None,
        }
    }

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
    for held in snapshot.hotbar.iter() {
        hotbar.set(held.slot, held.entry);
    }
    hotbar
}

/// The statuses `snapshot` holds, counted from `now`, and what those taken up where they were
/// had under way.
fn statuses(snapshot: &CharacterSnapshot, now: UnixMillis) -> (Statuses, CarriedStatuses) {
    let mut statuses = Statuses::default();
    let mut carried = CarriedStatuses::default();
    for held in snapshot.statuses.iter() {
        let since = now.before(held.ago);
        statuses.0.push(StatusInstance {
            effect: held.effect,
            power: held.power,
            since,
            until: held.left.map(|left| now.after(left)),
        });
        if let Some(running) = &held.running {
            carried.0.push(CarriedStatus {
                effect: held.effect,
                since,
                beats: running
                    .beats
                    .iter()
                    .map(|beat| StatusBeat {
                        every: beat.every,
                        next: now.after(beat.next_in),
                    })
                    .collect(),
                boosts: running.boosts.clone(),
            });
        }
    }
    (statuses, carried)
}

fn quest_log(content: &Content, snapshot: &CharacterSnapshot, then: WallClock) -> QuestLog {
    let mut log = QuestLog::default();
    for entry in snapshot.quests.iter() {
        let result = match entry.state {
            QuestState::Active => {
                let mut active = ActiveQuest::fresh(content, entry.quest);
                for (count, given) in active.counts.iter_mut().zip(&entry.progress) {
                    *count = *given;
                }
                active.deadline = entry.left.map(|left| then.now.after(left));
                active.decided = entry.decided.clone();
                active.dry_kills = entry.dry_kills;
                log.active.push(active);
                if snapshot.tracked_quests.is_none() {
                    log.tracked.push(entry.quest);
                }
                if entry.told_ready {
                    log.told_ready.push(entry.quest);
                }
                continue;
            }
            QuestState::Completed => QuestResult::Completed,
            QuestState::Failed => QuestResult::Failed,
        };
        let day = GameDay(then.day().0.saturating_sub(u64::from(entry.days_ago)));
        log.finished.push(FinishedQuest {
            resets_at: entry.resets_in.map(|left| then.now.after(left)),
            offered_again: entry.offered_again,
            ..FinishedQuest::new(entry.quest, result, day)
        });
    }
    if let Some(tracked) = &snapshot.tracked_quests {
        log.tracked = tracked
            .iter()
            .copied()
            .filter(|&quest| log.active(quest).is_some())
            .collect();
    }
    log
}

/// Whether the snapshot starts where the game holds a character to start; on the ground there,
/// when `thorough`.
fn check_start(
    content: &Content,
    settings: Option<&GameSettings>,
    snapshot: &CharacterSnapshot,
    thorough: bool,
    check: &mut RowCheck,
) {
    let start = match snapshot.start {
        CharacterStart::InitialSpawn(_) => settings.map(|settings| settings.initial_spawn),
        CharacterStart::At(place) => Some(place),
    };
    if let CharacterStart::At(place) = snapshot.start
        && thorough
        && let Err(problem) = area::check_place(content, place)
    {
        check.problem("/start", problem);
    }
    if let Some(start) = start
        && !snapshot.known_areas.is_empty()
        && !snapshot.known_areas.contains(&start.area)
    {
        check.error(
            "/known_areas",
            "start not known",
            start.area.path(content),
            "the known areas to include the start's area",
        );
    }
}

fn check_bag(
    content: &Content,
    settings: Option<&GameSettings>,
    snapshot: &CharacterSnapshot,
    check: &mut RowCheck,
) {
    let slots = settings.map(|settings| settings.slots.bag);
    for (index, entry) in snapshot.bag.iter().enumerate() {
        if let Some(slots) = slots
            && entry.slot >= slots
        {
            check.error(
                format!("/bag/{index}/slot"),
                "slot past the bag",
                entry.slot,
                format!("a slot below the bag's {slots}"),
            );
        }
        if snapshot.bag[..index]
            .iter()
            .any(|earlier| earlier.slot == entry.slot)
        {
            check.error(
                format!("/bag/{index}/slot"),
                "slot taken twice",
                entry.slot,
                "a slot no other stack of the bag is in",
            );
        }
        let stack = entry.stack();
        let item = stack.item.get(content);
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
                        requirement.line(content, &class.tree).plain_words(content)
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

fn check_hotbar(
    content: &Content,
    settings: Option<&GameSettings>,
    snapshot: &CharacterSnapshot,
    check: &mut RowCheck,
) {
    let class = snapshot.class.get(content);
    let known = skill::known_from(
        content,
        Some(&class.tree),
        Some(&learned(snapshot)),
        snapshot.equipped.iter().copied(),
        snapshot.bag.iter().map(|stack| stack.item),
        None,
    );
    let held = |item: ItemId| {
        snapshot.bag.iter().any(|stack| stack.item == item) || snapshot.equipped.contains(&item)
    };
    let waits_in_bag = |skill: SkillId| {
        snapshot
            .bag
            .iter()
            .any(|stack| skill::grants_of(content, stack.item).contains(&skill))
    };
    let slots = settings.map(|settings| settings.slots.hotbar);
    for (index, slot) in snapshot.hotbar.iter().enumerate() {
        if let Some(slots) = slots
            && slot.slot.0 >= slots
        {
            check.error(
                format!("/hotbar/{index}/slot"),
                "out of range",
                slot.slot.0,
                format!("0 to {}", slots - 1),
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
            HotbarEntry::Skill(skill) if !known.contains_key(&skill) && !waits_in_bag(skill) => {
                check.error(
                    format!("/hotbar/{index}/entry"),
                    "skill not known",
                    skill.path(content),
                    "a skill learned in the tree, or granted by gear worn or in the bag",
                )
            }
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
        for (objective, (&count, need)) in entry
            .progress
            .iter()
            .zip(def.objectives().map(Objective::need))
            .enumerate()
        {
            if count > need {
                check.error(
                    format!("/quests/{index}/progress/{objective}"),
                    "progress out of range",
                    count,
                    format!("0 to {need}"),
                );
            }
        }
    }
}

fn check_masteries(content: &Content, snapshot: &CharacterSnapshot, check: &mut RowCheck) {
    for (index, mastery) in snapshot.recipe_masteries.iter().enumerate() {
        let at = format!("/recipe_masteries/{index}");
        if snapshot.recipe_masteries[..index].contains(mastery) {
            check.error(
                at,
                "mastery listed twice",
                &mastery.recipe.0,
                "each recipe once",
            );
        } else if let Some(problem) = crafting::mastery_problem(content, mastery) {
            check.problem(at, problem);
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
        let wares = visited.shop.get(content).sells().count() as u32;
        for (entry, stock) in visited.stock.iter().enumerate() {
            if stock.ware >= wares {
                check.error(
                    format!("/shops/{index}/stock/{entry}/ware"),
                    "unknown ware",
                    stock.ware,
                    format!(
                        "0 to {}, a ware of {}",
                        wares.saturating_sub(1),
                        visited.shop.path(content)
                    ),
                );
            }
        }
    }
}
