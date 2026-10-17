pub mod card;
pub mod effects;
pub mod log;
pub mod tracker;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::{
    Content, ContentId, ContentProblem, ContentRow, Fixture, Loading, ModuleRule, ModuleSet,
    RowCheck,
};
use crate::core::math::{Percent, Rng};
use crate::core::time::{GameDay, Seconds, UnixMillis, WallClock};
use crate::systems::area::{self, AreaDef, AreaId};
use crate::systems::attention::{AttentionId, AttentionMark};
use crate::systems::class;
use crate::systems::dialogue::{self, ChoiceReveal, Revealable};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::item::{
    INVENTORY_MAX, Inventory, ItemDef, ItemFlag, ItemId, ItemStack, scatter_drop,
};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::NpcId;
use crate::systems::player::{Owner, sender_player};
use crate::systems::rewards::KillCredited;
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::{
    self, Condition, EventKind, Owner as ScriptOwner, OwnerKind, OwnerRow, Reach, RiftInstances,
    RiftScripts, ScriptModule, ScriptedRow, condition,
};
use crate::systems::text::{LineText, Span};
use crate::systems::visibility::Presence;
use crate::systems::{attention, drain_requests};

pub type QuestId = ContentId<QuestDef>;

pub const QUEST_LOG_CAP: usize = 20;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<QuestLog>()
        .add_client_message::<QuestRequest>(Channel::Ordered);
    attention::mark_source(app, marks);
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct QuestDef {
    pub title: String,
    pub category: String,
    pub blurb: String,
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal: Vec<ChoiceReveal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<QuestModule>,
}

impl ContentRow for QuestDef {
    const TABLE: &'static str = "quest";
}

impl ModuleSet for QuestDef {
    type Module = QuestModule;

    fn modules(&self) -> &[QuestModule] {
        &self.modules
    }

    fn rule(module: &QuestModule) -> ModuleRule {
        match module {
            QuestModule::Objective(_) | QuestModule::Drop(_) | QuestModule::Script(_) => {
                ModuleRule::AnyNumber
            }
            _ => ModuleRule::AtMostOne,
        }
    }
}

impl ScriptedRow for QuestDef {
    const OWNER: OwnerKind = OwnerKind::Quest;

    fn script(module: &QuestModule) -> Option<&ScriptModule> {
        match module {
            QuestModule::Script(script) => Some(script),
            _ => None,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestModule {
    Shown {
        condition: Condition,
    },
    Requires {
        condition: Condition,
    },
    Grants {
        items: Vec<ItemStack>,
    },
    HandIn {
        items: Vec<ItemStack>,
    },
    HandOver {
        text: Vec<Span>,
    },
    Reward {
        items: Vec<ItemStack>,
    },
    PickOne {
        items: Vec<ItemStack>,
    },
    Daily,
    TimeLimit {
        seconds: Seconds,
    },
    Drop(QuestDrop),
    #[serde(untagged)]
    Objective(Objective),
    #[serde(untagged)]
    Script(ScriptModule),
}

impl QuestDef {
    pub fn shown(&self) -> Option<&Condition> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::Shown { condition } => Some(condition),
            _ => None,
        })
    }

