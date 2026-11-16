//! Instanced areas: an area that is never shared. Whoever enters gets a copy of it, a world of
//! its own, with goals to meet and rewards for meeting them. A copy opens for a roster that is
//! fixed then: a player alone, or the members a party's leader gathered. Nobody joins it later,
//! and whoever leaves it is gone from it for good. What the rewards are and where the roster
//! comes back to are decided by whatever let them in. Every copy is society's: it opens them,
//! counts their goals and their time, hands out their rewards and says when one is gone.

pub mod effect;
pub mod entrance;
pub mod gather;
pub mod plate;
pub mod roster;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentProblem, Loading, RowCheck, SchemaMark, serial};
use crate::core::math::{Pos, Rng};
use crate::core::script::{Answer, NEXT, REFUSED};
use crate::core::time::UnixMillis;
use crate::core::units::{Meters, Seconds};
use crate::systems::account::AccountId;
use crate::systems::area::transition::Crossing;
use crate::systems::area::{self, AreaDef, AreaId};
use crate::systems::combat::Died;
use crate::systems::history::HistoryTopic;
use crate::systems::item::{DropRow, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::mount;
use crate::systems::notification::{self, Notification};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party::PlayerParties;
use crate::systems::payout::{self, PayoutReason};
use crate::systems::player::snapshot::{self, CharacterSnapshot};
use crate::systems::player::{self, RequestNeeds};
use crate::systems::script::{FlowKey, Place, RiftInstances};
use crate::systems::society::{Settling, Society, SocietyMember};
use crate::systems::text::LineText;
use crate::systems::worlds::WorldName;
use crate::systems::{GameSettings, SETTINGS_FILE, class, identity};

pub use roster::{CallReply, CallRequest, InstanceAsk};

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
    /// How many players a copy opens with.
    pub players: PlayerCount,
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

/// How many players a copy opens with: no fewer than `least`, no more than `most`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerCount {
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub least: u32,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub most: u32,
}

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
    Task { task: String, label: String },
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
        let mut tasks: Vec<&str> = Vec::new();
        for (index, goal) in self.goals.iter().enumerate() {
            let at = format!("{pointer}/goals/{index}");
            match goal {
                AreaInstanceGoal::Defeat { count, .. } => {
                    cx.at_least(format!("{at}/count"), *count, 1, "npcs");
                }
                AreaInstanceGoal::Task { task, label } => {
                    cx.text(format!("{at}/task"), task, 1, 40);
                    cx.text(format!("{at}/label"), label, 1, 60);
                    if tasks.contains(&task.as_str()) {
                        cx.error(
                            format!("{at}/task"),
                            "task named twice",
                            task.clone(),
                            "each task of an instance under a name of its own",
                        );
                    }
                    tasks.push(task);
                }
            }
        }
    }
}

/// What each participant of an instance gets as it is cleared: the experience whole, and the
/// items rolled for each of them.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaInstanceRewards {
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DropRow>,
}

impl AreaInstanceRewards {
    /// What the items come to for one participant this time, an item to a stack.
    fn rolled(&self, rng: &mut Rng) -> Vec<ItemStack> {
        let mut rolled: Vec<ItemStack> = Vec::new();
        for stack in self.items.iter().filter_map(|row| row.rolled(rng)) {
            match rolled.iter_mut().find(|held| held.item == stack.item) {
                Some(held) => held.count += stack.count,
                None => rolled.push(stack),
            }
        }
        rolled
    }
}

/// Which instance, of every one the server has opened since it started.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct AreaInstanceId(pub u32);

/// Where a member of a copy's roster stands with it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RosterPlace {
    Inside,
    /// Their player is gone; their place is kept until the copy ends.
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
    pub rewards: AreaInstanceRewards,
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

