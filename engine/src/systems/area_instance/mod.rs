//! Instanced areas: an area that is never shared. Whoever enters gets an instance of it, a world of
//! its own, with goals to meet and rewards for meeting them. An instance opens for a roster that is
//! fixed then: a player alone, or the members a party's leader gathered. Nobody joins it later,
//! and whoever leaves it is gone from it for good. What the rewards are and where the roster
//! comes back to are decided by whatever let them in. Every instance is society's: it opens them,
//! counts their goals and their time, hands out their rewards and says when one is gone.

pub mod effect;
pub mod entrance;
pub mod gather;
pub mod plate;
pub mod query;
pub mod roster;

use crate::core::script::Vocabulary;
use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentProblem, Loading, RowCheck, SchemaMark, serial};
use crate::core::math::{Pos, Rng};
use crate::core::script::{Answer, NEXT, REFUSED};
use crate::core::script::{FlowKey, Instances};
use crate::core::text::LineText;
use crate::core::time::UnixMillis;
use crate::core::units::{Meters, Seconds};
use crate::systems::account::AccountId;
use crate::systems::area::Place;
use crate::systems::area::transition::Crossing;
use crate::systems::area::{self, AreaDef, AreaId};
use crate::systems::combat::Died;
use crate::systems::history::HistoryTopic;
use crate::systems::item::deliver::DeliveryReason;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::mount;
use crate::systems::notification::{self, Notification};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party::CharacterParties;
use crate::systems::player::snapshot::{self, CharacterSnapshot};
use crate::systems::player::{self, RequestNeeds};
use crate::systems::reward::{self, Reward, Rolled};
use crate::systems::society::{Settling, Society, SocietyMember};
use crate::systems::worlds::WorldName;
use crate::systems::{GameSettings, SETTINGS_FILE, identity};

use crate::systems::messages::{self, ArgumentKindOf, Line, Phrase, messages};
pub use effect::EntersInstance;
pub use roster::{CallReply, CallRequest, InstanceAsk};

