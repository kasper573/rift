pub mod card;
pub mod log;
pub mod tracker;

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::math::{Percent, Rng};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{GameDay, Seconds, UnixMillis, WallClock};
use crate::data::area::Id as AreaId;
use crate::data::attention::Id as AttentionId;
use crate::data::dialogue::Id as DialogueId;
use crate::data::item::Id as ItemId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::area::AreaDef;
use crate::systems::area::{self, AreaTag, MarkerName};
use crate::systems::attention;
use crate::systems::dialogue::{
    self, Asked, ChoiceReveal, ChoiceTag, GotoNode, Offer, Revealable, StartConversation, Then,
};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::interact::Counterpart;
use crate::systems::item::{
    GiveItems, INVENTORY_MAX, Inventory, ItemFlag, ItemStack, ReservedBy, scatter_drop,
};
use crate::systems::job::MinLevel;
use crate::systems::movement::position;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::{Owner, Xp, conn_player, sender_player};
use crate::systems::rewards::KillCredited;
use crate::systems::rule::{self, Outcome, Requirement, RuleContext};
use crate::systems::text::{LineText, Span, TextFill};
use crate::systems::visibility::Presence;

pub use crate::data::quest::Id as QuestId;

pub const QUEST_LOG_CAP: usize = 20;
const FOUND_WITHIN: Tiles = Tiles(3.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<QuestLog>()
        .add_client_message::<QuestRequest>(Channel::Ordered);
    dialogue::topic_source(app, topics);
    attention::mark_source(app, marks);
}

#[derive(Clone)]
pub struct QuestDef {
    pub title: &'static str,
    pub category: &'static str,
    pub giver: Giver,
    pub turn_in: Counterpart,
    pub level: MinLevel,
    pub shown: &'static [&'static dyn Requirement],
    pub requires: &'static [&'static dyn Requirement],
    pub blurb: &'static str,
    pub offer: DialogueId,
    pub waiting: DialogueId,
    pub thanks: DialogueId,
    pub hand_over: &'static [Span],
    pub grants: &'static [ItemStack],
    pub objectives: &'static [Objective],
    pub hand_in: &'static [ItemStack],
    pub rewards: &'static [ItemStack],
    pub pick_one: &'static [ItemStack],
    pub xp: u32,
    pub reveal: &'static [ChoiceReveal],
    pub repeat: Repeat,
    pub time_limit: Option<Seconds>,
    pub drops: &'static [QuestDrop],
}

impl crate::core::content::ContentRow for QuestDef {
    const TABLE: &'static str = "quest";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Giver {
    Npc(NpcId),
    Prop(PropId),
    Item(ItemId),
}

impl Giver {
    pub fn counterpart(self) -> Option<Counterpart> {
        match self {
            Giver::Npc(npc) => Some(Counterpart::Npc(npc)),
            Giver::Prop(prop) => Some(Counterpart::Prop(prop)),
            Giver::Item(_) => None,
        }
    }

    pub fn name(self, content: &Content) -> &'static str {
        match self {
            Giver::Npc(npc) => npc.get(content).display_name,
            Giver::Prop(prop) => prop.get(content).display_name,
            Giver::Item(item) => item.get(content).display_name,
        }
    }
}

pub enum Objective {
    Hold(ItemStack),
    Defeat {
        npc: NpcId,
        count: u32,
    },
    Explore {
        area: AreaId,
        at: MarkerName,
        label: &'static str,
    },
    Deliver(ItemId),
    Decide {
        label: &'static str,
        paths: &'static [DecisionPath],
    },
}

impl Objective {
    pub fn icon(&self, content: &Content) -> AssetRef {
        match self {
            Objective::Hold(stack) => stack.item.get(content).icon,
            Objective::Defeat { .. } => AssetRef("icons/cursors/swords001.png"),
            Objective::Explore { .. } => AssetRef("icons/misc/map.png"),
            Objective::Deliver(item) => item.get(content).icon,
            Objective::Decide { .. } => AssetRef("icons/cursors/question001.png"),
        }
    }
}