/// One of the players a copy opened for.
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

    pub fn is_cleared(&self) -> bool {
        self.cleared.is_some()
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
    /// Who was away when a copy they were on the roster of ended, and which area it was, to tell
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

    /// The copy `account` is on the roster of and has not left.
    pub fn around(&self, account: &AccountId) -> Option<&AreaInstance> {
        self.live.values().find(|instance| instance.holds(account))
    }

    pub fn calls(&self) -> &roster::InstanceCalls {
        &self.calls
    }

    /// The copy `account` was away from and may come back into while it lives, with the world
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

    /// Whether another member of the party `account` is in holds a place in a copy of `area`
    /// that is still open.
    pub fn party_inside(&self, parties: &PlayerParties, account: &AccountId, area: AreaId) -> bool {
        let Some(party) = parties.of_account(account) else {
            return false;
        };
        self.live
            .values()
            .filter(|instance| instance.area == area && !instance.over)
            .any(|instance| {
                party
                    .accounts()
                    .any(|member| member != account && instance.holds(member))
            })
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
    pub rewards: AreaInstanceRewards,
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
    done: Vec<String>,
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

/// A member of a copy's roster, as the plate lists them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RosterRowInside {
    pub player: SocietyMember,
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
                    player: society.member(&member.account)?,
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
pub fn complete(world: &mut World, task: &str) {
    if world.contains_resource::<AreaInstanceWorld>() {
        world
            .resource_mut::<AreaInstanceTally>()
            .done
            .push(task.to_owned());
    }
}

pub fn requests(world: &mut World) {
    player::player_requests::<LeaveInstance>(world, RequestNeeds::Nothing, |world, player, _| {
        leave(world, player);
        Ok(())
    });
}

/// Sends `character` out of the instance this world is, to where leaving it leads. Whoever
/// leaves a copy that is still open has left it for good.
pub fn leave(world: &mut World, character: Entity) {
    let Some(run) = world.get_resource::<AreaInstanceWorld>().copied() else {
        return;
    };
    if world.get::<Crossing>(character).is_none() {
        area::warp::cross(world, character, run.return_to, None);
    }
}

/// Notes where every copy's roster stands as the roster of players now lists them: a member
/// inside is inside, one whose player is gone is away with their place kept, and one who has
/// been inside and now plays elsewhere has left. A copy nobody is inside counts its time from
/// here, and whoever comes back after a copy they were away from ended is told so.
pub(crate) fn look(settling: &mut Settling, content: &Content) {
    let mut listed: BTreeMap<AccountId, (Option<AreaInstanceId>, bool, Pos<Meters>)> =
        BTreeMap::new();
    let roster = settling.roster;
    roster.each(|enrolled| {
        if let Some(account) = &enrolled.account {
            let at = roster
                .place_of(enrolled.client)
                .map_or(Pos::new(0.0, 0.0), |place| place.at);
            listed.insert(account.clone(), (enrolled.instance, enrolled.away, at));
        }
    });
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
    let words = &content.built::<GameSettings>().words;
    for (account, area, others) in left {
        let said = LineText::plain(words.instance_member_left.as_str())
            .filling("{name}", settling.name(&account))
            .filling("{area}", Mention::Area(area).span(content));
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
        let said = LineText::plain(words.instance_ended_away.as_str())
            .filling("{area}", Mention::Area(area).span(content))
            .filling("{place}", place.map(LineText::from).unwrap_or_default());
        settling.tell(
            &account,
            Notification::feed(HistoryTopic::Notification, said),
        );
    }
}

/// Lets `by` into a copy of the entry's area of their own, and tells the flow that asked how it
/// went. Whoever already stands in a copy of that area moves within it instead.
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
        None => {
            alone(content, &entry).and_then(|()| open(settling, content, &entry, vec![by.clone()]))
        }
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
            .resource_mut::<RiftInstances>()
            .answer(entry.flow, Answer::Output(output));
    }
}

/// Whether the entry's area may be done alone, or why not, as the game words it.
fn alone(content: &Content, entry: &AreaInstanceEntry) -> Result<(), LineText> {
    let words = &content.built::<GameSettings>().words;
    let area = entry.to.area;
    let least = area
        .get(content)
        .instance
        .as_ref()
        .map_or(1, |rules| rules.players.least);
    if least <= 1 {
        return Ok(());
    }
    Err(LineText::plain(words.instance_needs_players.as_str())
        .filling("{area}", Mention::Area(area).span(content))
        .filling("{least}", least.to_string()))
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
    for character in content.ids::<CharacterSnapshot>() {
        let start = snapshot::start_of(content, character.get(content));
        if instanced(start) {
            loading.json_error(character.path(content), "/start", starts_inside(start));
        }
    }
}

/// Opens a copy of the entry's area for `roster`, unless the server runs as many as it may: the
/// world it is, or why not, as the game words it.
fn open(
    settling: &mut Settling,
    content: &Content,
    entry: &AreaInstanceEntry,
    roster: Vec<AccountId>,
) -> Result<AreaInstanceWorld, LineText> {
    let words = &content.built::<GameSettings>().words;
    let area = entry.to.area;
    let Some(rules) = &area.get(content).instance else {
        return Err(words.instances_busy.as_str().into());
    };
    let instances = &mut settling.society.instances;
    let worlds = content.ids::<AreaDef>().count() as u64 + u64::from(instances.opened);
    if instances.live.len() as u32 >= instances.most || worlds >= identity::MOST_WORLDS {
        return Err(words.instances_busy.as_str().into());
    }
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
/// left gets the experience whole and items rolled for them alone, paid as every payout is.
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
    let rolled: Vec<(AccountId, Vec<ItemStack>)> = run.resource_scope(|_, mut rng: Mut<Rng>| {
        participants
            .into_iter()
            .map(|account| (account, instance.rewards.rolled(&mut rng)))
            .collect()
    });
    let mut given: BTreeMap<AccountId, AreaInstanceGiven> = BTreeMap::new();
    for (account, stacks) in rolled {
        let xp = match settling.reach(&account) {
            Some((world, character)) => {
                class::grant_xp(world, character, instance.rewards.xp);
                instance.rewards.xp
            }
            None => 0,
        };
        let reason = PayoutReason::InstanceCleared(instance.area);
        let paid = payout::pay_account(settling, content, &account, &stacks, reason);
        let gave = AreaInstanceGiven {
            xp,
            kept: paid.kept,
            sent: paid.sent,
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
    let words = &content.built::<GameSettings>().words;
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
    let said = LineText::plain(words.instance_timed_out.as_str())
        .filling("{area}", Mention::Area(instance.area).span(content));
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
