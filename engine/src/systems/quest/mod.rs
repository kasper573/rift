pub mod card;
pub mod effects;
pub mod log;
pub mod query;
pub mod tracker;

use crate::core::script::EventDef;
use crate::core::script::ReachDef;
use crate::core::script::RunnerSpec;
use crate::core::script::Vocabulary;
use crate::systems::area::OBJECT_RUNNER;
use crate::systems::area::spawnable::SPAWNABLE_RUNNER;
use crate::systems::class;
use crate::systems::item::ITEM_RUNNER;
use crate::systems::npc::NPC_RUNNER;
use crate::systems::requirement;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::core::par;
use crate::core::tick::{Cached, Planner};
use crate::core::units::{Percent, Seconds};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString};

use crate::core::audio::playback::SfxId;
use crate::core::card::CardStyleId;
use crate::core::content::ImageRef;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Fixture, Loading, ModuleRule, ModuleSet,
    NamedContent, Nearby, RowCheck,
};
use crate::core::math::Rng;
use crate::core::script::ChoiceReveal;
use crate::core::script::condition::{self, CompiledCondition};
use crate::core::script::{
    EventKind, Expr, Instances, Owner, OwnerRow, RunnerKind, ScriptModule, ScriptedRow, Scripts,
};
use crate::core::text::LineText;
use crate::core::time::{GameDay, UnixMillis, WallClock};
use crate::systems::GameSettings;
use crate::systems::area::spawnable::SpawnableDef;
use crate::systems::area::{self, AreaDef, AreaId, AreaTag};
use crate::systems::attention::{AttentionId, AttentionMark};
use crate::systems::combat::credit::KillCredited;
use crate::systems::dialogue::span::ScriptSpan;
use crate::systems::dialogue::{self, Revealable};
use crate::systems::equipment::{self, Belongings, Equipment};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::deliver::DeliveryReason;
use crate::systems::item::{
    self, DropRow, Inventory, ItemId, ItemModuleKind, ItemStack, scatter_at,
};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::NpcId;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{PlayedBy, player_requests};
use crate::systems::reward::{self, Reward};
use crate::systems::visibility::Presence;