messages! {
    /// What the game says of instanced areas.
    pub struct InstanceMessages {
        /// What a player is told at a door that would open an instance, while the server runs as
        /// many as it is set to.
        busy,
        /// What everyone inside an instance of `{area}` is told as its time runs out uncleared.
        timed_out[area: Line],
        /// The entrance panel's choice to open an instance of one's own.
        go_alone,
        /// The entrance panel's choice, for a party's leader, to call the party in, with `{count}` of them standing nearby.
        gather(count: Number),
        /// What a member of a party is told at an entrance: only `{leader}` takes the party in.
        leader_only(leader: Text),
        /// What an entrance or a roster says while fewer than `{least}` players would go into `{area}`.
        needs_players[area: Line, least: Number],
        /// What a player in no party is told at an entrance that needs `{least}` players or more.
        form_party(least: Number),
        /// What an entrance tells a member of a party that already holds an instance of `{area}`.
        already_inside[area: Line],
        /// What a member's call card says once they answered: `{leader}` takes the party into `{area}`.
        call[area: Line, leader: Text],
        /// What a member's call card says while they decide: `{leader}` takes the party into `{area}`, and at most `{most}` go.
        call_open[area: Line, leader: Text, most: Number],
        /// What a leader is told as they call their party into `{area}`.
        call_sent[area: Line],
        /// What a member's call card says once they joined and `{leader}` did not pick them.
        benched(leader: Text),
        /// What a member's call card says while they stand out of `{leader}`'s sight.
        out_of_sight(leader: Text),
        /// What a member who joined and was not taken is told as the party goes into `{area}`.
        went_without[area: Line],
        /// What a member who joined and stood out of `{leader}`'s sight is told as the party goes
        /// into `{area}`.
        went_too_far[area: Line, leader: Text],
        /// What the members are told as `{leader}` calls off their call to `{area}`.
        called_off[area: Line, leader: Text],
        /// What the members are told as the call to `{area}` runs out of time.
        call_ran_out[area: Line],
        /// What the members are told as `{leader}` stops leading the party, ending the call to
        /// `{area}`.
        call_lost_lead[area: Line, leader: Text],
        /// What the members are told as `{leader}` walks away from the entrance, ending the call to
        /// `{area}`.
        call_left_door[leader: Text],
        /// What everyone inside is told as `{name}` leaves the instance of `{area}`.
        member_left[area: Line, name: Text],
        /// What a player is told coming back after the instance of `{area}` they were away from ended,
        /// now in `{place}`.
        ended_away[area: Line, place: Line],
        /// The title of the message the game sends a player whose bag had no room for what clearing
        /// an instance of `{area}` gave them.
        rewards_title(area: Text),
        /// The text of the message the game sends a player whose bag had no room for what clearing
        /// an instance of `{area}` gave them.
        rewards_text,
        /// How many players an instanced area opens with, `{count}`, where that is one number.
        players_exact(count: Number),
        /// How many players an instanced area opens with: `{least}` to `{most}`.
        players_range(least: Number, most: Number),
        /// How long an instance of an instanced area may last, `{time}`.
        time_limit(time: Text),
        /// What the entrance tells a party's leader of their party of `{size}`.
        lead_party(size: Number),
        /// Which members of the party, `{names}`, stand near the leader at the entrance, `{count}` of them.
        stand_near(count: Number, names: Text),
        /// What gathering the party means.
        gather_note,
        /// What going alone means for a party's leader.
        alone_note,
        /// The option that goes in alone while another member is inside an instance of the area.
        alone_own_instance,
        /// What the entrance says while the player may not start another instance of the area yet, for `{time}`.
        cooldown(time: Text),
        /// What the player is told when they no longer carry what starting `{area}` takes.
        items_gone[area: Line],
        /// What the player is told when what they inserted is not what starting `{area}` takes.
        wrong_items[area: Line],
        /// What stands under the name of an instanced area under the map.
        instanced,
        /// The heading over the roster of the instance the player is in.
        roster,
        /// How the player `{name}` is named in a roster.
        you_named(name: Text),
        /// Where a member of the roster stands while inside.
        inside,
        /// Where a member of the roster stands while gone, their place kept.
        away,
        /// Where a member of the roster stands once they have left for good.
        left,
        /// What the instance says once its goals are all met.
        cleared,
        /// How far a goal has come: `{done}` of `{of}`.
        goal_count(done: Number, of: Number),
        /// The button that leaves the instance.
        leave,
        /// The button that leaves a cleared instance before it closes.
        leave_now,
        /// How long until a cleared instance closes, `{time}`.
        closes_in(time: Text),
        /// What the player is asked before leaving the instance `{area}`.
        leave_title[area: Line],
        /// What the player reads before leaving the instance for good: they cannot come back and
        /// get no reward if it is cleared after, and return to `{area}`.
        leave_body[area: Line],
        /// The button that leaves the instance for good.
        leave_for_good,
        /// The button that stays in the instance.
        stay,
        /// The heading of what the player is told as the instance `{area}` is cleared.
        cleared_title[area: Line],
        /// What clearing the instance gave the player, `{items}`.
        yours[items: Line],
        /// Experience `{count}` that clearing the instance gave.
        experience(count: Number),
        /// When a cleared instance sends the player back to `{area}`.
        return_when[area: Line],
        /// The button that stays in a cleared instance until it closes.
        stay_till,
        /// The title of the window a leader picks who comes along in, for `{area}`.
        roster_title(area: Text),
        /// The button that takes the picked members in.
        enter,
        /// How many of the instance's places are taken, `{taken}` of `{most}`.
        slots(most: Number, taken: Number),
        /// How many of the instance's places are taken, `{taken}` of `{most}`, once all are.
        slots_full(most: Number, taken: Number),
        /// How many players the instance needs, `{least}`, beside how many are ready, `{count}`.
        needs_ready(count: Number, least: Number),
        /// The heading of the card that calls a member to an instance.
        call_heading,
        /// The button that declines a call.
        not_this_time,
        /// The button that takes back joining a call.
        withdraw,
        /// The button that joins a call.
        join,
        /// What a member may do while the call is open.
        change_answer,
        /// The button that picks a member to come along.
        pick,
        /// The button that takes a member off the ones coming along.
        unpick,
        /// Where a member `{name}` who is too far plays, `{area}`.
        in_area(area: Text, name: Text),
        /// The leader's place in the roster window.
        going_leader,
        /// A member who is picked to come along.
        going,
        /// The member's own card, once they are picked.
        you_go,
        /// A member who joined but is not picked.
        on_bench,
        /// A member who has not answered yet.
        deciding,
        /// A member who declined.
        not_going,
        /// A member who stands too far from the leader.
        too_far,
        /// A member whose player is gone.
        away_from_game,
        /// A member who has fallen.
        fallen,
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<LeaveInstance>();
    entrance::register(app);
    roster::register(app);
}

/// What makes an area one that is only ever entered as an instance.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaInstanceRules {
    /// How many players an instance opens with.
    pub players: PlayerCount,
    /// What starting an instance takes from the bag of the player who starts it.
    pub entry: InstanceEntry,
    /// How long after starting an instance the player who started it waits to start another.
    pub cooldown: Seconds,
    /// What clears one, once every one of them is met.
    pub goals: Vec<AreaInstanceGoal>,
    /// How long one with nobody inside waits before it is gone.
    pub empties_after: Seconds,
    /// How long one stays open once it is cleared.
    pub closes_after: Seconds,
    /// How long one may live uncleared. Without it, for as long as someone comes back to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lasts: Option<Seconds>,
}

/// How many players an instance opens with: no fewer than `least`, no more than `most`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerCount {
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub least: u32,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub most: u32,
}

/// What starting an instance of an instanced area takes: the items the player who starts it inserts at
/// the entrance, taken from their bag only as the instance opens, and what the entrance says of what
/// is inserted.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceEntry {
    /// Each item it takes, once, with how many of it.
    pub needs: Vec<ItemStack>,
    /// What the entrance says while nothing is inserted, as an ICU message that takes no argument. It may hint at what it takes.
    pub empty: Phrase,
    /// What it says while what is inserted is not what it takes, as an ICU message: `{items}` is what is inserted.
    pub wrong: Phrase,
    /// What it says once what it takes is inserted, and starting is open, as an ICU message that takes no argument.
    pub right: Phrase,
}