    pub fn requires(&self) -> Option<&Condition> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::Requires { condition } => Some(condition),
            _ => None,
        })
    }

    pub fn min_level(&self) -> u32 {
        self.requires()
            .into_iter()
            .flat_map(condition::parts)
            .filter_map(condition::min_level)
            .max()
            .unwrap_or(1)
    }

    pub fn grants(&self) -> &[ItemStack] {
        self.stacks(|module| match module {
            QuestModule::Grants { items } => Some(items),
            _ => None,
        })
    }

    pub fn objectives(&self) -> impl Iterator<Item = &Objective> + Clone {
        self.modules.iter().filter_map(|module| match module {
            QuestModule::Objective(objective) => Some(objective),
            _ => None,
        })
    }

    pub fn hand_in(&self) -> &[ItemStack] {
        self.stacks(|module| match module {
            QuestModule::HandIn { items } => Some(items),
            _ => None,
        })
    }

    pub fn hand_over(&self) -> &[Span] {
        self.modules
            .iter()
            .find_map(|module| match module {
                QuestModule::HandOver { text } => Some(text.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    pub fn rewards(&self) -> &[ItemStack] {
        self.stacks(|module| match module {
            QuestModule::Reward { items } => Some(items),
            _ => None,
        })
    }

    pub fn pick_one(&self) -> &[ItemStack] {
        self.stacks(|module| match module {
            QuestModule::PickOne { items } => Some(items),
            _ => None,
        })
    }

    pub fn daily(&self) -> bool {
        self.modules
            .iter()
            .any(|module| matches!(module, QuestModule::Daily))
    }

    pub fn time_limit(&self) -> Option<Seconds> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::TimeLimit { seconds } => Some(*seconds),
            _ => None,
        })
    }

    pub fn drops(&self) -> impl Iterator<Item = &QuestDrop> {
        self.modules.iter().filter_map(|module| match module {
            QuestModule::Drop(drop) => Some(drop),
            _ => None,
        })
    }

    pub fn objective_label(
        &self,
        content: &Content,
        parties: &Parties,
        quest: QuestId,
        objective: &Objective,
    ) -> String {
        match objective {
            Objective::Hold(stack) => stack.item.get(content).name.to_owned(),
            Objective::Defeat { npc, .. } => {
                format!("Defeat the {}", npc.get(content).name)
            }
            Objective::Task { label, .. } | Objective::Decide { label, .. } => (*label).to_owned(),
            Objective::Deliver { item } => format!(
                "Bring {} to {}",
                item.get(content).name,
                parties
                    .turn_in(quest)
                    .map_or("its taker", |party| party.name.as_str())
            ),
        }
    }

    pub fn progress(
        &self,
        content: &Content,
        parties: &Parties,
        active: &ActiveQuest,
        inventory: Option<&Inventory>,
    ) -> Vec<Progress> {
        let held = |item| inventory.map_or(0, |inventory| inventory.count(item));
        self.objectives()
            .enumerate()
            .map(|(index, objective)| {
                let counted = active.counts.get(index).copied().unwrap_or(0);
                let (have, need, shown) = match objective {
                    Objective::Hold(stack) => {
                        (held(stack.item).min(stack.count), stack.count, true)
                    }
                    Objective::Defeat { count, .. } => (counted.min(*count), *count, true),
                    Objective::Task { .. } => (counted.min(1), 1, true),
                    Objective::Deliver { item } => (held(*item).min(1), 1, false),
                    Objective::Decide { .. } => (0, 1, false),
                };
                Progress {
                    label: self.objective_label(content, parties, active.quest, objective),
                    have,
                    need,
                    counted: shown,
                }
            })
            .collect()
    }

    pub fn complete(
        &self,
        content: &Content,
        parties: &Parties,
        active: &ActiveQuest,
        inventory: Option<&Inventory>,
    ) -> bool {
        self.objectives()
            .zip(self.progress(content, parties, active, inventory))
            .all(|(objective, progress)| {
                matches!(objective, Objective::Decide { .. }) || progress.done()
            })
    }

    pub fn kinds(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        for objective in self.objectives() {
            let kind = match objective {
                Objective::Hold(_) => "collect",
                Objective::Defeat { .. } => "defeat",
                Objective::Task { .. } => "explore",
                Objective::Deliver { .. } => "deliver",
                Objective::Decide { .. } => "choice",
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if self.time_limit().is_some() {
            kinds.push("timed");
        }
        if self.daily() {
            kinds.push("daily");
        }
        match kinds.split_last() {
            Some((last, [])) => (*last).to_owned(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    }

    pub fn meta(&self, parties: &Parties, quest: QuestId) -> String {
        format!(
            "{} · Level {} · from {}",
            self.category,
            self.min_level(),
            parties
                .giver(quest)
                .map_or("nobody", |party| party.name.as_str())
        )
    }

    pub fn icon<'a>(&'a self, content: &'a Content) -> &'a AssetRef {
        if self.time_limit().is_some() {
            return &InterfaceIcon::Timed.get(content).image;
        }
        if self.daily() {
            return &AttentionMark::RepeatableOffered.get(content).icon;
        }
        self.objectives().next().map_or(
            &AttentionMark::QuestOffered.get(content).icon,
            |objective| objective.icon(content),
        )
    }

    pub fn hand_in_hint(&self, parties: &Parties, quest: QuestId) -> String {
        let giver = parties.giver(quest);
        let taker = parties.turn_in(quest);
        let verb = if giver == taker { "Return" } else { "Go" };
        match taker {
            Some(party) => format!("{verb} to {}", party.name),
            None => format!("{verb} and hand it in"),
        }
    }

    pub fn retry_hint(&self, parties: &Parties, quest: QuestId) -> String {
        match parties.giver(quest) {
            Some(party) if party.item.is_some() => format!("Use the {} to try again", party.name),
            Some(party) => format!("Talk to {} to try again", party.name),
            None => "Try again".to_owned(),
        }
    }

    pub fn reveals(&self, reveal: ChoiceReveal) -> bool {
        self.reveal.contains(&reveal)
    }

    pub fn decision(&self) -> Option<&[DecisionPath]> {
        self.objectives().find_map(|objective| match objective {
            Objective::Decide { paths, .. } => Some(paths.as_slice()),
            _ => None,
        })
    }

    fn stacks(&self, pick: fn(&QuestModule) -> Option<&Vec<ItemStack>>) -> &[ItemStack] {
        self.modules
            .iter()
            .find_map(pick)
            .map_or(&[], Vec::as_slice)
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Objective {
    Hold(ItemStack),
    Defeat {
        npc: NpcId,
        count: u32,
    },
    Task {
        task: String,
        label: String,
    },
    Deliver {
        item: ItemId,
    },
    Decide {
        label: String,
        paths: Vec<DecisionPath>,
    },
}

impl Objective {
    pub fn icon<'a>(&'a self, content: &'a Content) -> &'a AssetRef {
        match self {
            Objective::Hold(stack) => &stack.item.get(content).icon,
            Objective::Defeat { .. } => &InterfaceIcon::ObjectiveDefeat.get(content).image,
            Objective::Task { .. } => &InterfaceIcon::ObjectiveTask.get(content).image,
            Objective::Deliver { item } => &item.get(content).icon,
            Objective::Decide { .. } => &InterfaceIcon::ObjectiveDecide.get(content).image,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct DecisionPath {
    pub choice: String,
    pub means: Vec<String>,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct QuestDrop {
    pub from: NpcId,
    pub item: ItemId,
    pub chance: Percent,
    pub grows: Percent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub label: String,
    pub have: u32,
    pub need: u32,
    pub counted: bool,
}

impl Progress {
    pub fn done(&self) -> bool {
        self.have >= self.need
    }
}

#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Parties(Arc<HashMap<QuestId, QuestParties>>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestParties {
    pub givers: Vec<Party>,
    pub turn_ins: Vec<Party>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    pub name: String,
    pub item: Option<ItemId>,
    pub npc: Option<NpcId>,
}

impl Parties {
    pub fn of(content: &Content, scripts: &RiftScripts) -> Parties {
        let mut parties: HashMap<QuestId, QuestParties> = HashMap::new();
        for (owner, role) in quest_roles(scripts) {
            let Some(party) = party_name(content, owner) else {
                continue;
            };
            let parties = parties.entry(role.quest()).or_default();
            match role {
                QuestRole::Gives(_) if !parties.givers.contains(&party) => {
                    parties.givers.push(party);
                }
                QuestRole::TurnsIn(_) if !parties.turn_ins.contains(&party) => {
                    parties.turn_ins.push(party);
                }
                _ => {}
            }
        }
        Parties(Arc::new(parties))
    }

    pub fn get(&self, quest: QuestId) -> Option<&QuestParties> {
        self.0.get(&quest)
    }

    pub fn giver(&self, quest: QuestId) -> Option<&Party> {
        self.get(quest).and_then(|parties| parties.givers.first())
    }

    pub fn turn_in(&self, quest: QuestId) -> Option<&Party> {
        self.get(quest).and_then(|parties| parties.turn_ins.first())
    }
}

enum QuestRole {
    Gives(QuestId),
    TurnsIn(QuestId),
}

impl QuestRole {
    fn quest(&self) -> QuestId {
        match *self {
            QuestRole::Gives(quest) | QuestRole::TurnsIn(quest) => quest,
        }
    }
}

fn quest_roles(scripts: &RiftScripts) -> Vec<(OwnerRow, QuestRole)> {
    let mut roles = Vec::new();
    for (owner, _) in scripts.owners() {
        for reach in scripts.may_do(*owner).reaches {
            match reach {
                Reach::Offers(quest) | Reach::Accepts(quest) => {
                    roles.push((*owner, QuestRole::Gives(quest)));
                }
                Reach::TurnsIn(quest) => roles.push((*owner, QuestRole::TurnsIn(quest))),
                _ => {}
            }
        }
    }
    roles
}

fn party_name(content: &Content, owner: OwnerRow) -> Option<Party> {
    let placed = |npc: NpcId| {
        content.ids::<AreaDef>().any(|area| {
            area::residents(area::placed(content, area)).any(|resident| resident == npc)
        })
    };
    Some(match owner.kind {
        OwnerKind::Npc => {
            let npc: NpcId = owner.id();
            if !placed(npc) {
                return None;
            }
            Party {
                name: npc.get(content).name.to_owned(),
                item: None,
                npc: Some(npc),
            }
        }
        OwnerKind::Object => {
            let area: AreaId = owner.id();
            let (_, object) = area::objects(area::placed(content, area))
                .find(|(index, _)| Some(*index) == owner.object)?;
            Party {
                name: object.label.clone().or_else(|| object.name.clone())?,
                item: None,
                npc: None,
            }
        }
        OwnerKind::Item => {
            let item: ItemId = owner.id();
            Party {
                name: item.get(content).name.to_owned(),
                item: Some(item),
                npc: None,
            }
        }
        _ => return None,
    })
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Hash)]
pub struct QuestLog {
    pub active: Vec<ActiveQuest>,
    pub finished: Vec<FinishedQuest>,
    pub tracked: Vec<QuestId>,
    pub told_ready: Vec<QuestId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct ActiveQuest {
    pub quest: QuestId,
    pub counts: Vec<u32>,
    pub deadline: Option<UnixMillis>,
    pub dry_kills: u32,
    pub decided: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Hash)]
pub struct FinishedQuest {
    pub quest: QuestId,
    pub result: QuestResult,
    pub day: GameDay,
    pub resets_at: Option<UnixMillis>,
    pub offered_again: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuestResult {
    Completed,
    Failed,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum QuestRequest {
    Track { quest: QuestId, tracked: bool },
    Abandon { quest: QuestId },
}

impl ActiveQuest {
    pub fn left(&self, now: UnixMillis) -> Option<Seconds> {
        self.deadline
            .map(|deadline| Seconds(deadline.since(now).0.ceil()))
    }

    pub fn ready(
        &self,
        content: &Content,
        parties: &Parties,
        inventory: Option<&Inventory>,
    ) -> bool {
        self.quest
            .get(content)
            .complete(content, parties, self, inventory)
    }

    pub fn fresh(content: &Content, quest: QuestId) -> ActiveQuest {
        ActiveQuest {
            quest,
            counts: vec![0; quest.get(content).objectives().count()],
            deadline: None,
            dry_kills: 0,
            decided: None,
        }
    }
}

impl QuestLog {
    pub fn active(&self, quest: QuestId) -> Option<&ActiveQuest> {
        self.active.iter().find(|active| active.quest == quest)
    }

    pub fn finished(&self, quest: QuestId) -> Option<&FinishedQuest> {
        self.finished
            .iter()
            .find(|finished| finished.quest == quest)
    }

    pub fn trackable(&self, content: &Content, quest: QuestId) -> bool {
        self.active(quest).is_some()
            || self
                .finished(quest)
                .is_some_and(|finished| match finished.result {
                    QuestResult::Failed => finished.offered_again,
                    QuestResult::Completed => quest.get(content).daily(),
                })
    }

    pub fn done_for_now(&self, content: &Content, quest: QuestId, today: GameDay) -> bool {
        self.finished(quest).is_some_and(|finished| {
            finished.result == QuestResult::Completed
                && (!quest.get(content).daily() || finished.day == today)
        })
    }

    fn finish(&mut self, quest: QuestId, result: QuestResult, clock: WallClock) {
        self.active.retain(|active| active.quest != quest);
        self.told_ready.retain(|told| *told != quest);
        self.finished.retain(|finished| finished.quest != quest);
        let mut finished = FinishedQuest::new(quest, result, clock.day());
        finished.resets_at =
            (result == QuestResult::Completed).then(|| clock.now.after(clock.until_next_day()));
        self.finished.push(finished);
    }
}

impl FinishedQuest {
    pub fn new(quest: QuestId, result: QuestResult, day: GameDay) -> FinishedQuest {
        FinishedQuest {
            quest,
            result,
            day,
            resets_at: None,
            offered_again: false,
        }
    }

    pub fn resets_in(&self, now: UnixMillis) -> Option<Seconds> {
        self.resets_at
            .filter(|at| now < *at)
            .map(|at| Seconds((at.since(now).0 / 60.0).ceil() * 60.0))
    }
}

pub fn clock_label(left: Seconds) -> String {
    let seconds = left.0.max(0.0).ceil() as u32;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub fn resets_label(left: Seconds) -> String {
    let minutes = (left.0.max(0.0) / 60.0).ceil() as u32;
    match minutes {
        0..60 => format!("resets in {minutes}m"),
        _ => format!("resets in {}h", minutes.div_ceil(60)),
    }
}

pub fn log_of(world: &World, player: Entity) -> Option<&QuestLog> {
    world.get::<QuestLog>(player)
}

pub fn ready_to_hand_in(world: &World, player: Entity, quest: QuestId) -> bool {
    let content = world.resource::<Content>();
    let parties = world.resource::<Parties>();
    log_of(world, player)
        .and_then(|log| log.active(quest))
        .is_some_and(|active| active.ready(content, parties, world.get::<Inventory>(player)))
}

pub fn accept(world: &mut World, player: Entity, quest: QuestId) -> Result<(), String> {
    if let Some(refusal) = accept_refusal(world, player, quest) {
        return Err(refusal);
    }
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let def = quest.get(&content);
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?;
    inventory
        .exchange(&content, &[], def.grants())
        .map_err(|refusal| refusal.describe(&content))?;
    let mut log = world
        .get_mut::<QuestLog>(player)
        .ok_or_else(|| "Has no quest log".to_owned())?;
    log.finished
        .retain(|finished| finished.quest != quest || finished.result == QuestResult::Completed);
    log.active.push(ActiveQuest {
        deadline: def.time_limit().map(|limit| clock.now.after(limit)),
        ..ActiveQuest::fresh(&content, quest)
    });
    if !log.tracked.contains(&quest) {
        log.tracked.push(quest);
    }
    tell(
        world,
        player,
        format!("Quest accepted · {}", def.title),
        &offered_mark(def).get(&content).icon,
        Some(InterfaceSound::QuestAccepted.id(&content)),
    );
    raise(world, quest, EventKind::Accepted, player);
    announce(world, player, clock);
    Ok(())
}

pub fn not_ready(content: &Content, quest: QuestId) -> String {
    format!("{} isn't ready to hand in", quest.get(content).title)
}

pub fn turn_in(
    world: &mut World,
    player: Entity,
    quest: QuestId,
    pick: Option<ItemStack>,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    if !ready_to_hand_in(world, player, quest) {
        return Err(not_ready(&content, quest));
    }
    let gives: Vec<ItemStack> = def.rewards().iter().copied().chain(pick).collect();
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?;
    inventory
        .exchange(&content, def.hand_in(), &gives)
        .map_err(|refusal| refusal.describe(&content))?;
    complete(world, player, quest);
    Ok(())
}

pub fn complete(world: &mut World, player: Entity, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let def = quest.get(&content);
    let Some(mut log) = world.get_mut::<QuestLog>(player) else {
        return;
    };
    if log.active(quest).is_none() {
        return;
    }
    log.finish(quest, QuestResult::Completed, clock);
    if !def.daily() {
        log.tracked.retain(|&tracked| tracked != quest);
    }
    class::grant_xp(world, player, def.xp);
    notification::notify(
        world,
        player,
        Notification::new(
            NotificationKind::Milestone {
                label: "Quest complete".into(),
                topic: HistoryTopic::Quest,
                sfx: Some(InterfaceSound::QuestCompleted.id(&content)),
            },
            LineText::plain(def.title.as_str()),
        ),
    );
    raise(world, quest, EventKind::Completed, player);
    announce(world, player, clock);
}

pub fn fail(world: &mut World, player: Entity, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let Some(mut log) = world.get_mut::<QuestLog>(player) else {
        return;
    };
    if log.active(quest).is_none() {
        return;
    }
    log.finish(quest, QuestResult::Failed, clock);
    take_back(world, player, quest.get(&content).grants());
    let failed = Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Quest,
            icon: Some(quest.get(&content).icon(&content).path()),
            tally: None,
            failure: true,
            sfx: Some(InterfaceSound::QuestAbandoned.id(&content)),
        },
        LineText::plain(format!("Quest failed · {}", quest.get(&content).title)),
    );
    notification::notify(world, player, failed);
    raise(world, quest, EventKind::Failed, player);
}

pub fn abandon(world: &mut World, player: Entity, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let Some(mut log) = world.get_mut::<QuestLog>(player) else {
        return;
    };
    if log.active(quest).is_none() {
        return;
    }
    log.active.retain(|active| active.quest != quest);
    log.tracked.retain(|&tracked| tracked != quest);
    take_back(world, player, quest.get(&content).grants());
    tell(
        world,
        player,
        format!("Quest abandoned · {}", quest.get(&content).title),
        &AttentionMark::QuestInProgress.get(&content).icon,
        Some(InterfaceSound::QuestAbandoned.id(&content)),
    );
    raise(world, quest, EventKind::Failed, player);
}

pub fn complete_task(world: &mut World, player: Entity, quest: QuestId, index: usize) {
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let done = world.get_mut::<QuestLog>(player).is_some_and(|mut log| {
        log.active
            .iter_mut()
            .find(|active| active.quest == quest)
            .and_then(|active| active.counts.get_mut(index))
            .is_some_and(|count| {
                let fresh = *count == 0;
                *count = 1;
                fresh
            })
    });
    if !done {
        return;
    }
    let def = quest.get(&content);
    if let Some(objective) = def.objectives().nth(index) {
        let parties = world.resource::<Parties>().clone();
        tell(
            world,
            player,
            format!(
                "Objective done · {}",
                def.objective_label(&content, &parties, quest, objective)
            ),
            objective.icon(&content),
            Some(InterfaceSound::TallyTick.id(&content)),
        );
    }
    announce(world, player, clock);
}

pub fn decide(world: &mut World, player: Entity, quest: QuestId, path: &str) {
    if let Some(mut log) = world.get_mut::<QuestLog>(player)
        && let Some(active) = log.active.iter_mut().find(|active| active.quest == quest)
    {
        active.decided = Some(path.to_owned());
    }
}

fn raise(world: &mut World, quest: QuestId, kind: EventKind, player: Entity) {
    world.resource_mut::<RiftInstances>().raise(
        ScriptOwner::Row(OwnerKind::Quest, quest.raw()),
        kind,
        Some(player),
        Vec::new(),
        |_| true,
    );
}

fn tell(world: &mut World, player: Entity, text: String, icon: &AssetRef, sfx: Option<SfxId>) {
    let told = Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Quest,
            icon: Some(icon.path()),
            tally: None,
            failure: false,
            sfx,
        },
        LineText::plain(text),
    );
    notification::notify(world, player, told);
}

pub fn requests(world: &mut World) {
    for request in drain_requests::<QuestRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        match request.message {
            QuestRequest::Track { quest, tracked } => track(world, player, quest, tracked),
            QuestRequest::Abandon { quest } => abandon(world, player, quest),
        }
    }
}

pub fn credit(world: &mut World, mut kills: Local<MessageCursor<KillCredited>>) {
    let kills: Vec<KillCredited> = kills
        .read(world.resource::<Messages<KillCredited>>())
        .copied()
        .collect();
    for kill in kills {
        credit_kill(world, kill);
    }
}

pub fn refresh(world: &mut World, logs: &mut QueryState<Entity, With<QuestLog>>) {
    let clock = *world.resource::<WallClock>();
    let players: Vec<Entity> = logs.iter(world).collect();
    for player in players {
        announce(world, player, clock);
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, quest) in loading.rows::<QuestDef>(&content) {
        let key = id.key(&content);
        let mut check = RowCheck::default();
        let item_rows = |stacks: &[ItemStack]| -> Vec<ItemId> {
            stacks
                .iter()
                .filter(|stack| content.table::<ItemDef>().get(stack.item).is_some())
                .map(|stack| stack.item)
                .collect()
        };
        for (index, module) in quest.modules.iter().enumerate() {
            match module {
                QuestModule::Shown { condition } | QuestModule::Requires { condition } => {
                    if let Err(fault) = condition::check(&content, condition) {
                        check.error(
                            format!("/modules/{index}/condition"),
                            fault.rule,
                            "the condition",
                            fault.expected,
                        );
                    }
                }
                QuestModule::Grants { items } => {
                    for (at, item) in item_rows(items).into_iter().enumerate() {
                        if !item.get(&content).has(ItemFlag::Quest) {
                            check.error(
                                format!("/modules/{index}/items/{at}/item"),
                                "not a quest item",
                                format!("grants {}", item.key(&content)),
                                "an item with quest_item",
                            );
                        }
                    }
                }
                QuestModule::Drop(drop)
                    if content.table::<ItemDef>().get(drop.item).is_some()
                        && !drop.item.get(&content).has(ItemFlag::Quest) =>
                {
                    check.error(
                        format!("/modules/{index}/item"),
                        "not a quest item",
                        format!("drops {}", drop.item.key(&content)),
                        "an item with quest_item",
                    );
                }
                _ => {}
            }
        }
        if quest.objectives().next().is_none() {
            check.error(
                "/modules",
                "no objective",
                key,
                "at least one objective module",
            );
        }
        let slots = quest.rewards().len() + usize::from(!quest.pick_one().is_empty());
        if slots > INVENTORY_MAX as usize {
            check.error(
                "/modules",
                "rewards don't fit",
                format!("{slots} slots"),
                format!("at most {INVENTORY_MAX} slots of rewards, a pick counting as one"),
            );
        }
        if quest.decision().is_some() && !quest.pick_one().is_empty() {
            check.error(
                "/modules",
                "decision and pick",
                key,
                "a decide objective settles the quest, so no pick_one",
            );
        }
        if quest.decision().is_none() && quest.pick_one().is_empty() && quest.hand_over().is_empty()
        {
            check.error(
                "/modules",
                "nothing to say",
                key,
                "a decision, a pick_one or a hand_over line",
            );
        }
        if let Err(fault) = dialogue::check_reveal(
            &quest.reveal,
            Revealable {
                needs: quest.requires().is_some(),
                costs: !quest.hand_in().is_empty(),
                gains: !quest.rewards().is_empty() || !quest.pick_one().is_empty() || quest.xp > 0,
            },
        ) {
            check.error(
                "/reveal",
                fault.rule,
                fault.expected,
                "reveal only what the quest has, once",
            );
        }
        loading.row_errors(id, check);
    }
}

pub fn check_parties(loading: &mut Loading, scripts: &RiftScripts, parties: &Parties) {
    let content = loading.content();
    let mut offered = HashSet::new();
    let mut turned_in = HashSet::new();
    for (_, role) in quest_roles(scripts) {
        match role {
            QuestRole::Gives(quest) => offered.insert(quest),
            QuestRole::TurnsIn(quest) => turned_in.insert(quest),
        };
    }
    for (id, _) in loading.rows::<QuestDef>(&content) {
        let key = id.key(&content);
        let found = parties.get(id);
        let givers = found.is_some_and(|found| !found.givers.is_empty());
        let turn_ins = found.is_some_and(|found| !found.turn_ins.is_empty());
        if !offered.contains(&id) {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "never offered",
                    key,
                    "a script that may reach offer_quest or accept_quest for it",
                ),
            );
        } else if !givers {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "lives nowhere",
                    key,
                    "an owner placed in an area that may offer it",
                ),
            );
        }
        if !turned_in.contains(&id) {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "never turned in",
                    key,
                    "a script that may reach turn_in for it",
                ),
            );
        } else if !turn_ins {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "lives nowhere",
                    key,
                    "an owner placed in an area that may turn it in",
                ),
            );
        }
    }
}