messages! {
    /// What the game says of quests.
    pub struct QuestMessages {
        accept,
        /// The player's choice turning a quest offer down.
        decline,
        /// The player's choice leaving a quest unhanded for now.
        not_yet,
        /// What a player is told offered a quest with every slot of their quest log taken.
        log_full,
        /// The title of the message the game sends a player whose bag had no room for the rewards of
        /// the quest `{quest}`.
        rewards_title(quest: Text),
        /// The text of the message the game sends a player whose bag had no room for the rewards of
        /// the quest `{quest}`.
        rewards_text,
        /// How a delivery objective reads, bringing `{item}` to `{taker}`.
        bring[item: Line, taker: Line],
        /// Who a delivery objective names as `{taker}` when no one takes the delivery.
        its_taker,
        /// How an objective to defeat `{npc}` reads.
        defeat(npc: Text),
        /// What a quest card calls one kind of quest, `{kind}` being `collect`, `defeat`, `explore`,
        /// `deliver`, `choice`, `timed` or `daily`, and `{position}` `first` in the list of kinds
        /// the card names or `later` after it.
        kind(kind: Text, position: Text),
        /// The line under a quest's name, of its minimum `{level}` and who it comes `{giver}`.
        meta(giver: Text, level: Number),
        /// Who a quest comes from, as `{giver}`, when no one offers it.
        no_giver,
        /// What to do to hand in a quest the one who gave it takes back, to `{taker}`.
        return_to[taker: Line],
        /// What to do to hand in a quest another takes, to `{taker}`.
        go_to[taker: Line],
        /// What to do to hand in a quest the one who gave it takes back, when no one is named.
        return_and_hand_in,
        /// What to do to hand in a quest another takes, when no one is named.
        go_and_hand_in,
        /// What to do to take on a failed quest again when an item offers it, `{giver}`.
        use_to_retry[giver: Line],
        /// What to do to take on a failed quest again when someone offers it, `{giver}`.
        talk_to_retry[giver: Line],
        /// What to do to take on a failed quest again when no one is named.
        try_again,
        /// When a quest done for today is open again, in `{time}`.
        resets_in(time: Text),
        /// How long a timed quest has left, as a clock, `{clock}`.
        clock_left(clock: Text),
        /// What the tracker says of a daily quest done for today.
        done_today,
        /// What a quest card says of a daily quest done for today, with when it `{resets}`.
        done_today_resets(resets: Text),
        /// What the player is told on taking `{quest}`.
        accepted[quest: Line],
        /// The heading of the notice that a quest is complete.
        complete_label,
        /// What the player is told when `{quest}` fails.
        failed_notice(quest: Text),
        /// What the player is told on abandoning `{quest}`.
        abandoned_notice(quest: Text),
        /// What the player is told on finishing the objective `{objective}`.
        objective_done[objective: Line],
        /// What the player is told when a quest can be handed in, with what to do about it, `{hint}`.
        ready_notice[hint: Line],
        /// What the player is told when `{quest}` can be taken again.
        open_again[quest: Line],
        /// What the player is told handing in `{quest}` before it is done.
        not_ready[quest: Line],
        /// What the player is told offered `{quest}` while on it.
        already_on[quest: Line],
        /// What the player is told offered `{quest}` while it is done for the day.
        done_for_now[quest: Line],
        /// What the player is told taking a quest with nowhere to keep it.
        no_log,
        /// What the player says picking one reward item, `{item}`.
        take_item[item: Line],
        /// What the player says picking a reward of several items, `{items}`.
        take_items[items: Line],
        /// The quest log's tab of quests in progress.
        tab_active,
        /// A quest log tab, `{tab}`, with how many quests it holds, `{count}`.
        tab_label(count: Number, tab: Text),
        /// The heading over the quests in progress.
        active_title,
        /// The quest log's tab and heading of finished quests, and what a finished quest reads as.
        completed,
        /// What a failed quest reads as.
        failed,
        /// How many quests are in progress, `{count}`, of how many the log holds, `{limit}`.
        active_count(count: Number, limit: Number),
        /// How many quests are finished, `{count}`.
        finished_count(count: Number),
        /// What the quest log says with no quest in progress.
        none_yet,
        /// What the quest log says with no quest finished.
        nothing_finished,
        /// What the quest list says of a quest waiting for a decision.
        status_choice,
        /// What the quest list says of a quest ready to hand in.
        status_ready,
        /// What the quest list says of a daily quest in progress.
        status_daily,
        /// What the quest list says of a failed quest.
        status_failed,
        /// What the quest list says of a finished quest.
        status_done,
        /// The heading over a quest's objectives.
        objectives,
        /// The heading over a quest card's objectives.
        card_objectives,
        /// The heading over a quest card's rewards.
        card_rewards,
        /// The heading over the rewards of which the player picks one, on a quest card.
        card_pick_one,
        /// The heading over what the player hands in for a quest, on a quest card.
        card_hand_in,
        /// The heading over a quest's choices.
        choices,
        /// The heading over a quest's rewards.
        rewards,
        /// The heading over the rewards of which the player picks one on returning.
        pick_one,
        /// The heading over what the player hands in for a quest.
        hand_in,
        /// The heading over what happens when a timed quest runs out.
        time_runs_out,
        /// What happens when a timed quest runs out, with what to do about it, `{hint}`.
        fails_retry[hint: Line],
        /// What the quest log says of a failed quest that is offered again, with what to do about it, `{hint}`.
        failed_retry[hint: Line],
        /// One path of a quest's choice, `{choice}`, and what it means, `{means}`.
        choice_means(choice: Text, means: Text),
        /// The checkbox that shows a quest on the HUD.
        track,
        /// The button that gives a quest up.
        abandon,
        /// The question confirming giving up `{quest}`.
        abandon_title[quest: Line],
        /// The button that confirms giving a quest up.
        abandon_confirm,
        /// The button that keeps a quest instead of giving it up.
        abandon_cancel,
        /// What the confirmation says of what a quest gave, `{item}`, going away.
        item_taken_back[item: Line],
        /// What the confirmation says of the progress a quest has.
        progress_lost,
        /// What the confirmation says of how to take the quest again, by using the item `{giver}`.
        offered_again_item(giver: Text),
        /// What the confirmation says of how to take the quest again, from `{giver}`.
        offered_again(giver: Text),
        /// What the confirmation says when nobody offers the quest again.
        offered_nobody,
        /// What the key that selects a quest does in the quest log.
        select_hint,
        /// What an item note says of an item that begins a quest.
        item_begins,
        /// What an item note says of a count of an item, `{count}`, wanted for `{quest}`.
        item_wanted(count: Number, quest: Text),
        /// What an item note says of a quest item taken back on abandoning `{quest}`.
        item_quest_given(quest: Text),
        /// What an item note says of a quest item.
        item_quest,
        /// The band of a quest card offering a quest.
        moment_offer,
        /// The band of a quest card completing a quest.
        moment_turn_in,
        /// The band of a quest card asking for a decision.
        moment_decision,
        /// The band of a quest card, of its moment, `{moment}`, and its category, `{category}`.
        band(category: Text, moment: Text),
        /// What a quest offer card says of how to answer.
        offer_foot,
        /// What a quest completion card says of how to pick a reward.
        pick_foot,
        /// How many of an item the player has, `{count}`.
        held(count: Number),
        /// The experience a quest gives, `{xp}`.
        reward_xp(xp: Number),
        /// How far along an objective is on the tracker, `{have}` of `{need}`.
        of(have: Number, need: Number),
        /// How far along an objective is, `{have}` out of `{need}`, in the quest log and on quest cards.
        progress(have: Number, need: Number),
        /// A tracker row, its objective `{label}` beside its count or clock, `{count}`.
        row_counted[label: Line, count: Text],
        /// The heading of the quest tracker.
        tracker_title,
        /// The quest tracker's heading while it is folded away: `{count}` quests tracked.
        tracker_folded(count: Number),
        /// What a screen reader names the folded quest tracker's heading, which opens it.
        tracker_show,
        /// What a screen reader names the open quest tracker's heading, which folds it.
        tracker_hide,
        /// What the player is told when a quest is to be offered to nobody.
        nobody_to_offer,
        /// What the player is told when a quest is to be taken by nobody.
        nobody_to_accept,
        /// What the player is told when a quest is to be handed in by nobody.
        nobody_to_hand_in,
    }
}