impl InstanceEntry {
    /// Whether `offer` is what it takes: every item it needs, as many of each, and nothing else.
    pub fn takes(&self, offer: &[ItemStack]) -> bool {
        let mut summed: Vec<ItemStack> = Vec::new();
        for stack in offer {
            match summed.iter_mut().find(|held| held.item == stack.item) {
                Some(held) => held.count += stack.count,
                None => summed.push(*stack),
            }
        }
        summed.len() == self.needs.len() && self.needs.iter().all(|need| summed.contains(need))
    }

    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.at_least(format!("{pointer}/needs"), self.needs.len(), 1, "items");
        for (index, need) in self.needs.iter().enumerate() {
            cx.at_least(
                format!("{pointer}/needs/{index}/count"),
                need.count,
                1,
                "items",
            );
            if self.needs[..index]
                .iter()
                .any(|other| other.item == need.item)
            {
                cx.error(
                    format!("{pointer}/needs/{index}"),
                    "listed twice",
                    "the same item on two lines",
                    "each item once, with how many of it",
                );
            }
        }
        self.empty.check(&format!("{pointer}/empty"), &[], cx);
        self.wrong.check(
            &format!("{pointer}/wrong"),
            &[("items", <Line as ArgumentKindOf>::USES)],
            cx,
        );
        self.right.check(&format!("{pointer}/right"), &[], cx);
    }
}

/// When each instanced area a player started may be started by them again.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct InstanceCooldowns(pub BTreeMap<AreaId, UnixMillis>);

impl InstanceCooldowns {
    /// How long until `area` may be started again: nothing once it may.
    pub fn left(&self, area: AreaId, now: UnixMillis) -> Option<Seconds> {
        self.0
            .get(&area)
            .filter(|&&ready| ready > now)
            .map(|ready| ready.since(now))
    }

    /// Starts the cooldown of `area`, started at `now`, forgetting those that ran out.
    pub fn start(&mut self, area: AreaId, now: UnixMillis, cooldown: Seconds) {
        self.0.retain(|_, ready| *ready > now);
        self.0.insert(area, now.after(cooldown));
    }
}

/// What a script completes to count toward a task goal of the instance its world is.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoalTask(pub String);

/// One thing an instance asks of those inside, counted for all of them together.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AreaInstanceGoal {
    /// `count` of `npc` fall inside, whoever strikes the blow.
    Defeat {
        npc: NpcId,
        #[schemars(transform = SchemaMark::Count.with(true))]
        #[schemars(extend("default" = serial::one()))]
        #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
        count: u32,
    },
    /// A script completes `task`, which those inside read as `label`.
    Task { task: GoalTask, label: String },
}

impl AreaInstanceGoal {
    /// How many times it is counted before it is met.
    pub fn count(&self) -> u32 {
        match self {
            AreaInstanceGoal::Defeat { count, .. } => *count,
            AreaInstanceGoal::Task { .. } => 1,
        }
    }

    /// What those inside read it as.
    pub fn label(&self, content: &Content) -> String {
        match self {
            AreaInstanceGoal::Defeat { npc, .. } => {
                crate::systems::npc::defeat_label(content, *npc)
            }
            AreaInstanceGoal::Task { label, .. } => label.clone(),
        }
    }
}

impl AreaInstanceRules {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        let players = self.players;
        cx.at_least(
            format!("{pointer}/players/least"),
            players.least,
            1,
            "players",
        );
        cx.at_least(
            format!("{pointer}/players/most"),
            players.most,
            players.least.max(1),
            "players",
        );
        cx.at_least(format!("{pointer}/goals"), self.goals.len(), 1, "goals");
        cx.at_least(
            format!("{pointer}/cooldown"),
            self.cooldown.0,
            0.0,
            "seconds",
        );
        self.entry.check(&format!("{pointer}/entry"), cx);
        cx.at_least(
            format!("{pointer}/empties_after"),
            self.empties_after.0,
            0.0,
            "seconds",
        );
        cx.at_least(
            format!("{pointer}/closes_after"),
            self.closes_after.0,
            0.0,
            "seconds",
        );
        if let Some(lasts) = self.lasts {
            cx.at_least(format!("{pointer}/lasts"), lasts.0, 1.0, "seconds");
        }
        let mut tasks: Vec<&GoalTask> = Vec::new();
        for (index, goal) in self.goals.iter().enumerate() {
            let at = format!("{pointer}/goals/{index}");
            match goal {
                AreaInstanceGoal::Defeat { count, .. } => {
                    cx.at_least(format!("{at}/count"), *count, 1, "npcs");
                }
                AreaInstanceGoal::Task { task, label } => {
                    cx.text(format!("{at}/task"), &task.0, 1, 40);
                    cx.text(format!("{at}/label"), label, 1, 60);
                    if tasks.contains(&task) {
                        cx.error(
                            format!("{at}/task"),
                            "task named twice",
                            task.0.clone(),
                            "each task of an instance under a name of its own",
                        );
                    }
                    tasks.push(task);
                }
            }
        }
    }
}

/// Which instance, of every one the server has opened since it started.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct AreaInstanceId(pub u32);

/// Where a member of an instance's roster stands with it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RosterPlace {
    Inside,
    /// Their player is gone; their place is kept until the instance ends.
    Away,
    /// They left, and can't come back.
    Left,
}

impl RosterPlace {
    /// Whether the member may still be inside: who left may not.
    pub fn holds(self) -> bool {
        self != RosterPlace::Left
    }
}