fn track(world: &mut World, player: Entity, quest: QuestId, tracked: bool) {
    let content = world.resource::<Content>().clone();
    let Some(mut log) = world.get_mut::<QuestLog>(player) else {
        return;
    };
    let trackable = log.trackable(&content, quest);
    log.tracked.retain(|&other| other != quest);
    if tracked && trackable {
        log.tracked.push(quest);
    }
}

pub fn offer_refusal(world: &World, player: Entity, quest: QuestId) -> Option<String> {
    let content = world.resource::<Content>();
    let log = log_of(world, player)?;
    let def = quest.get(content);
    if log.active(quest).is_some() {
        return Some(format!("You're already on {}", def.title));
    }
    if log.done_for_now(content, quest, world.resource::<WallClock>().day()) {
        return Some(format!("{} is done for now", def.title));
    }
    if log.active.len() >= QUEST_LOG_CAP {
        return Some(format!("Your quest log is full ({QUEST_LOG_CAP})"));
    }
    None
}

fn accept_refusal(world: &World, player: Entity, quest: QuestId) -> Option<String> {
    if let Some(refusal) = offer_refusal(world, player, quest) {
        return Some(refusal);
    }
    let def = quest.get(world.resource::<Content>());
    condition::first_unmet(world, player, def.requires().into_iter().chain(def.shown()))
        .map(|unmet| format!("Needs {unmet}"))
}