pub type QuestId = ContentId<QuestDef>;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate_private::<QuestLog>()
        .add_client_message::<QuestRequest>();
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct QuestDef {
    pub title: String,
    pub category: String,
    pub blurb: String,
    /// A picture of the quest, of any size and in any colours: the log prints it in ink beside
    /// what it says of the quest, and reads best of one taller than it is wide.
    pub art: ImageRef,
    /// What the quest's card is drawn in. Left out, the game's default card style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_style: Option<CardStyleId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal: Vec<ChoiceReveal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<QuestModule>,
}

impl ContentFile for QuestDef {
    const TABLE: &'static str = "quest";

    fn fresh(near: &Nearby) -> Result<QuestDef, String> {
        Ok(QuestDef {
            title: near.name(),
            category: String::new(),
            blurb: String::new(),
            art: near.image()?,
            card_style: None,
            reveal: Vec::new(),
            modules: vec![
                QuestModule::Objective(Objective::Task {
                    task: "task".to_owned(),
                    label: "See it through".to_owned(),
                }),
                QuestModule::HandOver {
                    text: vec![ScriptSpan::plain("It's done.")],
                },
            ],
        })
    }
}

impl NamedContent for QuestDef {
    fn shown_name(&self) -> &str {
        &self.title
    }
}

impl ModuleSet for QuestDef {
    type Module = QuestModule;