/// An instance of an area, as society holds it while it lives. Its roster is fixed when it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaInstance {
    pub id: AreaInstanceId,
    pub area: AreaId,
    /// What clearing it gives, as whoever opened it was let in with.
    pub rewards: Reward,
    /// Where its roster appears, and rises after a fall.
    pub entrance: Place,
    /// Where leaving it leads, as whoever opened it was let in with.
    pub return_to: Place,
    /// Everyone it opened for, in the order they were taken.
    roster: Vec<RosterMember>,
    opened: UnixMillis,
    /// How far each of the area's goals has come, in their order.
    progress: Vec<u32>,
    /// Since when nobody has been inside.
    empty_since: Option<UnixMillis>,
    cleared: Option<AreaInstanceClearing>,
    /// Whether it has ended, and everyone inside is on their way out.
    over: bool,
}

/// One of the characters an instance opened for.
#[derive(Clone, Debug, PartialEq)]
pub struct RosterMember {
    pub account: AccountId,
    pub place: RosterPlace,
    /// Where they last stood inside, once they have been inside.
    stood: Option<Pos<Meters>>,
}

impl AreaInstance {
    /// Its roster, in the order they were taken.
    pub fn roster(&self) -> impl Iterator<Item = &RosterMember> {
        self.roster.iter()
    }

    /// Where `account` stands with it, if it opened for them.
    pub fn place_of(&self, account: &AccountId) -> Option<RosterPlace> {
        self.member(account).map(|member| member.place)
    }

    fn member(&self, account: &AccountId) -> Option<&RosterMember> {
        self.roster.iter().find(|member| member.account == *account)
    }

    /// The members who may still be inside: everyone but who left.
    fn holding(&self) -> impl Iterator<Item = &AccountId> {
        self.roster
            .iter()
            .filter(|member| member.place.holds())
            .map(|member| &member.account)
    }

    /// Whether `account` may be inside it.
    fn holds(&self, account: &AccountId) -> bool {
        self.place_of(account).is_some_and(RosterPlace::holds)
    }
}

/// An instance cleared: when, and what each participant got of it.
#[derive(Clone, Debug, PartialEq)]
struct AreaInstanceClearing {
    at: UnixMillis,
    given: BTreeMap<AccountId, AreaInstanceGiven>,
}

/// What clearing an instance gave one participant.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct AreaInstanceGiven {
    pub xp: u32,
    /// The stacks that went into their bag.
    pub kept: Vec<ItemStack>,
    /// The stacks their bag had no room for.
    pub sent: Vec<ItemStack>,
}

/// Every instance the server runs, and the calls of leaders gathering their party for one.
#[derive(Debug, Default)]
pub struct AreaInstances {
    live: BTreeMap<AreaInstanceId, AreaInstance>,
    pub(crate) calls: roster::InstanceCalls,
    /// Who was away when an instance they were on the roster of ended, and which area it was, to tell
    /// them when they are back.
    ended_away: BTreeMap<AccountId, AreaId>,
    opened: u32,
    /// How many the server runs at once.
    most: u32,
}

impl AreaInstances {
    pub fn get(&self, id: AreaInstanceId) -> Option<&AreaInstance> {
        self.live.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &AreaInstance> {
        self.live.values()
    }

    /// The instance `account` is on the roster of and has not left.
    pub fn around(&self, account: &AccountId) -> Option<&AreaInstance> {
        self.live.values().find(|instance| instance.holds(account))
    }

    pub fn calls(&self) -> &roster::InstanceCalls {
        &self.calls
    }

    /// The instance `account` was away from and may come back into while it lives, with the world
    /// it is and the spot they last stood on inside.
    pub fn rejoin(&self, account: &AccountId) -> Option<(AreaInstanceWorld, Place)> {
        let (instance, member) = self.live.values().find_map(|instance| {
            let member = instance.member(account)?;
            (!instance.over && member.place == RosterPlace::Away).then_some((instance, member))
        })?;
        let at = member.stood.unwrap_or(instance.entrance.at);
        let place = Place {
            area: instance.area,
            at,
        };
        Some((AreaInstanceWorld::of(instance), place))
    }

    /// Whether another member of the party `account` is in holds a place in an instance of `area`
    /// that is still open.
    pub fn party_inside(
        &self,
        parties: &CharacterParties,
        account: &AccountId,
        area: AreaId,
    ) -> bool {
        let Some(party) = parties.of_account(account) else {
            return false;
        };
        self.live
            .values()
            .filter(|instance| instance.area == area && !instance.over)
            .any(|instance| party.others(account).any(|member| instance.holds(member)))
    }

    /// Lets the server run `most` at once.
    pub(crate) fn run_at_most(&mut self, most: u32) {
        self.most = most;
    }
}

/// What a script asks society for as it lets a player into an instance.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaInstanceEntry {
    /// The instanced area, and where in it entrants appear.
    pub to: Place,
    pub rewards: Reward,
    pub return_to: Place,
    /// The world the script that asked runs in, and its flow that waits to hear whether they
    /// were let in.
    pub asked_in: WorldName,
    pub flow: FlowKey,
    /// Whether the player is told what refused them, the script having nothing to say of it.
    pub told: bool,
}

/// On the world of an instance: which it is, where its roster appears, and where leaving it
/// leads.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct AreaInstanceWorld {
    pub id: AreaInstanceId,
    pub entrance: Place,
    pub return_to: Place,
}

impl AreaInstanceWorld {
    pub fn of(instance: &AreaInstance) -> AreaInstanceWorld {
        AreaInstanceWorld {
            id: instance.id,
            entrance: instance.entrance,
            return_to: instance.return_to,
        }
    }
}

/// What happened in an instance's world toward its goals since the worlds last conferred.
#[derive(Resource, Default)]
pub struct AreaInstanceTally {
    defeated: Vec<NpcId>,
    done: Vec<GoalTask>,
}