pub struct DecisionPath {
    pub choice: &'static str,
    pub means: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Once,
    Daily,
}

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

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct QuestLog {
    pub active: Vec<ActiveQuest>,
    pub finished: Vec<FinishedQuest>,
    pub tracked: Vec<QuestId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActiveQuest {
    pub quest: QuestId,
    pub counts: Vec<u32>,
    pub deadline: Option<UnixMillis>,
    pub left: Option<Seconds>,
    pub ready: bool,
    pub dry_kills: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FinishedQuest {
    pub quest: QuestId,
    pub result: QuestResult,
    pub day: GameDay,
    pub resets_in: Option<Seconds>,
    pub offered_again: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestResult {
    Completed,
    Failed,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum QuestRequest {
    Track { quest: QuestId, tracked: bool },
    Abandon { quest: QuestId },
}

impl QuestDef {
    pub fn objective_label(&self, content: &Content, objective: &Objective) -> String {
        match objective {
            Objective::Hold(stack) => stack.item.get(content).display_name.to_owned(),
            Objective::Defeat { npc, .. } => {
                format!("Defeat the {}", npc.get(content).display_name)
            }
            Objective::Explore { label, .. } | Objective::Decide { label, .. } => {
                (*label).to_owned()
            }
            Objective::Deliver(item) => format!(
                "Bring {} to {}",
                item.get(content).display_name,
                self.turn_in.name(content)
            ),
        }
    }

    pub fn progress(
        &self,
        content: &Content,
        active: &ActiveQuest,
        inventory: Option<&Inventory>,
    ) -> Vec<Progress> {
        let held = |item| inventory.map_or(0, |inventory| inventory.count(item));
        self.objectives
            .iter()
            .enumerate()
            .map(|(index, objective)| {
                let counted = active.counts.get(index).copied().unwrap_or(0);
                let (have, need, shown) = match objective {
                    Objective::Hold(stack) => {
                        (held(stack.item).min(stack.count), stack.count, true)
                    }
                    Objective::Defeat { count, .. } => (counted.min(*count), *count, true),
                    Objective::Explore { .. } => (counted.min(1), 1, true),
                    Objective::Deliver(item) => (held(*item).min(1), 1, false),
                    Objective::Decide { .. } => (0, 1, false),
                };
                Progress {
                    label: self.objective_label(content, objective),
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
        active: &ActiveQuest,
        inventory: Option<&Inventory>,
    ) -> bool {
        self.objectives
            .iter()
            .zip(self.progress(content, active, inventory))
            .all(|(objective, progress)| {
                matches!(objective, Objective::Decide { .. }) || progress.done()
            })
    }

    pub fn kinds(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        for objective in self.objectives {
            let kind = match objective {
                Objective::Hold(_) => "collect",
                Objective::Defeat { .. } => "defeat",
                Objective::Explore { .. } => "explore",
                Objective::Deliver(_) => "deliver",
                Objective::Decide { .. } => "choice",
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if self.time_limit.is_some() {
            kinds.push("timed");
        }
        if self.repeat == Repeat::Daily {
            kinds.push("daily");
        }
        match kinds.split_last() {
            Some((last, [])) => (*last).to_owned(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    }

    pub fn meta(&self, content: &Content) -> String {
        format!(
            "{} · Level {} · from {}",
            self.category,
            self.level.0,
            self.giver.name(content)
        )
    }

    pub fn icon(&self, content: &Content) -> AssetRef {
        if self.time_limit.is_some() {
            return AssetRef("icons/cursors/sandclock001.png");
        }
        if self.repeat == Repeat::Daily {
            return AttentionId::RepeatableOffered.get(content).icon;
        }
        self.objectives
            .first()
            .map_or(AttentionId::QuestOffered.get(content).icon, |objective| {
                objective.icon(content)
            })
    }

    pub fn hand_in_hint(&self, content: &Content) -> String {
        let verb = match self.giver.counterpart() == Some(self.turn_in) {
            true => "Return",
            false => "Go",
        };
        match self.turn_in {
            Counterpart::Npc(npc) => format!("{verb} to {}", npc.get(content).display_name),
            Counterpart::Prop(prop) => {
                format!(
                    "{verb} to the {}",
                    prop.get(content).display_name.to_lowercase()
                )
            }
        }
    }

    pub fn retry_hint(&self, content: &Content) -> String {
        match self.giver {
            Giver::Item(item) => format!("Use the {} to try again", item.get(content).display_name),
            giver => format!("Talk to {} to try again", giver.name(content)),
        }
    }

    pub fn reveals(&self, reveal: ChoiceReveal) -> bool {
        self.reveal.contains(&reveal)
    }

    pub fn decision(&self) -> Option<&'static [DecisionPath]> {
        self.objectives
            .iter()
            .find_map(|objective| match objective {
                Objective::Decide { paths, .. } => Some(*paths),
                _ => None,
            })
    }
}

impl ActiveQuest {
    pub fn fresh(content: &Content, quest: QuestId) -> ActiveQuest {
        ActiveQuest {
            quest,
            counts: vec![0; quest.get(content).objectives.len()],
            deadline: None,
            left: None,
            ready: false,
            dry_kills: 0,
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
                    QuestResult::Completed => quest.get(content).repeat == Repeat::Daily,
                })
    }

    pub fn done_for_now(&self, content: &Content, quest: QuestId, today: GameDay) -> bool {
        self.finished(quest).is_some_and(|finished| {
            finished.result == QuestResult::Completed
                && match quest.get(content).repeat {
                    Repeat::Once => true,
                    Repeat::Daily => finished.day == today,
                }
        })
    }

    fn finish(&mut self, quest: QuestId, result: QuestResult, day: GameDay) {
        self.active.retain(|active| active.quest != quest);
        self.finished.retain(|finished| finished.quest != quest);
        self.finished.push(FinishedQuest {
            quest,
            result,
            day,
            resets_in: None,
            offered_again: false,
        });
    }
}

pub struct OnQuest(pub QuestId);

impl Requirement for OnQuest {
    fn met(&self, world: &World, player: Entity) -> bool {
        log_of(world, player).is_some_and(|log| log.active(self.0).is_some())
    }

    fn describe(&self, content: &Content) -> String {
        format!("On {}", self.0.get(content).title)
    }
}

pub struct QuestDone(pub QuestId);

impl Requirement for QuestDone {
    fn met(&self, world: &World, player: Entity) -> bool {
        log_of(world, player)
            .and_then(|log| log.finished(self.0))
            .is_some_and(|finished| finished.result == QuestResult::Completed)
    }

    fn describe(&self, content: &Content) -> String {
        self.0.get(content).title.to_owned()
    }
}

pub struct QuestOnCooldown(pub QuestId);

impl Requirement for QuestOnCooldown {
    fn met(&self, world: &World, player: Entity) -> bool {
        let content = world.resource::<Content>();
        let today = world.resource::<WallClock>().day();
        self.0.get(content).repeat == Repeat::Daily
            && log_of(world, player).is_some_and(|log| log.done_for_now(content, self.0, today))
    }

    fn describe(&self, content: &Content) -> String {
        format!("{} done for today", self.0.get(content).title)
    }
}

pub struct QuestResetsIn(pub QuestId);

impl TextFill for QuestResetsIn {
    fn text(&self, world: &World, _player: Entity) -> String {
        world.resource::<WallClock>().until_next_day().in_words()
    }

    fn check(&self, content: &Content) {
        if self.0.get(content).repeat != Repeat::Daily {
            panic!("QuestResetsIn({:?}) never resets", self.0);
        }
    }
}

pub struct AcceptQuest(pub QuestId);

impl Outcome for AcceptQuest {
    fn apply(&self, ctx: &mut RuleContext) {
        accept(ctx.world, ctx.player, self.0);
    }

    fn blocked(&self, world: &World, player: Entity) -> Option<String> {
        accept_refusal(world, player, self.0)
    }

    fn gives(&self, content: &Content) -> Vec<ItemStack> {
        self.0.get(content).grants.to_vec()
    }
}

pub struct TurnIn(pub QuestId);

impl Outcome for TurnIn {
    fn apply(&self, ctx: &mut RuleContext) {
        complete(ctx.world, ctx.player, self.0);
    }

    fn blocked(&self, world: &World, player: Entity) -> Option<String> {
        let content = world.resource::<Content>();
        (!ready_to_hand_in(world, player, self.0))
            .then(|| format!("{} isn't ready to hand in", self.0.get(content).title))
    }

    fn takes(&self, content: &Content) -> Vec<ItemStack> {
        self.0.get(content).hand_in.to_vec()
    }

    fn gives(&self, content: &Content) -> Vec<ItemStack> {
        self.0.get(content).rewards.to_vec()
    }
}

pub struct FailQuest(pub QuestId);

impl Outcome for FailQuest {
    fn apply(&self, ctx: &mut RuleContext) {
        fail(ctx.world, ctx.player, self.0);
    }
}

pub struct OfferQuest(pub QuestId);

impl Outcome for OfferQuest {
    fn apply(&self, ctx: &mut RuleContext) {
        let content = ctx.world.resource::<Content>().clone();
        let def = self.0.get(&content);
        let node = match log_of(ctx.world, ctx.player).and_then(|log| log.active(self.0)) {
            Some(_) => def.waiting,
            None if offerable(ctx.world, ctx.player, self.0) => def.offer,
            None => return,
        };
        StartConversation { node }.apply(ctx);
    }

    fn leads_to(&self, content: &Content) -> Vec<DialogueId> {
        vec![self.0.get(content).offer]
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
    log_of(world, player)
        .and_then(|log| log.active(quest))
        .is_some_and(|active| {
            quest
                .get(content)
                .complete(content, active, world.get::<Inventory>(player))
        })
}

pub fn accept(world: &mut World, player: Entity, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let def = quest.get(&content);
    let Some(mut log) = world.get_mut::<QuestLog>(player) else {
        return;
    };
    if log.active(quest).is_some() {
        return;
    }
    log.finished
        .retain(|finished| finished.quest != quest || finished.result == QuestResult::Completed);
    log.active.push(ActiveQuest {
        deadline: def.time_limit.map(|limit| clock.now.after(limit)),
        left: def.time_limit,
        ..ActiveQuest::fresh(&content, quest)
    });
    if !log.tracked.contains(&quest) {
        log.tracked.push(quest);
    }
    tell(
        world,
        player,
        format!("Quest accepted · {}", def.title),
        offered_mark(def).get(&content).icon,
        Some(SfxId::QuestAccepted),
    );
    refresh_log(world, player, clock);
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
    log.finish(quest, QuestResult::Completed, clock.day());
    if def.repeat == Repeat::Once {
        log.tracked.retain(|&tracked| tracked != quest);
    }
    if let Some(mut xp) = world.get_mut::<Xp>(player) {
        xp.gain(def.xp);
    }
    notification::notify(
        world,
        player,
        Notification::new(
            NotificationKind::Milestone {
                label: "Quest complete".into(),
                topic: HistoryTopic::Quest,
                sfx: Some(SfxId::QuestCompleted),
            },
            LineText::plain(def.title),
        ),
    );
    refresh_log(world, player, clock);
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
    log.finish(quest, QuestResult::Failed, clock.day());
    take_back(world, player, quest.get(&content).grants);
    let failed = Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Quest,
            icon: Some(quest.get(&content).icon(&content).0.to_owned()),
            tally: None,
            failure: true,
            sfx: Some(SfxId::QuestAbandoned),
        },
        LineText::plain(format!("Quest failed · {}", quest.get(&content).title)),
    );
    notification::notify(world, player, failed);
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
    take_back(world, player, quest.get(&content).grants);
    tell(
        world,
        player,
        format!("Quest abandoned · {}", quest.get(&content).title),
        AttentionId::QuestInProgress.get(&content).icon,
        Some(SfxId::QuestAbandoned),
    );
}

fn tell(world: &mut World, player: Entity, text: String, icon: AssetRef, sfx: Option<SfxId>) {
    let told = Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Quest,
            icon: Some(icon.0.to_owned()),
            tally: None,
            failure: false,
            sfx,
        },
        LineText::plain(text),
    );
    notification::notify(world, player, told);
}

pub fn requests(world: &mut World) {
    for request in crate::systems::requests::<QuestRequest>(world) {
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
        refresh_log(world, player, clock);
    }
}

pub fn conversation_starts(content: &Content) -> Vec<DialogueId> {
    content
        .table::<QuestDef>()
        .rows()
        .iter()
        .flat_map(|quest| [quest.offer, quest.waiting, quest.thanks])
        .collect()
}

pub fn check(assets: &AssetService) {
    let content = assets.content();
    let places: Vec<Counterpart> = content
        .table::<AreaDef>()
        .rows()
        .iter()
        .flat_map(|area| {
            area.residents
                .iter()
                .map(|resident| Counterpart::Npc(resident.npc))
                .chain(
                    area.props
                        .iter()
                        .map(|fixture| Counterpart::Prop(fixture.prop)),
                )
        })
        .collect();
    for (id, quest) in content.table::<QuestDef>().iter() {
        let parties = quest.giver.counterpart().into_iter().chain([quest.turn_in]);
        for party in parties {
            if !places.contains(&party) {
                panic!("quest {id:?}: {party:?} lives nowhere");
            }
        }
        for stack in quest.grants {
            if !stack.item.get(content).has(ItemFlag::Quest) {
                panic!(
                    "quest {id:?}: grants {:?}, which isn't a quest item",
                    stack.item
                );
            }
        }
        for drop in quest.drops {
            if !drop.item.get(content).has(ItemFlag::Quest) {
                panic!(
                    "quest {id:?}: drops {:?}, which isn't a quest item",
                    drop.item
                );
            }
        }
        let slots = quest.rewards.len() + usize::from(!quest.pick_one.is_empty());
        if slots > INVENTORY_MAX as usize {
            panic!("quest {id:?}: its rewards can never fit in a bag");
        }
        if quest.decision().is_some() && !quest.pick_one.is_empty() {
            panic!("quest {id:?}: a decision settles itself, so it offers no pick");
        }
        if quest.decision().is_none() && quest.pick_one.is_empty() && quest.hand_over.is_empty() {
            panic!("quest {id:?}: nothing to say when handing it in");
        }
        dialogue::check_reveal(
            &format!("quest {id:?}"),
            quest.reveal,
            Revealable {
                needs: quest.level.0 > 1 || !quest.requires.is_empty(),
                costs: !quest.hand_in.is_empty(),
                gains: !quest.rewards.is_empty() || !quest.pick_one.is_empty() || quest.xp > 0,
            },
        );
        for objective in quest.objectives {
            if let Objective::Explore { area, at, .. } = objective
                && area::load(assets, *area).marker(*at).is_none()
            {
                panic!("quest {id:?}: {area:?} has no marker {:?}", at.0);
            }
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

fn accept_refusal(world: &World, player: Entity, quest: QuestId) -> Option<String> {
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
    std::iter::once(&def.level as &dyn Requirement)
        .chain(def.requires.iter().copied())
        .chain(def.shown.iter().copied())
        .find(|requirement| !requirement.met(world, player))
        .map(|unmet| format!("Needs {}", unmet.describe(content)))
}

fn offerable(world: &World, player: Entity, quest: QuestId) -> bool {
    log_of(world, player).is_some_and(|log| {
        let today = world.resource::<WallClock>().day();
        offerable_in(world, player, log, today, quest)
    })
}

fn offerable_in(
    world: &World,
    player: Entity,
    log: &QuestLog,
    today: GameDay,
    quest: QuestId,
) -> bool {
    let content = world.resource::<Content>();
    log.active(quest).is_none()
        && !log.done_for_now(content, quest, today)
        && rule::met(world, player, quest.get(content).shown)
}

fn open_to(world: &World, player: Entity, quest: QuestId) -> bool {
    let content = world.resource::<Content>();
    let def = quest.get(content);
    def.level.met(world, player) && rule::met(world, player, def.requires)
}

fn take_back(world: &mut World, player: Entity, grants: &[ItemStack]) {
    let content = world.resource::<Content>().clone();
    let Some(mut inventory) = world.get_mut::<Inventory>(player) else {
        return;
    };
    let held: Vec<ItemStack> = grants
        .iter()
        .map(|stack| ItemStack::new(stack.item, stack.count.min(inventory.count(stack.item))))
        .filter(|stack| stack.count > 0)
        .collect();
    let _ = inventory.exchange(&content, &held, &[]);
}

fn refresh_log(world: &mut World, player: Entity, clock: WallClock) {
    let content = world.resource::<Content>().clone();
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
    let found = explored(world, player);
    let inventory = world.get::<Inventory>(player).cloned();
    let Some(mut log) = log_of(world, player).cloned() else {
        return;
    };
    let mut told: Vec<(String, AssetRef, Option<SfxId>)> = Vec::new();
    for active in &mut log.active {
        let def = active.quest.get(&content);
        for &(quest, index) in &found {
            if quest == active.quest && active.counts[index] == 0 {
                active.counts[index] = 1;
                let objective = &def.objectives[index];
                told.push((
                    format!(
                        "Objective done · {}",
                        def.objective_label(&content, objective)
                    ),
                    objective.icon(&content),
                    Some(SfxId::TallyTick),
                ));
            }
        }
        active.left = active
            .deadline
            .map(|deadline| Seconds(deadline.since(clock.now).0.ceil()));
        let ready = def.complete(&content, active, inventory.as_ref());
        if ready && !active.ready {
            told.push((
                format!("Quest ready · {}", def.hand_in_hint(&content)),
                ready_mark(def).get(&content).icon,
                Some(SfxId::UiChime),
            ));
        }
        active.ready = ready;
    }
    let today = clock.day();
    for finished in &mut log.finished {
        let def = finished.quest.get(&content);
        let resets = def.repeat == Repeat::Daily
            && finished.result == QuestResult::Completed
            && finished.day == today;
        if finished.resets_in.is_some() && !resets {
            told.push((
                format!("{} is open again", def.title),
                AttentionId::RepeatableOffered.get(&content).icon,
                Some(SfxId::UiPage),
            ));
        }
        finished.resets_in = resets.then(|| {
            let minutes = (clock.until_next_day().0 / 60.0).ceil();
            Seconds(minutes * 60.0)
        });
        finished.offered_again =
            finished.result == QuestResult::Failed && rule::met(world, player, def.shown);
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

fn explored(world: &World, player: Entity) -> Vec<(QuestId, usize)> {
    let content = world.resource::<Content>();
    let (Some(log), Some(at), Some(here)) = (
        log_of(world, player),
        position(world, player),
        world.get::<AreaTag>(player).map(|tag| tag.area),
    ) else {
        return Vec::new();
    };
    let assets = world.resource::<AssetService>();
    log.active
        .iter()
        .flat_map(|active| {
            active
                .quest
                .get(content)
                .objectives
                .iter()
                .enumerate()
                .filter_map(move |(index, objective)| match objective {
                    Objective::Explore {
                        area, at: marker, ..
                    } if *area == here => Some((active.quest, index, *area, *marker)),
                    _ => None,
                })
        })
        .filter(|&(_, _, area, marker)| {
            area::load(assets, area)
                .marker(marker)
                .is_some_and(|spot| at.distance(spot.center()) <= FOUND_WITHIN)
        })
        .map(|(quest, index, ..)| (quest, index))
        .collect()
}

fn credit_kill(world: &mut World, kill: KillCredited) {
    let content = world.resource::<Content>().clone();
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
            for (index, objective) in def.objectives.iter().enumerate() {
                if let Objective::Defeat { npc, count } = *objective
                    && npc == kill.npc
                    && active.counts[index] < count
                {
                    active.counts[index] += 1;
                    counted.push(Notification::new(
                        NotificationKind::Feed {
                            topic: HistoryTopic::Quest,
                            icon: Some(objective.icon(&content).0.to_owned()),
                            tally: Some(RecordTally::Progress {
                                have: active.counts[index],
                                need: count,
                            }),
                            failure: false,
                            sfx: None,
                        },
                        LineText::plain(def.objective_label(&content, objective)),
                    ));
                }
            }
            for drop in def.drops.iter().filter(|drop| drop.from == kill.npc) {
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
    for notification in counted {
        notification::notify(world, player, notification);
    }
    if let Some(client) = world.get::<Owner>(player).map(|owner| owner.client) {
        scatter_drop(
            world,
            kill.victim,
            &drops,
            ReservedBy::Account(client),
            Some(Presence::For(client)),
        );
    }
}

fn still_needs(def: &QuestDef, item: ItemId, inventory: Option<&Inventory>) -> bool {
    let held = inventory.map_or(0, |inventory| inventory.count(item));
    def.objectives.iter().any(|objective| match objective {
        Objective::Hold(stack) => stack.item == item && held < stack.count,
        Objective::Deliver(wanted) => *wanted == item && held == 0,
        _ => false,
    })
}

fn topics(world: &World, asked: &Asked) -> Vec<Offer> {
    let mut offers = hand_ins(world, asked);
    if let Some(&counterpart) = asked
        .greeting(world.resource::<Content>())
        .and_then(|with| world.get::<Counterpart>(with))
    {
        offers.extend(
            world
                .resource::<Content>()
                .ids::<QuestDef>()
                .filter_map(|quest| greeting_topic(world, asked.player, counterpart, quest)),
        );
    }
    offers
}

fn hand_ins(world: &World, asked: &Asked) -> Vec<Offer> {
    let content = world.resource::<Content>();
    let Some((quest, def)) = content
        .table::<QuestDef>()
        .iter()
        .find(|(_, def)| def.thanks == asked.node)
    else {
        return Vec::new();
    };
    if def.decision().is_some() || !ready_to_hand_in(world, asked.player, quest) {
        return Vec::new();
    }
    if def.pick_one.is_empty() {
        return vec![quest_offer(
            def,
            LineText::spoken(def.hand_over, world, asked.player),
            None,
            Vec::new(),
            vec![Arc::new(TurnIn(quest))],
        )];
    }
    def.pick_one
        .iter()
        .map(|pick| {
            quest_offer(
                def,
                LineText::plain(pick_label(content, *pick)),
                Some(pick.item.get(content).icon),
                Vec::new(),
                vec![
                    Arc::new(TurnIn(quest)),
                    Arc::new(GiveItems(std::slice::from_ref(pick))),
                ],
            )
        })
        .collect()
}

fn pick_label(content: &Content, pick: ItemStack) -> String {
    match pick.count {
        1 => format!("I'll take the {}.", pick.item.get(content).display_name),
        _ => format!("I'll take {}.", pick.describe(content)),
    }
}

fn greeting_topic(
    world: &World,
    player: Entity,
    counterpart: Counterpart,
    quest: QuestId,
) -> Option<Offer> {
    let content = world.resource::<Content>();
    let def = quest.get(content);
    let log = log_of(world, player)?;
    let gives = def.giver.counterpart() == Some(counterpart);
    let takes = def.turn_in == counterpart;
    let label = || LineText::plain(def.title);
    match log.active(quest) {
        Some(active) if takes && active.ready => Some(quest_offer(
            def,
            label(),
            Some(ready_mark(def).get(content).icon),
            Vec::new(),
            vec![Arc::new(GotoNode(def.thanks))],
        )),
        Some(_) if gives => Some(quest_offer(
            def,
            label(),
            Some(AttentionId::QuestInProgress.get(content).icon),
            Vec::new(),
            vec![Arc::new(GotoNode(def.waiting))],
        )),
        Some(_) => None,
        None if gives && offerable(world, player, quest) => Some(quest_offer(
            def,
            label(),
            Some(offered_mark(def).get(content).icon),
            std::iter::once(Arc::new(def.level) as Arc<dyn Requirement>)
                .chain(
                    def.requires
                        .iter()
                        .map(|&requirement| Arc::new(requirement) as Arc<dyn Requirement>),
                )
                .collect(),
            vec![Arc::new(GotoNode(def.offer))],
        )),
        None => None,
    }
}

fn quest_offer(
    def: &QuestDef,
    label: LineText,
    icon: Option<AssetRef>,
    requires: Vec<Arc<dyn Requirement>>,
    then: Vec<Arc<dyn Outcome>>,
) -> Offer {
    Offer {
        label,
        tag: ChoiceTag::Quest,
        icon,
        requires,
        costs: &[],
        then: Then::Made(then),
        reveal: def.reveal,
        warn: None,
    }
}

fn marks(world: &World, player: Entity, target: Entity) -> Vec<AttentionId> {
    let Some(&counterpart) = world.get::<Counterpart>(target) else {
        return Vec::new();
    };
    let Some(log) = log_of(world, player) else {
        return Vec::new();
    };
    let today = world.resource::<WallClock>().day();
    world
        .resource::<Content>()
        .table::<QuestDef>()
        .iter()
        .filter_map(|(quest, def)| {
            let gives = def.giver.counterpart() == Some(counterpart);
            if !gives && def.turn_in != counterpart {
                return None;
            }
            let kind = match log.active(quest) {
                Some(active) if def.turn_in == counterpart && active.ready => ready_mark(def),
                Some(_) if def.turn_in == counterpart => AttentionId::QuestInProgress,
                Some(_) => return None,
                None if gives && offerable_in(world, player, log, today, quest) => {
                    if open_to(world, player, quest) {
                        offered_mark(def)
                    } else if def.reveals(ChoiceReveal::Locked) {
                        AttentionId::QuestLocked
                    } else {
                        return None;
                    }
                }
                None => return None,
            };
            Some(kind)
        })
        .collect()
}

fn ready_mark(def: &QuestDef) -> AttentionId {
    match def.repeat {
        Repeat::Once => AttentionId::QuestReady,
        Repeat::Daily => AttentionId::RepeatableReady,
    }
}

fn offered_mark(def: &QuestDef) -> AttentionId {
    match def.repeat {
        Repeat::Once => AttentionId::QuestOffered,
        Repeat::Daily => AttentionId::RepeatableOffered,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "lowercase")]
enum QuestStep {
    Accept,
    Ready,
    Fail,
    Abandon,
    Forget,
}

impl bevy_terminal::CommandArg for QuestStep {
    fn parse(name: &str, raw: Option<&str>) -> Result<QuestStep, String> {
        let raw = raw.ok_or_else(|| format!("missing {name}"))?;
        raw.parse().map_err(|_| {
            format!(
                "{name} must be one of {}",
                <QuestStep as strum::VariantNames>::VARIANTS.join(", ")
            )
        })
    }
}

/// Move one of your quests along: accept, ready, fail, abandon or forget.
#[bevy_terminal::command(name = "quest", access = crate::systems::account::role::is_admin)]
fn quest_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    quest: String,
    step: QuestStep,
) -> Result<String, String> {
    let quest = crate::core::content::named::<QuestDef>(world, &quest)?;
    let content = world.resource::<Content>().clone();
    let player = conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?;
    match step {
        QuestStep::Accept => accept(world, player, quest),
        QuestStep::Ready => {
            accept(world, player, quest);
            if let Some(mut log) = world.get_mut::<QuestLog>(player)
                && let Some(active) = log.active.iter_mut().find(|active| active.quest == quest)
            {
                for (count, objective) in
                    active.counts.iter_mut().zip(quest.get(&content).objectives)
                {
                    *count = match objective {
                        Objective::Defeat { count, .. } => *count,
                        _ => 1,
                    };
                }
            }
            let clock = *world.resource::<WallClock>();
            refresh_log(world, player, clock);
        }
        QuestStep::Fail => fail(world, player, quest),
        QuestStep::Abandon => abandon(world, player, quest),
        QuestStep::Forget => {
            if let Some(mut log) = world.get_mut::<QuestLog>(player) {
                log.active.retain(|active| active.quest != quest);
                log.finished.retain(|finished| finished.quest != quest);
                log.tracked.retain(|&tracked| tracked != quest);
            }
        }
    }
    Ok(format!("{quest:?}: {step:?}"))
}