pub fn open_to(world: &World, player: Entity, quest: QuestId) -> bool {
    let content = world.resource::<Content>();
    quest
        .get(content)
        .requires()
        .is_none_or(|requires| condition::holds(world, player, requires))
}

fn shown_to(world: &World, player: Entity, quest: QuestId) -> bool {
    let content = world.resource::<Content>();
    quest
        .get(content)
        .shown()
        .is_none_or(|shown| condition::holds(world, player, shown))
}

fn take_back(world: &mut World, player: Entity, grants: &[ItemStack]) {
    if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
        inventory.take_up_to(grants);
    }
}

/// Time-driven and news-bearing changes to a player's quests: expiries, and the moment a quest
/// becomes ready or a daily quest is open again. What the log shows is read from the state
/// itself, so this only runs where something may have changed.
fn announce(world: &mut World, player: Entity, clock: WallClock) {
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let Some(log) = log_of(world, player) else {
        return;
    };
    if log.active.is_empty() && log.finished.is_empty() {
        return;
    }
    let expired: Vec<QuestId> = log
        .active
        .iter()
        .filter(|active| {
            active
                .deadline
                .is_some_and(|deadline| clock.now >= deadline)
        })
        .map(|active| active.quest)
        .collect();
    for quest in expired {
        fail(world, player, quest);
    }
    let inventory = world.get::<Inventory>(player).cloned();
    let Some(mut log) = log_of(world, player).cloned() else {
        return;
    };
    let mut told: Vec<(String, &AssetRef, Option<SfxId>)> = Vec::new();
    let ready: Vec<QuestId> = log
        .active
        .iter()
        .filter(|active| active.ready(&content, &parties, inventory.as_ref()))
        .map(|active| active.quest)
        .collect();
    for &quest in &ready {
        if !log.told_ready.contains(&quest) {
            log.told_ready.push(quest);
            let def = quest.get(&content);
            told.push((
                format!("Quest ready · {}", def.hand_in_hint(&parties, quest)),
                &ready_mark(def).get(&content).icon,
                Some(InterfaceSound::UiChime.id(&content)),
            ));
        }
    }
    log.told_ready.retain(|quest| ready.contains(quest));
    for finished in &mut log.finished {
        let def = finished.quest.get(&content);
        if finished.resets_at.is_some_and(|at| clock.now >= at) {
            finished.resets_at = None;
            told.push((
                format!("{} is open again", def.title),
                &AttentionMark::RepeatableOffered.get(&content).icon,
                Some(InterfaceSound::UiPage.id(&content)),
            ));
        }
        finished.offered_again =
            finished.result == QuestResult::Failed && shown_to(world, player, finished.quest);
    }
    let trackable: Vec<QuestId> = log
        .tracked
        .iter()
        .copied()
        .filter(|&quest| log.trackable(&content, quest))
        .collect();
    log.tracked = trackable;
    if world.get::<QuestLog>(player) != Some(&log) {
        world.entity_mut(player).insert(log);
    }
    for (text, icon, sfx) in told {
        tell(world, player, text, icon, sfx);
    }
}