/// A player asks to leave the instance they are inside, for good.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaveInstance;

/// The instance a player is inside, as their client shows it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaInstanceShown {
    pub area: AreaId,
    pub goals: Vec<AreaInstanceGoalShown>,
    /// Its roster, in the order they were taken, with where each stands with it.
    pub roster: Vec<RosterRowInside>,
    /// When it ends uncleared, in an area whose instances only last so long.
    pub ends: Option<UnixMillis>,
    /// The area leaving it leads to.
    pub returns_to: AreaId,
    /// Once it is cleared: what that gave the player, and when everyone is returned.
    pub cleared: Option<AreaInstanceClearedShown>,
}

/// A member of an instance's roster, as the plate lists them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RosterRowInside {
    pub character: SocietyMember,
    pub place: RosterPlace,
    /// Whether it is the player the plate is shown to.
    pub me: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct AreaInstanceGoalShown {
    pub label: String,
    pub done: u32,
    pub of: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaInstanceClearedShown {
    pub given: AreaInstanceGiven,
    pub closes: UnixMillis,
}

impl AreaInstanceShown {
    /// The instance `account` is inside, as `society` holds it.
    pub(crate) fn of(
        society: &Society,
        account: &AccountId,
        content: &Content,
    ) -> Option<AreaInstanceShown> {
        let instance = society.instances().around(account)?;
        let rules = instance.area.get(content).instance.as_ref()?;
        let goals = rules
            .goals
            .iter()
            .zip(&instance.progress)
            .map(|(goal, done)| AreaInstanceGoalShown {
                label: goal.label(content),
                done: *done,
                of: goal.count(),
            })
            .collect();
        let cleared = instance
            .cleared
            .as_ref()
            .map(|clearing| AreaInstanceClearedShown {
                given: clearing.given.get(account).cloned().unwrap_or_default(),
                closes: clearing.at.after(rules.closes_after),
            });
        let roster = instance
            .roster()
            .filter_map(|member| {
                Some(RosterRowInside {
                    character: society.member(&member.account)?,
                    place: member.place,
                    me: member.account == *account,
                })
            })
            .collect();
        Some(AreaInstanceShown {
            area: instance.area,
            goals,
            roster,
            ends: match cleared {
                Some(_) => None,
                None => rules.lasts.map(|lasts| instance.opened.after(lasts)),
            },
            returns_to: instance.return_to.area,
            cleared,
        })
    }
}

/// Whether this world is an instance.
pub fn inside(world: &World) -> bool {
    world.contains_resource::<AreaInstanceWorld>()
}

/// Counts every npc that fell in an instance's world toward the instance's goals.
pub fn count_defeats(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let fallen: Vec<NpcId> = deaths
        .read(world.resource::<Messages<Died>>())
        .filter_map(|died| world.get::<Npc>(died.entity).map(|npc| npc.def))
        .collect();
    if world.contains_resource::<AreaInstanceWorld>() {
        world
            .resource_mut::<AreaInstanceTally>()
            .defeated
            .extend(fallen);
    }
}

/// Counts `task` as done toward the goals of the instance this world is, if it is one.
pub fn complete(world: &mut World, task: GoalTask) {
    if world.contains_resource::<AreaInstanceWorld>() {
        world.resource_mut::<AreaInstanceTally>().done.push(task);
    }
}

pub fn requests(world: &mut World) {
    player::player_requests::<LeaveInstance>(world, RequestNeeds::Nothing, |world, player, _| {
        leave(world, player);
        Ok(())
    });
}

/// Sends `character` out of the instance this world is, to where leaving it leads. Whoever
/// leaves an instance that is still open has left it for good.
pub fn leave(world: &mut World, character: Entity) {
    let Some(run) = world.get_resource::<AreaInstanceWorld>().copied() else {
        return;
    };
    if world.get::<Crossing>(character).is_none() {
        area::warp::cross(world, character, run.return_to, None);
    }
}

/// Notes where every instance's roster stands as the roster of players now lists them: a member
/// inside is inside, one whose player is gone is away with their place kept, and one who has
/// been inside and now plays elsewhere has left. An instance nobody is inside counts its time from
/// here, and whoever comes back after an instance they were away from ended is told so.
pub(crate) fn look(settling: &mut Settling, content: &Content) {
    let listed = watched(settling);
    let now = settling.now;
    let mut changed: Vec<AccountId> = Vec::new();
    let mut left: Vec<(AccountId, AreaId, Vec<AccountId>)> = Vec::new();
    for instance in settling.society.instances.live.values_mut() {
        let before: Vec<RosterPlace> = instance.roster.iter().map(|member| member.place).collect();
        let mut anyone_inside = false;
        for member in &mut instance.roster {
            if member.place == RosterPlace::Left {
                continue;
            }
            match listed.get(&member.account) {
                Some(&(Some(id), away, at)) if id == instance.id => {
                    anyone_inside = true;
                    member.stood = Some(at);
                    member.place = if away {
                        RosterPlace::Away
                    } else {
                        RosterPlace::Inside
                    };
                }
                Some(_) if member.stood.is_some() && !instance.over => {
                    member.place = RosterPlace::Left;
                }
                Some(_) => {}
                None => member.place = RosterPlace::Away,
            }
        }
        let now_is: Vec<RosterPlace> = instance.roster.iter().map(|member| member.place).collect();
        for (index, (was, is)) in before.iter().zip(&now_is).enumerate() {
            if was != is && *is == RosterPlace::Left {
                let others = instance
                    .roster
                    .iter()
                    .filter(|other| other.place == RosterPlace::Inside)
                    .map(|other| other.account.clone())
                    .collect();
                left.push((
                    instance.roster[index].account.clone(),
                    instance.area,
                    others,
                ));
            }
        }
        if before != now_is {
            changed.extend(instance.roster.iter().map(|member| member.account.clone()));
        }
        instance.empty_since = if anyone_inside {
            None
        } else {
            instance.empty_since.or(Some(now))
        };
    }
    for account in changed {
        settling.society.changed(&account);
    }
    let messages = &messages::of(content).instance;
    for (account, area, others) in left {
        let said = messages.member_left(Mention::Area(area).span(content), settling.name(&account));
        for other in others {
            settling.tell(
                &other,
                Notification::feed(HistoryTopic::Notification, said.clone()),
            );
        }
    }
    let back: Vec<(AccountId, AreaId)> = settling
        .society
        .instances
        .ended_away
        .iter()
        .filter(|(account, _)| listed.get(*account).is_some_and(|(_, away, _)| !away))
        .map(|(account, area)| (account.clone(), *area))
        .collect();
    for (account, area) in back {
        settling.society.instances.ended_away.remove(&account);
        let place = settling
            .reach(&account)
            .and_then(|(world, _)| world.get_resource::<crate::systems::WorldArea>().copied())
            .map(|here| Mention::Area(here.0).span(content));
        let said = messages.ended_away(
            Mention::Area(area).span(content),
            place.map(LineText::from).unwrap_or_default(),
        );
        settling.tell(
            &account,
            Notification::feed(HistoryTopic::Notification, said),
        );
    }
}

/// Where the roster lists everyone whose place `look` notes: whoever an instance holds a place for,
/// and whoever is told on coming back that an instance ended. Each is looked up on their own, so this
/// costs what the instances hold, however many play.
fn watched(
    settling: &Settling,
) -> BTreeMap<AccountId, (Option<AreaInstanceId>, bool, Pos<Meters>)> {
    let instances = &settling.society.instances;
    let held = instances
        .live
        .values()
        .flat_map(|instance| &instance.roster)
        .filter(|member| member.place != RosterPlace::Left)
        .map(|member| &member.account)
        .chain(instances.ended_away.keys());
    let roster = settling.roster;
    held.filter_map(|account| {
        let enrolled = roster.account(account)?;
        let at = roster.place_of(enrolled.client)?.at;
        Some((account.clone(), (enrolled.instance, enrolled.away, at)))
    })
    .collect()
}

/// Lets `by` into an instance of the entry's area of their own, and tells the flow that asked how it
/// went. Whoever already stands in an instance of that area moves within it instead.
pub(crate) fn go_alone(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    entry: AreaInstanceEntry,
) {
    let here = settling
        .reach(by)
        .and_then(|(world, _)| WorldName::of(world));
    if here != Some(entry.asked_in) {
        answer(settling, &entry, REFUSED);
        return;
    }
    let within = settling
        .society
        .instances
        .around(by)
        .filter(|instance| instance.area == entry.to.area && !instance.over)
        .map(AreaInstanceWorld::of);
    let admitted = match within {
        Some(run) => Ok(run),
        None => alone(content, &entry)
            .and_then(|()| open(settling, content, &entry, by, vec![by.clone()])),
    };
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let output = match admitted {
        Ok(run) => {
            step_in(world, character, entry.to, run);
            NEXT
        }
        Err(why) => {
            if entry.told {
                notification::error(world, character, why);
            }
            REFUSED
        }
    };
    answer(settling, &entry, output);
}

/// Answers the flow that asked for `entry`, in the world it runs in: never in another, whose
/// flows its key does not name.
pub(crate) fn answer(settling: &mut Settling, entry: &AreaInstanceEntry, output: &'static str) {
    if let Some(world) = settling.world(entry.asked_in) {
        world
            .resource_mut::<Instances>()
            .answer(entry.flow, Answer::Output(output));
    }
}

/// Whether the entry's area may be done alone, or why not, as the game words it.
fn alone(content: &Content, entry: &AreaInstanceEntry) -> Result<(), LineText> {
    let messages = &messages::of(content).instance;
    let area = entry.to.area;
    let least = area
        .get(content)
        .instance
        .as_ref()
        .map_or(1, |rules| rules.players.least);
    if least <= 1 {
        return Ok(());
    }
    Err(messages.needs_players(Mention::Area(area).span(content), least))
}

/// What every instance comes to as the worlds confer: its goals counted, its rewards handed out
/// once they are all met, its time kept, everyone sent out once it is over, and one that is done
/// with gone.
pub(crate) fn keep(settling: &mut Settling, content: &Content) {
    if settling.society.instances.live.is_empty() {
        return;
    }
    tally(settling, content);
    let ids: Vec<AreaInstanceId> = settling.society.instances.live.keys().copied().collect();
    for id in ids {
        clear(settling, content, id);
        time(settling, content, id);
        send_out(settling, id);
    }
    let now = settling.now;
    let instances = &mut settling.society.instances;
    let mut gone: Vec<AreaInstanceId> = Vec::new();
    for instance in instances.live.values() {
        let Some(since) = instance.empty_since else {
            continue;
        };
        let rules = instance.area.get(content).instance.as_ref();
        let waited = rules.is_none_or(|rules| now.since(since) >= rules.empties_after);
        let everyone_left = instance.holding().next().is_none();
        if instance.over || everyone_left || waited {
            gone.push(instance.id);
        }
    }
    for id in gone {
        let Some(instance) = instances.live.remove(&id) else {
            continue;
        };
        for member in &instance.roster {
            if member.place == RosterPlace::Away {
                instances
                    .ended_away
                    .insert(member.account.clone(), instance.area);
            }
        }
    }
}

/// Nobody starts inside an instanced area.
pub fn check_all(loading: &mut Loading, content: &Content, settings: &GameSettings) {
    let instanced = |place: Place| place.area.get(content).instance.is_some();
    let starts_inside = |place: Place| {
        ContentProblem::new(
            "starts in an instance",
            place.area.path(content),
            "a start in an area everyone shares",
        )
    };
    if instanced(settings.initial_spawn) {
        let problem = starts_inside(settings.initial_spawn);
        loading.json_error(SETTINGS_FILE, "/initial_spawn", problem);
    }
    for (character, snapshot) in loading.rows::<CharacterSnapshot>(content) {
        let start = snapshot::start_of(content, snapshot);
        if instanced(start) {
            loading.json_error(character.path(content), "/start", starts_inside(start));
        }
    }
}

/// Opens an instance of the entry's area for `roster`, unless the server runs as many as it may: the
/// world it is, or why not, as the game words it.
fn open(
    settling: &mut Settling,
    content: &Content,
    entry: &AreaInstanceEntry,
    by: &AccountId,
    roster: Vec<AccountId>,
) -> Result<AreaInstanceWorld, LineText> {
    let messages = &messages::of(content).instance;
    let area = entry.to.area;
    let Some(rules) = &area.get(content).instance else {
        return Err(messages.busy().into());
    };
    let instances = &settling.society.instances;
    let worlds = content.ids::<AreaDef>().count() as u64 + u64::from(instances.opened);
    if instances.live.len() as u32 >= instances.most || worlds >= identity::MOST_WORLDS {
        return Err(messages.busy().into());
    }
    let now = settling.now;
    let Some((world, starter)) = settling.reach(by) else {
        return Err(messages.busy().into());
    };
    start(world, content, starter, rules, area, now)?;
    let instances = &mut settling.society.instances;
    let id = AreaInstanceId(instances.opened);
    instances.opened += 1;
    let instance = AreaInstance {
        id,
        area,
        rewards: entry.rewards.clone(),
        entrance: entry.to,
        return_to: entry.return_to,
        roster: roster
            .iter()
            .map(|account| RosterMember {
                account: account.clone(),
                place: RosterPlace::Inside,
                stood: None,
            })
            .collect(),
        opened: settling.now,
        progress: vec![0; rules.goals.len()],
        empty_since: None,
        cleared: None,
        over: false,
    };
    let run = AreaInstanceWorld::of(&instance);
    let mut moved_on: Vec<AccountId> = Vec::new();
    for other in instances.live.values_mut() {
        let mut moves = false;
        for member in &mut other.roster {
            if member.place.holds() && roster.contains(&member.account) {
                member.place = RosterPlace::Left;
                moves = true;
            }
        }
        if moves {
            moved_on.extend(other.roster.iter().map(|member| member.account.clone()));
        }
    }
    instances.live.insert(id, instance);
    for account in &roster {
        instances.ended_away.remove(account);
    }
    for account in roster.iter().chain(&moved_on) {
        settling.society.changed(account);
    }
    Ok(run)
}

/// Has `starter` start an instance of `area` at `now`, unless they wait on its cooldown: what it takes
/// is taken from their bag, for good, and the cooldown begins.
fn start(
    world: &mut World,
    content: &Content,
    starter: Entity,
    rules: &AreaInstanceRules,
    area: AreaId,
    now: UnixMillis,
) -> Result<(), LineText> {
    let messages = messages::of(content);
    if let Some(left) = world
        .get::<InstanceCooldowns>(starter)
        .and_then(|cooldowns| cooldowns.left(area, now))
    {
        let time = messages.time.exact(left);
        return Err(messages.instance.cooldown(time).into());
    }
    let mut bag = world.get_mut::<Inventory>(starter);
    let taken = bag
        .as_mut()
        .is_some_and(|bag| bag.exchange(content, &rules.entry.needs, &[]).is_ok());
    if !taken {
        return Err(messages
            .instance
            .items_gone(Mention::Area(area).span(content)));
    }
    let mut cooldowns = world
        .get::<InstanceCooldowns>(starter)
        .cloned()
        .unwrap_or_default();
    cooldowns.start(area, now, rules.cooldown);
    world.entity_mut(starter).insert(cooldowns);
    Ok(())
}

/// Has `character` step through into the instance `run`, at `to`: mounted only where their
/// mount may be.
fn step_in(world: &mut World, character: Entity, to: Place, run: AreaInstanceWorld) {
    let here = world
        .get_resource::<AreaInstanceWorld>()
        .map(|here| here.id);
    if here == Some(run.id) {
        mount::warping(world, character, to);
        world
            .entity_mut(character)
            .insert(player::SpawnPoint(to.at));
        area::warp::land(world, character, to.at);
    } else {
        area::warp::cross(world, character, to, Some(run));
    }
}

/// Counts what every instance's world noted toward its goals since the worlds last conferred.
fn tally(settling: &mut Settling, content: &Content) {
    let mut noted: Vec<(AreaInstanceId, AreaInstanceTally)> = Vec::new();
    for world in settling.views.iter_mut().flatten() {
        let Some(run) = world.get_resource::<AreaInstanceWorld>().copied() else {
            continue;
        };
        let tally = std::mem::take(&mut *world.resource_mut::<AreaInstanceTally>());
        if !tally.defeated.is_empty() || !tally.done.is_empty() {
            noted.push((run.id, tally));
        }
    }
    let mut changed: Vec<AccountId> = Vec::new();
    for (id, tally) in noted {
        let Some(instance) = settling.society.instances.live.get_mut(&id) else {
            continue;
        };
        let Some(rules) = &instance.area.get(content).instance else {
            continue;
        };
        if instance.cleared.is_some() || instance.over {
            continue;
        }
        let before = instance.progress.clone();
        for (goal, done) in rules.goals.iter().zip(&mut instance.progress) {
            match goal {
                AreaInstanceGoal::Defeat { npc, count } => {
                    let fell = tally.defeated.iter().filter(|fell| *fell == npc).count();
                    *done = done.saturating_add(fell as u32).min(*count);
                }
                AreaInstanceGoal::Task { task, .. } if tally.done.contains(task) => *done = 1,
                AreaInstanceGoal::Task { .. } => {}
            }
        }
        if instance.progress != before {
            changed.extend(instance.roster.iter().map(|member| member.account.clone()));
        }
    }
    for account in changed {
        settling.society.changed(&account);
    }
}

/// Clears the instance `id` once every goal is met, once: each member of its roster who has not
/// left gets the experience whole and items rolled for them alone, delivered as every delivery is.
fn clear(settling: &mut Settling, content: &Content, id: AreaInstanceId) {
    let Some(instance) = settling.society.instances.get(id).cloned() else {
        return;
    };
    let def = instance.area.get(content);
    let Some(rules) = &def.instance else {
        return;
    };
    let met = rules
        .goals
        .iter()
        .zip(&instance.progress)
        .all(|(goal, done)| *done >= goal.count());
    if !met || instance.cleared.is_some() || instance.over {
        return;
    }
    let participants: Vec<AccountId> = instance.holding().cloned().collect();
    let Some(run) = settling
        .views
        .iter_mut()
        .flatten()
        .find(|world| world.get_resource::<AreaInstanceWorld>().map(|run| run.id) == Some(id))
    else {
        return;
    };
    let rolled: Vec<(AccountId, Rolled)> = run.resource_scope(|_, mut rng: Mut<Rng>| {
        participants
            .into_iter()
            .map(|account| (account, instance.rewards.roll(&mut rng)))
            .collect()
    });
    let mut given: BTreeMap<AccountId, AreaInstanceGiven> = BTreeMap::new();
    for (account, rolled) in rolled {
        let reason = DeliveryReason::InstanceCleared(instance.area);
        let granted = reward::grant_account(settling, content, &account, rolled, reason);
        let gave = AreaInstanceGiven {
            xp: granted.xp,
            kept: granted.delivered.kept,
            sent: granted.delivered.sent,
        };
        settling.society.changed(&account);
        given.insert(account, gave);
    }
    if let Some(instance) = settling.society.instances.live.get_mut(&id) {
        instance.cleared = Some(AreaInstanceClearing {
            at: settling.now,
            given,
        });
    }
}

/// Keeps the time of the instance `id`: one cleared closes when the area says, and one that only
/// lasts so long ends unrewarded, everyone inside told.
fn time(settling: &mut Settling, content: &Content, id: AreaInstanceId) {
    let messages = &messages::of(content).instance;
    let now = settling.now;
    let Some(instance) = settling.society.instances.live.get_mut(&id) else {
        return;
    };
    let def = instance.area.get(content);
    let Some(rules) = &def.instance else {
        return;
    };
    if instance.over {
        return;
    }
    let closed = instance
        .cleared
        .as_ref()
        .is_some_and(|clearing| now >= clearing.at.after(rules.closes_after));
    let ran_out = instance.cleared.is_none()
        && rules
            .lasts
            .is_some_and(|lasts| now >= instance.opened.after(lasts));
    instance.over = closed || ran_out;
    if !ran_out {
        return;
    }
    let said = messages.timed_out(Mention::Area(instance.area).span(content));
    let inside: Vec<AccountId> = instance
        .roster()
        .filter(|member| member.place == RosterPlace::Inside)
        .map(|member| member.account.clone())
        .collect();
    for account in inside {
        settling.tell(
            &account,
            Notification::feed(HistoryTopic::Notification, said.clone()),
        );
    }
}

/// Sends everyone out of the instance `id` once it is over.
fn send_out(settling: &mut Settling, id: AreaInstanceId) {
    let Some(instance) = settling.society.instances.get(id) else {
        return;
    };
    if !instance.over {
        return;
    }
    let inside: Vec<AccountId> = instance.holding().cloned().collect();
    for account in inside {
        if let Some((world, character)) = settling.reach(&account)
            && world.get_resource::<AreaInstanceWorld>().map(|run| run.id) == Some(id)
        {
            leave(world, character);
        }
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .effect::<effect::EnterInstance>("enter_instance")
        .effect::<effect::LeaveInstance>("leave_instance")
        .effect::<effect::CompleteGoal>("complete_goal");
}