    fn modules(&self) -> &[QuestModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<QuestModule> {
        &mut self.modules
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
    const OWNER: RunnerKind = QUEST_RUNNER;

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
        condition: Expr,
    },
    Requires {
        condition: Expr,
    },
    Grants {
        items: Vec<ItemStack>,
    },
    HandIn {
        items: Vec<ItemStack>,
    },
    HandOver {
        text: Vec<ScriptSpan>,
    },
    Reward(Reward),
    PickOne {
        items: Vec<ItemStack>,
    },
    Daily,
    TimeLimit {
        seconds: Seconds,
    },
    Drop(QuestDrop),
    Script(ScriptModule),
    #[serde(untagged)]
    Objective(Objective),
}

impl QuestDef {
    pub fn shown(&self) -> Option<&Expr> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::Shown { condition } => Some(condition),
            _ => None,
        })
    }

    pub fn requires(&self) -> Option<&Expr> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::Requires { condition } => Some(condition),
            _ => None,
        })
    }

    pub fn min_level(&self) -> u32 {
        self.requires()
            .into_iter()
            .flat_map(condition::parts)
            .filter_map(class::min_level)
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

    pub fn hand_over(&self) -> &[ScriptSpan] {
        self.modules
            .iter()
            .find_map(|module| match module {
                QuestModule::HandOver { text } => Some(text.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    pub fn reward(&self) -> Option<&Reward> {
        self.modules.iter().find_map(|module| match module {
            QuestModule::Reward(reward) => Some(reward),
            _ => None,
        })
    }

    /// The experience completing the quest gives.
    pub fn xp(&self) -> u32 {
        self.reward().map_or(0, |reward| reward.xp)
    }

    /// The items completing the quest gives, whole: a quest's reward is never left to chance.
    pub fn rewards(&self) -> impl Iterator<Item = ItemStack> + '_ {
        self.reward()
            .into_iter()
            .flat_map(|reward| &reward.items)
            .map(DropRow::whole)
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
    ) -> LineText {
        match objective {
            Objective::Hold(stack) => Mention::Item(stack.item).line(content),
            Objective::Defeat { npc, .. } => {
                LineText::plain(crate::systems::npc::defeat_label(content, *npc))
            }
            Objective::Task { label, .. } | Objective::Decide { label, .. } => {
                LineText::plain(label.clone())
            }
            Objective::Deliver { item } => {
                let words = words(content);
                words.bring(
                    Mention::Item(*item).span(content),
                    parties
                        .turn_in(quest)
                        .map_or(words.its_taker(), |party| party.name.as_str()),
                )
            }
        }
    }

    pub fn progress(
        &self,
        content: &Content,
        parties: &Parties,
        active: &ActiveQuest,
        belongings: Belongings<'_>,
    ) -> Vec<ObjectiveProgress> {
        let held = |item| belongings.count(item);
        self.objectives()
            .enumerate()
            .map(|(index, objective)| {
                let counted = active.counts.get(index).copied().unwrap_or(0);
                let need = objective.need();
                let (have, shown) = match objective {
                    Objective::Hold(stack) => (held(stack.item), true),
                    Objective::Defeat { .. } | Objective::Task { .. } => (counted, true),
                    Objective::Deliver { item } => (held(*item), false),
                    Objective::Decide { .. } => (0, false),
                };
                let have = have.min(need);
                ObjectiveProgress {
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
        belongings: Belongings<'_>,
    ) -> bool {
        self.objectives()
            .zip(self.progress(content, parties, active, belongings))
            .all(|(objective, progress)| {
                matches!(objective, Objective::Decide { .. }) || progress.done()
            })
    }

    pub fn kinds(&self, content: &Content) -> String {
        let messages = messages::of(content);
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
        let named = kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                messages
                    .quest
                    .kind(kind, if index == 0 { "first" } else { "later" })
            })
            .collect();
        messages.interface.listed_words(named)
    }

    pub fn meta(&self, content: &Content, parties: &Parties, quest: QuestId) -> String {
        let words = words(content);
        words.meta(
            parties
                .giver(quest)
                .map_or(words.no_giver(), |party| party.name.as_str()),
            self.min_level(),
        )
    }

    pub fn icon<'a>(&'a self, content: &'a Content) -> &'a ImageRef {
        if self.daily() {
            return &AttentionMark::RepeatableOffered.get(content).icon;
        }
        self.objectives()
            .next()
            .and_then(|objective| objective.icon(content))
            .unwrap_or(&AttentionMark::QuestOffered.get(content).icon)
    }

    pub fn hand_in_hint(&self, content: &Content, parties: &Parties, quest: QuestId) -> LineText {
        let giver = parties.giver(quest);
        let taker = parties.turn_in(quest);
        let words = words(content);
        let returns = giver == taker;
        match taker {
            Some(party) if returns => words.return_to(party.named(content)),
            Some(party) => words.go_to(party.named(content)),
            None if returns => LineText::plain(words.return_and_hand_in()),
            None => LineText::plain(words.go_and_hand_in()),
        }
    }

    pub fn retry_hint(&self, content: &Content, parties: &Parties, quest: QuestId) -> LineText {
        let words = words(content);
        match parties.giver(quest) {
            Some(party) if party.item.is_some() => words.use_to_retry(party.named(content)),
            Some(party) => words.talk_to_retry(party.named(content)),
            None => LineText::plain(words.try_again()),
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

#[derive(JsonSchema, Clone, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum_discriminants(
    name(ObjectiveKind),
    derive(EnumString),
    strum(serialize_all = "snake_case")
)]
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

impl ObjectiveKind {
    /// The kind of objective a `type` tag names, when it names one.
    pub fn of(tag: &str) -> Option<ObjectiveKind> {
        tag.parse().ok()
    }
}

impl Objective {
    pub fn icon<'a>(&'a self, content: &'a Content) -> Option<&'a ImageRef> {
        match self {
            Objective::Hold(stack) => Some(&stack.item.get(content).icon),
            Objective::Deliver { item } => Some(&item.get(content).icon),
            Objective::Defeat { .. } | Objective::Task { .. } | Objective::Decide { .. } => None,
        }
    }

    /// How much of it there is to do: the count to hold or defeat, else one.
    pub fn need(&self) -> u32 {
        match self {
            Objective::Hold(stack) => stack.count,
            Objective::Defeat { count, .. } => *count,
            Objective::Task { .. } | Objective::Deliver { .. } | Objective::Decide { .. } => 1,
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
pub struct ObjectiveProgress {
    pub label: LineText,
    pub have: u32,
    pub need: u32,
    pub counted: bool,
}

impl ObjectiveProgress {
    pub fn done(&self) -> bool {
        self.have >= self.need
    }
}

#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Parties(Arc<HashMap<QuestId, QuestParties>>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestParties {
    pub givers: Vec<QuestParty>,
    pub turn_ins: Vec<QuestParty>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestParty {
    pub name: String,
    pub item: Option<ItemId>,
    pub npc: Option<NpcId>,
}

impl QuestParty {
    /// The party as a line names it: an item as the interface mentions one, anyone else by name.
    pub fn named(&self, content: &Content) -> LineText {
        match self.item {
            Some(item) => Mention::Item(item).line(content),
            None => self.name.as_str().into(),
        }
    }
}

impl Parties {
    pub fn of(content: &Content, scripts: &Scripts) -> Parties {
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

    pub fn giver(&self, quest: QuestId) -> Option<&QuestParty> {
        self.get(quest).and_then(|parties| parties.givers.first())
    }

    pub fn turn_in(&self, quest: QuestId) -> Option<&QuestParty> {
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

fn quest_roles(scripts: &Scripts) -> Vec<(OwnerRow, QuestRole)> {
    let mut roles = Vec::new();
    for offers in scripts.reaching::<Offers>() {
        roles.push((offers.row, QuestRole::Gives(offers.reach.0)));
    }
    for accepts in scripts.reaching::<Accepts>() {
        roles.push((accepts.row, QuestRole::Gives(accepts.reach.0)));
    }
    for turns_in in scripts.reaching::<TurnsIn>() {
        roles.push((turns_in.row, QuestRole::TurnsIn(turns_in.reach.0)));
    }
    roles
}

fn party_name(content: &Content, owner: OwnerRow) -> Option<QuestParty> {
    let placed = |npc: NpcId| {
        content.ids::<AreaDef>().any(|area| {
            area::residents(area::placed(content, area)).any(|resident| resident == npc)
        })
    };
    Some(match owner.kind {
        NPC_RUNNER => {
            let npc: NpcId = owner.id();
            if !placed(npc) {
                return None;
            }
            QuestParty {
                name: npc.get(content).name.to_owned(),
                item: None,
                npc: Some(npc),
            }
        }
        OBJECT_RUNNER => {
            let area: AreaId = owner.id();
            let (_, object) = area::objects(area::placed(content, area))
                .find(|(index, _)| Some(*index) == owner.object)?;
            QuestParty {
                name: object.title.clone().or_else(|| object.name.clone())?,
                item: None,
                npc: None,
            }
        }
        ITEM_RUNNER => {
            let item: ItemId = owner.id();
            QuestParty {
                name: item.get(content).name.to_owned(),
                item: Some(item),
                npc: None,
            }
        }
        SPAWNABLE_RUNNER => QuestParty {
            name: owner.id::<SpawnableDef>().get(content).title.clone(),
            item: None,
            npc: None,
        },
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

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuestResult {
    Completed,
    Failed,
}

#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum QuestRequest {
    Track { quest: QuestId, tracked: bool },
    Abandon { quest: QuestId },
}

impl ActiveQuest {
    pub fn left(&self, now: UnixMillis) -> Option<Seconds> {
        self.deadline
            .map(|deadline| Seconds(deadline.since(now).0.ceil()))
    }

    pub fn ready(&self, content: &Content, parties: &Parties, belongings: Belongings<'_>) -> bool {
        self.quest
            .get(content)
            .complete(content, parties, self, belongings)
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
            .map(|at| at.since(now).up_to_minute())
    }
}

pub fn resets_label(content: &Content, left: Seconds) -> String {
    let messages = messages::of(content);
    messages.quest.resets_in(messages.time.rough(left))
}

fn words(content: &Content) -> &QuestMessages {
    &messages::of(content).quest
}

pub fn log_of(world: &World, player: Entity) -> Option<&QuestLog> {
    world.get::<QuestLog>(player)
}

pub fn ready_to_hand_in(world: &World, player: Entity, quest: QuestId) -> bool {
    let content = world.resource::<Content>();
    let parties = world.resource::<Parties>();
    log_of(world, player)
        .and_then(|log| log.active(quest))
        .is_some_and(|active| active.ready(content, parties, Belongings::by(world, player)))
}

pub fn accept(world: &mut World, player: Entity, quest: QuestId) -> Result<(), LineText> {
    if let Some(refusal) = accept_refusal(world, player, quest) {
        return Err(refusal);
    }
    let content = world.resource::<Content>().clone();
    let clock = *world.resource::<WallClock>();
    let def = quest.get(&content);
    item::effects::exchange(world, player, &[], def.grants())?;
    let mut log = world
        .get_mut::<QuestLog>(player)
        .ok_or_else(|| LineText::plain(words(&content).no_log()))?;
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
        words(&content).accepted(Mention::Quest(quest).span(&content)),
        Some(&offered_mark(def).get(&content).icon),
        Some(InterfaceSound::QuestAccepted.id(&content)),
    );
    raise(world, quest, EventKind::of::<Accepted>(), player);
    announce(world, player, clock);
    Ok(())
}

pub fn not_ready(content: &Content, quest: QuestId) -> LineText {
    words(content).not_ready(Mention::Quest(quest).line(content))
}

pub fn turn_in(
    world: &mut World,
    player: Entity,
    quest: QuestId,
    pick: Option<ItemStack>,
) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    if !ready_to_hand_in(world, player, quest) {
        return Err(not_ready(&content, quest));
    }
    equipment::take_belongings(world, player, def.hand_in())?;
    finish_completed(world, player, quest, pick);
    Ok(())
}

pub fn complete(world: &mut World, player: Entity, quest: QuestId) {
    finish_completed(world, player, quest, None);
}

/// Completes `quest` for `player`, who is given its reward and the item they picked.
fn finish_completed(world: &mut World, player: Entity, quest: QuestId, pick: Option<ItemStack>) {
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
    let mut reward = def.reward().cloned().unwrap_or_default();
    reward.items.extend(pick.map(DropRow::certain));
    let to = reward::Where::Bag(DeliveryReason::QuestCompleted(quest));
    reward::grant(world, &[player], &reward, to);
    notification::notify(
        world,
        player,
        None,
        Notification::new(
            NotificationKind::Milestone {
                label: words(&content).complete_label().to_owned().into(),
                topic: HistoryTopic::Quest,
                sfx: Some(InterfaceSound::QuestCompleted.id(&content)),
            },
            Mention::Quest(quest).line(&content),
        ),
    );
    raise(world, quest, EventKind::of::<Completed>(), player);
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
    equipment::take_belongings_up_to(world, player, quest.get(&content).grants());
    let failed = Notification::feed(
        HistoryTopic::Quest,
        words(&content).failed_notice(&quest.get(&content).title),
    )
    .icon(quest.get(&content).icon(&content).clone())
    .failed()
    .sound(InterfaceSound::QuestAbandoned.id(&content));
    notification::notify(world, player, None, failed);
    raise(world, quest, EventKind::of::<Failed>(), player);
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
    equipment::take_belongings_up_to(world, player, quest.get(&content).grants());
    tell(
        world,
        player,
        words(&content).abandoned_notice(&quest.get(&content).title),
        Some(&AttentionMark::QuestInProgress.get(&content).icon),
        Some(InterfaceSound::QuestAbandoned.id(&content)),
    );
    raise(world, quest, EventKind::of::<Failed>(), player);
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
            words(&content)
                .objective_done(def.objective_label(&content, &parties, quest, objective)),
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
    world.resource_mut::<Instances>().raise(
        Owner::Row(QUEST_RUNNER, quest.raw()),
        kind,
        Some(player),
        Vec::new(),
        |_| true,
    );
}

fn tell(
    world: &mut World,
    player: Entity,
    text: impl Into<LineText>,
    icon: Option<&ImageRef>,
    sfx: Option<SfxId>,
) {
    let mut told = Notification::feed(HistoryTopic::Quest, text);
    if let Some(icon) = icon {
        told = told.icon(icon.clone());
    }
    if let Some(sfx) = sfx {
        told = told.sound(sfx);
    }
    notification::notify(world, player, None, told);
}

pub fn requests(world: &mut World) {
    player_requests::<QuestRequest>(world, RequestNeeds::Nothing, |world, player, request| {
        match request {
            QuestRequest::Track { quest, tracked } => track(world, player, quest, tracked),
            QuestRequest::Abandon { quest } => abandon(world, player, quest),
        }
        Ok(())
    });
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

/// Tells every player what moved in their quest log, each replication tick: what each is told
/// is worked out on every core from the world as the tick found it, then told in the order the
/// players were found. A character with a quest past its deadline is settled on their own, as the
/// failing changes what the rest says.
#[derive(Default)]
pub struct AnnounceQuests {
    logs: Cached<Entity, With<QuestLog>>,
    announced: Vec<(Entity, Announcement)>,
}

impl Planner for AnnounceQuests {
    fn prepare(&mut self, world: &mut World) {
        self.logs.ready(world);
    }

    fn plan(&mut self, world: &World) {
        self.announced.clear();
        if !crate::systems::replication_tick(world) {
            return;
        }
        let clock = *world.resource::<WallClock>();
        let players: Vec<Entity> = self.logs.query(world).iter().collect();
        self.announced = players
            .par_iter()
            .with_min_len(par::grain(players.len()))
            .filter_map(|&player| Some((player, announcement(world, player, clock)?)))
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        let clock = *world.resource::<WallClock>();
        for (player, announcement) in self.announced.drain(..) {
            match announcement {
                Announcement::Expired => announce(world, player, clock),
                Announcement::Settled { log, told } => {
                    if let Some(log) = log {
                        world.entity_mut(player).insert(log);
                    }
                    for (text, icon, sfx) in told {
                        tell(world, player, text, Some(&icon), sfx);
                    }
                }
            }
        }
    }
}

/// What a character's log says this tick.
enum Announcement {
    /// A quest ran past its deadline: the log is settled once it has failed.
    Expired,
    Settled {
        log: Option<QuestLog>,
        told: Vec<(LineText, ImageRef, Option<SfxId>)>,
    },
}

fn announcement(world: &World, player: Entity, clock: WallClock) -> Option<Announcement> {
    let log = log_of(world, player)?;
    if log.active.is_empty() && log.finished.is_empty() {
        return None;
    }
    let expired = log.active.iter().any(|active| {
        active
            .deadline
            .is_some_and(|deadline| clock.now >= deadline)
    });
    if expired {
        return Some(Announcement::Expired);
    }
    let content = world.resource::<Content>();
    let parties = world.resource::<Parties>();
    let belongings = Belongings::by(world, player);
    let mut log = log.clone();
    let mut told: Vec<(LineText, ImageRef, Option<SfxId>)> = Vec::new();
    let ready: Vec<QuestId> = log
        .active
        .iter()
        .filter(|active| active.ready(content, parties, belongings))
        .map(|active| active.quest)
        .collect();
    for &quest in &ready {
        if !log.told_ready.contains(&quest) {
            log.told_ready.push(quest);
            let def = quest.get(content);
            told.push((
                words(content).ready_notice(def.hand_in_hint(content, parties, quest)),
                ready_mark(def).get(content).icon.clone(),
                Some(InterfaceSound::UiChime.id(content)),
            ));
        }
    }
    log.told_ready.retain(|quest| ready.contains(quest));
    for finished in &mut log.finished {
        if finished.resets_at.is_some_and(|at| clock.now >= at) {
            finished.resets_at = None;
            told.push((
                words(content).open_again(Mention::Quest(finished.quest).line(content)),
                AttentionMark::RepeatableOffered.get(content).icon.clone(),
                Some(InterfaceSound::UiPage.id(content)),
            ));
        }
        finished.offered_again =
            finished.result == QuestResult::Failed && shown_to(world, player, finished.quest);
    }
    let trackable: Vec<QuestId> = log
        .tracked
        .iter()
        .copied()
        .filter(|&quest| log.trackable(content, quest))
        .collect();
    log.tracked = trackable;
    let log = (log_of(world, player) != Some(&log)).then_some(log);
    Some(Announcement::Settled { log, told })
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, quest) in loading.rows::<QuestDef>(&content) {
        let key = id.path(&content);
        let mut check = RowCheck::default();
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
                    for (at, ItemStack { item, .. }) in items.iter().enumerate() {
                        if !item.get(&content).has(ItemModuleKind::QuestItem) {
                            check.error(
                                format!("/modules/{index}/items/{at}/item"),
                                "not a quest item",
                                format!("grants {}", item.path(&content)),
                                "an item with quest_item",
                            );
                        }
                    }
                }
                QuestModule::Drop(drop)
                    if !drop.item.get(&content).has(ItemModuleKind::QuestItem) =>
                {
                    check.error(
                        format!("/modules/{index}/item"),
                        "not a quest item",
                        format!("drops {}", drop.item.path(&content)),
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
        if quest
            .reward()
            .is_some_and(|reward| reward.items.iter().any(|row| row.chance.is_some()))
        {
            check.error(
                "/modules",
                "reward left to chance",
                key,
                "reward items given every time, as the quest's card tells them",
            );
        }
        if let Err(fault) = dialogue::check_reveal(
            &quest.reveal,
            Revealable {
                needs: quest.requires().is_some(),
                costs: !quest.hand_in().is_empty(),
                gains: quest.rewards().next().is_some()
                    || !quest.pick_one().is_empty()
                    || quest.xp() > 0,
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

/// What a whole game must do with its quests: offer and take back every one, from an owner
/// placed somewhere.
pub fn check_reached(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    check_parties(loading, scripts, content.built::<Parties>());
}

/// Works out who offers and takes back each quest, from the scripts that reach it.
pub fn build_parties(loading: &mut Loading) {
    let content = loading.content();
    let parties = Parties::of(&content, content.built::<Scripts>());
    loading.built(parties);
}

fn check_parties(loading: &mut Loading, scripts: &Scripts, parties: &Parties) {
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
        let key = id.path(&content);
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

pub fn offer_refusal(world: &World, player: Entity, quest: QuestId) -> Option<LineText> {
    let content = world.resource::<Content>();
    let log = log_of(world, player)?;
    if log.active(quest).is_some() {
        return Some(words(content).already_on(Mention::Quest(quest).span(content)));
    }
    if log.done_for_now(content, quest, world.resource::<WallClock>().day()) {
        return Some(words(content).done_for_now(Mention::Quest(quest).line(content)));
    }
    let settings = content.built::<GameSettings>();
    if log.active.len() >= settings.slots.quest_log as usize {
        return Some(LineText::plain(words(content).log_full()));
    }
    None
}

fn accept_refusal(world: &World, player: Entity, quest: QuestId) -> Option<LineText> {
    if let Some(refusal) = offer_refusal(world, player, quest) {
        return Some(refusal);
    }
    let def = quest.get(world.resource::<Content>());
    requirement::first_unmet(world, player, def.requires().into_iter().chain(def.shown())).map(
        |unmet| {
            let content = world.resource::<Content>();
            messages::of(content).dialogue.needs(unmet)
        },
    )
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

/// Time-driven and news-bearing changes to a character's quests: expiries, and the moment a quest
/// becomes ready or a daily quest is open again. What the log shows is read from the state
/// itself, so this only runs where something may have changed.
fn announce(world: &mut World, player: Entity, clock: WallClock) {
    let Some(log) = log_of(world, player) else {
        return;
    };
    if log.active.is_empty() && log.finished.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
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
    let Some(mut log) = log_of(world, player).cloned() else {
        return;
    };
    let mut told: Vec<(LineText, &ImageRef, Option<SfxId>)> = Vec::new();
    let ready: Vec<QuestId> = log
        .active
        .iter()
        .filter(|active| active.ready(&content, &parties, Belongings::by(world, player)))
        .map(|active| active.quest)
        .collect();
    for &quest in &ready {
        if !log.told_ready.contains(&quest) {
            log.told_ready.push(quest);
            let def = quest.get(&content);
            told.push((
                words(&content).ready_notice(def.hand_in_hint(&content, &parties, quest)),
                &ready_mark(def).get(&content).icon,
                Some(InterfaceSound::UiChime.id(&content)),
            ));
        }
    }
    log.told_ready.retain(|quest| ready.contains(quest));
    for finished in &mut log.finished {
        if finished.resets_at.is_some_and(|at| clock.now >= at) {
            finished.resets_at = None;
            told.push((
                words(&content).open_again(Mention::Quest(finished.quest).line(&content)),
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
        tell(world, player, text, Some(icon), sfx);
    }
}

fn credit_kill(world: &mut World, kill: KillCredited) {
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let player = kill.credited;
    let Some(mut log) = log_of(world, player).cloned() else {
        return;
    };
    let bag = world.get::<Inventory>(player).cloned();
    let worn = world.get::<Equipment>(player).cloned();
    let belongings = Belongings::of(bag.as_ref(), worn.as_ref());
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
                    counted.push(
                        Notification::feed(
                            HistoryTopic::Quest,
                            def.objective_label(&content, &parties, active.quest, objective),
                        )
                        .tally(RecordTally::Progress {
                            have: active.counts[index],
                            need: count,
                        }),
                    );
                }
            }
            for drop in def.drops().filter(|drop| drop.from == kill.npc) {
                if !still_needs(def, drop.item, belongings) {
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
        notification::notify(world, player, None, notification);
    }
    if let Some(client) = world.get::<PlayedBy>(player).map(|owner| owner.client)
        && let Some(area) = world.get::<AreaTag>(player).map(|tag| tag.area)
    {
        scatter_at(
            world,
            kill.at,
            area,
            &drops,
            Some(client),
            Some(Presence::For(client)),
            None,
        );
    }
}

fn still_needs(def: &QuestDef, item: ItemId, belongings: Belongings<'_>) -> bool {
    let held = belongings.count(item);
    def.objectives().any(|objective| match objective {
        Objective::Hold(stack) => stack.item == item && held < stack.count,
        Objective::Deliver { item: wanted } => *wanted == item && held == 0,
        _ => false,
    })
}

pub fn pick_label(content: &Content, pick: ItemStack) -> LineText {
    let words = words(content);
    match pick.count {
        1 => words.take_item(Mention::Item(pick.item).span(content)),
        _ => words.take_items(pick.named()),
    }
}

pub(crate) fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = crate::systems::area::script::owner_of(world, target) else {
        return;
    };
    let Some(may_do) = world.resource::<Instances>().may_do(owner) else {
        return;
    };
    let mut quests: Vec<(QuestId, bool, bool)> = Vec::new();
    let reached = may_do
        .reaching::<Offers>()
        .map(|offers| (offers.0, true, false))
        .chain(
            may_do
                .reaching::<Accepts>()
                .map(|accepts| (accepts.0, true, false)),
        )
        .chain(
            may_do
                .reaching::<TurnsIn>()
                .map(|turns_in| (turns_in.0, false, true)),
        );
    for (quest, gives, takes) in reached {
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

pub const QUEST_RUNNER: RunnerKind = RunnerKind::new("quest");

/// A script that may offer a quest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offers(pub QuestId);

impl ReachDef for Offers {}

/// A script that may have someone accept a quest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accepts(pub QuestId);

impl ReachDef for Accepts {}

/// A script that may take a quest back, once done.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnsIn(pub QuestId);

impl ReachDef for TurnsIn {}

/// A script that may fail a quest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fails(pub QuestId);

impl ReachDef for Fails {}

/// A script that may complete a task of a quest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Completes(pub QuestId);

impl ReachDef for Completes {}

/// A script that may decide a path of a quest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decides(pub QuestId);

impl ReachDef for Decides {}

/// A player took the quest.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Accepted {}

/// A player completed the quest.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Completed {}

/// A player failed the quest.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Failed {}

macro_rules! quest_events {
    ($($event:ident),*) => {$(
        impl EventDef for $event {
            fn owners(&self) -> &'static [RunnerKind] {
                &[QUEST_RUNNER]
            }
        }
    )*};
}

quest_events!(Accepted, Completed, Failed);

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .runner(
            QUEST_RUNNER,
            RunnerSpec::named_row::<QuestDef>()
                .stateless()
                .starting_with_world(),
        )
        .binder(|loading, content, bind| bind.bind_table::<QuestDef>(loading, content))
        .event::<Accepted>("accepted")
        .event::<Completed>("completed")
        .event::<Failed>("failed");
    effects::scripting(vocabulary);
}