fn credit_kill(world: &mut World, kill: KillCredited) {
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let player = kill.credited;
    let Some(mut log) = log_of(world, player).cloned() else {
        return;
    };
    let inventory = world.get::<Inventory>(player).cloned();
    let mut counted = Vec::new();
    let mut drops = Vec::new();
    world.resource_scope(|_, mut rng: Mut<Rng>| {
        for active in &mut log.active {
            let def = active.quest.get(&content);
            for (index, objective) in def.objectives().enumerate() {
                if let Objective::Defeat { npc, count } = *objective
                    && npc == kill.npc
                    && active.counts[index] < count
                {
                    active.counts[index] += 1;
                    counted.push(Notification::new(
                        NotificationKind::Feed {
                            topic: HistoryTopic::Quest,
                            icon: Some(objective.icon(&content).path()),
                            tally: Some(RecordTally::Progress {
                                have: active.counts[index],
                                need: count,
                            }),
                            failure: false,
                            sfx: None,
                        },
                        LineText::plain(def.objective_label(
                            &content,
                            &parties,
                            active.quest,
                            objective,
                        )),
                    ));
                }
            }
            for drop in def.drops().filter(|drop| drop.from == kill.npc) {
                if !still_needs(def, drop.item, inventory.as_ref()) {
                    continue;
                }
                if drop
                    .chance
                    .plus(drop.grows.times(active.dry_kills))
                    .rolled(&mut rng)
                {
                    drops.push((drop.item, 1));
                    active.dry_kills = 0;
                } else {
                    active.dry_kills += 1;
                }
            }
        }
    });
    world.entity_mut(player).insert(log);
    let clock = *world.resource::<WallClock>();
    announce(world, player, clock);
    for notification in counted {
        notification::notify(world, player, notification);
    }
    if let Some(client) = world.get::<Owner>(player).map(|owner| owner.client) {
        scatter_drop(
            world,
            kill.victim,
            &drops,
            Some(client),
            Some(Presence::For(client)),
        );
    }
}

fn still_needs(def: &QuestDef, item: ItemId, inventory: Option<&Inventory>) -> bool {
    let held = inventory.map_or(0, |inventory| inventory.count(item));
    def.objectives().any(|objective| match objective {
        Objective::Hold(stack) => stack.item == item && held < stack.count,
        Objective::Deliver { item: wanted } => *wanted == item && held == 0,
        _ => false,
    })
}

pub fn pick_label(content: &Content, pick: ItemStack) -> String {
    match pick.count {
        1 => format!("I'll take the {}.", pick.item.get(content).name),
        _ => format!("I'll take {}.", pick.describe(content)),
    }
}

fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = script::owner_of(world, target) else {
        return;
    };
    let Some(may_do) = world.resource::<RiftInstances>().may_do(owner) else {
        return;
    };
    let mut quests: Vec<(QuestId, bool, bool)> = Vec::new();
    for reach in &may_do.reaches {
        let (quest, gives, takes) = match reach {
            Reach::Offers(quest) | Reach::Accepts(quest) => (*quest, true, false),
            Reach::TurnsIn(quest) => (*quest, false, true),
            _ => continue,
        };
        match quests.iter_mut().find(|(known, ..)| *known == quest) {
            Some(known) => {
                known.1 |= gives;
                known.2 |= takes;
            }
            None => quests.push((quest, gives, takes)),
        }
    }
    if quests.is_empty() {
        return;
    }
    let today = world.resource::<WallClock>().day();
    let content = world.resource::<Content>();
    let checks: Vec<QuestCheck> = quests
        .into_iter()
        .map(|(quest, gives, takes)| {
            let def = quest.get(content);
            QuestCheck {
                quest,
                gives,
                takes,
                shown: def
                    .shown()
                    .map(|shown| CompiledCondition::new(content, shown)),
                requires: def
                    .requires()
                    .map(|requires| CompiledCondition::new(content, requires)),
            }
        })
        .collect();
    for &player in viewers {
        let Some(log) = log_of(world, player) else {
            continue;
        };
        for check in &checks {
            let def = check.quest.get(content);
            let kind = match log.active(check.quest) {
                Some(_) if check.takes && ready_to_hand_in(world, player, check.quest) => {
                    ready_mark(def)
                }
                Some(_) if check.takes => AttentionMark::QuestInProgress,
                Some(_) => continue,
                None if check.gives
                    && !log.done_for_now(content, check.quest, today)
                    && check
                        .shown
                        .as_ref()
                        .is_none_or(|shown| shown.holds(world, player)) =>
                {
                    let open = check
                        .requires
                        .as_ref()
                        .is_none_or(|requires| requires.holds(world, player));
                    if open {
                        offered_mark(def)
                    } else if def.reveals(ChoiceReveal::Locked) {
                        AttentionMark::QuestLocked
                    } else {
                        continue;
                    }
                }
                None => continue,
            };
            out(player, kind.id(content));
        }
    }
}

struct QuestCheck {
    quest: QuestId,
    gives: bool,
    takes: bool,
    shown: Option<CompiledCondition>,
    requires: Option<CompiledCondition>,
}

pub fn ready_mark(def: &QuestDef) -> AttentionMark {
    if def.daily() {
        AttentionMark::RepeatableReady
    } else {
        AttentionMark::QuestReady
    }
}

pub fn offered_mark(def: &QuestDef) -> AttentionMark {
    if def.daily() {
        AttentionMark::RepeatableOffered
    } else {
        AttentionMark::QuestOffered
    }
}
