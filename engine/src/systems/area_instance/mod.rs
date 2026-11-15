//! Instanced areas: an area that is never shared. Whoever enters gets a copy of it that is their
//! party's own, a world of its own, with goals to meet and rewards for meeting them. What the
//! rewards are and where the party comes back to are decided by whatever let them in. Every
//! instance is society's: it opens them, counts their goals and their time, hands out their
//! rewards and says when one is gone.

pub mod effect;
pub mod plate;

use std::collections::{BTreeMap, BTreeSet};

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentProblem, Loading, RowCheck, SchemaMark, serial};
use crate::core::math::Rng;
use crate::core::script::{Answer, NEXT, REFUSED};
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::account::AccountId;
use crate::systems::area::transition::Crossing;
use crate::systems::area::{self, AreaDef, AreaId};
use crate::systems::combat::Died;
use crate::systems::history::HistoryTopic;
use crate::systems::item::{DropRow, Inventory, ItemStack};
use crate::systems::notification::{self, Notification};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party::{PartyId, PlayerParties};
use crate::systems::player::snapshot::{self, CharacterSnapshot};
use crate::systems::player::{self, CharacterAccount, RequestNeeds};
use crate::systems::script::{EffectReach, FlowKey, OwnerRow, Place, RiftInstances, RiftScripts};
use crate::systems::society::{ServerSociety, Settling, Society};
use crate::systems::{GameSettings, SETTINGS_FILE, class, identity, inbox};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<LeaveInstance>();
}

/// What makes an area one that is only ever entered as an instance.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaInstanceRules {
    /// The most players inside one at once.
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub holds: u32,
    /// What clears one, once every one of them is met.
    pub goals: Vec<AreaInstanceGoal>,
    /// How long one with nobody inside waits before it is gone.
    pub empties_after: Seconds,
    /// How long one stays open once it is cleared, and how long whoever it is no longer theirs to
    /// be in stays inside.
    pub closes_after: Seconds,
    /// How long one may live uncleared. Without it, for as long as someone comes back to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lasts: Option<Seconds>,
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
        cx.at_least(format!("{pointer}/holds"), self.holds, 1, "players");
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

/// Whose an instance is: a party's, or one player's who was in no party as they opened it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AreaInstanceHolder {
    Party(PartyId),
    Alone(AccountId),
}

impl AreaInstanceHolder {
    /// Who holds what `account` opens now.
    fn of(parties: &PlayerParties, account: &AccountId) -> AreaInstanceHolder {
        match parties.of_account(account) {
            Some(party) => AreaInstanceHolder::Party(party.id),
            None => AreaInstanceHolder::Alone(account.clone()),
        }
    }

    /// Whether what it holds is `account`'s to be in.
    fn lets_in(&self, parties: &PlayerParties, account: &AccountId) -> bool {
        match self {
            AreaInstanceHolder::Party(party) => parties
                .of_account(account)
                .is_some_and(|theirs| theirs.id == *party),
            AreaInstanceHolder::Alone(holder) => holder == account,
        }
    }

    /// Whether nobody can be let in any longer.
    fn is_gone(&self, parties: &PlayerParties) -> bool {
        match self {
            AreaInstanceHolder::Party(party) => !parties.holds(*party),
            AreaInstanceHolder::Alone(_) => false,
        }
    }
}

/// An instance of an area, as society holds it while it lives.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaInstance {
    pub id: AreaInstanceId,
    pub area: AreaId,
    pub holder: AreaInstanceHolder,
    /// What clearing it gives, as whoever opened it was let in with.
    pub rewards: AreaInstanceRewards,
    /// Where leaving it leads, as whoever opened it was let in with.
    pub return_to: Place,
    opened: UnixMillis,
    /// How far each of the area's goals has come, in their order.
    progress: Vec<u32>,
    /// Who is inside, as the roster listed them when the worlds last conferred.
    inside: BTreeSet<AccountId>,
    /// Who was let in since, and is on their way.
    entering: BTreeSet<AccountId>,
    /// Whose player left the game while they were inside, and has not been inside since.
    dropped: BTreeSet<AccountId>,
    /// Since when nobody has been inside.
    empty_since: Option<UnixMillis>,
    cleared: Option<AreaInstanceClearing>,
    /// When each of those inside it is no longer theirs to be in is sent out.
    sent_out: BTreeMap<AccountId, UnixMillis>,
    /// Whether it has ended, and everyone inside is on their way out.
    over: bool,
}

impl AreaInstance {
    pub fn inside(&self) -> impl Iterator<Item = &AccountId> {
        self.inside.iter()
    }

    pub fn is_cleared(&self) -> bool {
        self.cleared.is_some()
    }

    fn headcount(&self) -> usize {
        self.inside.union(&self.entering).count()
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

/// Every instance the server runs.
#[derive(Debug, Default)]
pub struct AreaInstances {
    live: BTreeMap<AreaInstanceId, AreaInstance>,
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

    /// The instance `account` is inside.
    pub fn around(&self, account: &AccountId) -> Option<&AreaInstance> {
        self.live
            .values()
            .find(|instance| instance.inside.contains(account))
    }

    /// Lets the server run `most` at once.
    pub(crate) fn run_at_most(&mut self, most: u32) {
        self.most = most;
    }

    /// The instance of `area` a door lets `holder` into, while it lives.
    fn of_holder(&self, holder: &AreaInstanceHolder, area: AreaId) -> Option<AreaInstanceId> {
        self.live
            .values()
            .find(|instance| instance.area == area && instance.holder == *holder && !instance.over)
            .map(|instance| instance.id)
    }
}

/// What a script asks society for as it lets a player into an instance.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaInstanceEntry {
    /// The instanced area, and where in it entrants appear.
    pub to: Place,
    pub rewards: AreaInstanceRewards,
    pub return_to: Place,
    /// The flow that waits to hear whether they were let in.
    pub flow: FlowKey,
    /// Whether the player is told what refused them, the script having nothing to say of it.
    pub told: bool,
}

/// On the world of an instance: which it is, and where leaving it leads.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct AreaInstanceWorld {
    pub id: AreaInstanceId,
    pub return_to: Place,
}

impl AreaInstanceWorld {
    pub fn of(instance: &AreaInstance) -> AreaInstanceWorld {
        AreaInstanceWorld {
            id: instance.id,
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

/// A player asks to leave the instance they are inside.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaveInstance;

/// The instance a player is inside, as their client shows it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaInstanceShown {
    pub area: AreaId,
    pub goals: Vec<AreaInstanceGoalShown>,
    /// How many are inside, of how many it holds.
    pub inside: u32,
    pub holds: u32,
    /// When it ends uncleared, in an area whose instances only last so long.
    pub ends: Option<UnixMillis>,
    /// The area leaving it leads to.
    pub returns_to: AreaId,
    /// Once it is cleared: what that gave the player, and when everyone is returned.
    pub cleared: Option<AreaInstanceClearedShown>,
    /// When the player is sent out of one that is no longer theirs to be in.
    pub sent_out: Option<UnixMillis>,
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
        Some(AreaInstanceShown {
            area: instance.area,
            goals,
            inside: instance.inside.len() as u32,
            holds: rules.holds,
            ends: match cleared {
                Some(_) => None,
                None => rules.lasts.map(|lasts| instance.opened.after(lasts)),
            },
            returns_to: instance.return_to.area,
            cleared,
            sent_out: instance.sent_out.get(account).copied(),
        })
    }
}

/// Whether this world is an instance.
pub fn inside(world: &World) -> bool {
    world.contains_resource::<AreaInstanceWorld>()
}

/// How many are inside the instance of `area` a door would let `character` into.
pub fn headcount(world: &World, character: Entity, area: AreaId) -> u32 {
    let Some(account) = world.get::<CharacterAccount>(character) else {
        return 0;
    };
    let society = world.resource::<ServerSociety>().read();
    let holder = AreaInstanceHolder::of(society.parties(), &account.0);
    society
        .instances()
        .of_holder(&holder, area)
        .and_then(|id| society.instances().get(id))
        .map_or(0, |instance| instance.inside.len() as u32)
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

/// Sends `character` out of the instance this world is, to where leaving it leads.
pub fn leave(world: &mut World, character: Entity) {
    let Some(run) = world.get_resource::<AreaInstanceWorld>().copied() else {
        return;
    };
    if world.get::<Crossing>(character).is_none() {
        area::warp::cross(world, character, run.return_to, None);
    }
}

/// Notes who is inside every instance as the roster now lists them: whoever was inside and
/// plays nowhere any longer lost their player inside, and one nobody is inside counts its time
/// from here.
pub(crate) fn look(settling: &mut Settling) {
    if settling.society.instances.live.is_empty() {
        return;
    }
    let mut listed: BTreeMap<AreaInstanceId, BTreeSet<AccountId>> = BTreeMap::new();
    settling.roster.each(|enrolled| {
        if let (Some(instance), Some(account)) = (enrolled.instance, &enrolled.account) {
            listed.entry(instance).or_default().insert(account.clone());
        }
    });
    let (roster, now) = (settling.roster, settling.now);
    let mut changed: Vec<AccountId> = Vec::new();
    for instance in settling.society.instances.live.values_mut() {
        let inside = listed.remove(&instance.id).unwrap_or_default();
        for left in instance.inside.difference(&inside) {
            if roster.account(left).is_none() {
                instance.dropped.insert(left.clone());
            }
        }
        instance.dropped.retain(|account| !inside.contains(account));
        if inside != instance.inside {
            changed.extend(instance.inside.union(&inside).cloned());
        }
        instance.inside = inside;
        instance.entering.clear();
        instance.empty_since = if instance.inside.is_empty() {
            instance.empty_since.or(Some(now))
        } else {
            None
        };
    }
    for account in changed {
        settling.society.changed(&account);
    }
}

/// Lets `by` into their instance of the area the entry leads to, opening one when they have
/// none, and tells the flow that asked how it went.
pub(crate) fn enter(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    entry: AreaInstanceEntry,
) {
    let admitted = admit(settling, content, by, &entry).map(|id| AreaInstanceWorld {
        id,
        return_to: settling
            .society
            .instances
            .get(id)
            .map_or(entry.return_to, |instance| instance.return_to),
    });
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
    world
        .resource_mut::<RiftInstances>()
        .answer(entry.flow, Answer::Output(output));
}

/// What every instance comes to as the worlds confer: its goals counted, its rewards handed out
/// once they are all met, its time kept, whoever must leave sent out, and one that is done with
/// gone.
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
    let society = &mut *settling.society;
    let parties = &society.parties;
    society.instances.live.retain(|_, instance| {
        let waited = |since: UnixMillis| {
            let rules = instance.area.get(content).instance.as_ref();
            rules.is_none_or(|rules| now.since(since) >= rules.empties_after)
        };
        let done = instance.over || instance.holder.is_gone(parties);
        let empty = instance.headcount() == 0;
        !(empty && (done || instance.empty_since.is_some_and(waited)))
    });
}

/// What the game's settings must hold for its instances: nobody starts inside an instanced
/// area, and a door that rewards items has messages to send what a bag has no room for.
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
    if settings.messages.is_some() {
        return;
    }
    let scripts = content.built::<RiftScripts>();
    let unsent: Vec<OwnerRow> = scripts
        .owners()
        .map(|(row, _)| *row)
        .filter(|row| {
            let reaches = &scripts.may_do(*row).reaches;
            reaches
                .iter()
                .any(|reach| matches!(reach, EffectReach::EntersInstance { items: true, .. }))
        })
        .collect();
    for row in unsent {
        let site = crate::systems::script::owner_site(content, row);
        loading.json_error(
            &site.file,
            &site.pointer,
            ContentProblem::new(
                "rewards items without messages",
                "an enter_instance whose rewards hold items",
                "messages in the game's settings, to send what a bag has no room for",
            ),
        );
    }
}

/// Whether `by` is let into their instance of the entry's area: the one they are let into, or
/// why not, as the game words it.
fn admit(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    entry: &AreaInstanceEntry,
) -> Result<AreaInstanceId, String> {
    let words = &content.built::<GameSettings>().words;
    let area = entry.to.area;
    let def = area.get(content);
    let Some(rules) = &def.instance else {
        return Err(words.instances_busy.clone());
    };
    let society = &mut *settling.society;
    let holder = AreaInstanceHolder::of(&society.parties, by);
    let instances = &mut society.instances;
    if let Some(id) = instances.of_holder(&holder, area) {
        let instance = instances.live.get_mut(&id).expect("a live instance");
        if !instance.inside.contains(by) && !instance.entering.contains(by) {
            if instance.headcount() >= rules.holds as usize {
                return Err(words
                    .instance_full
                    .replace("{area}", &def.name)
                    .replace("{holds}", &rules.holds.to_string()));
            }
            instance.entering.insert(by.clone());
        }
        return Ok(id);
    }
    let worlds = content.ids::<AreaDef>().count() as u64 + u64::from(instances.opened);
    if instances.live.len() as u32 >= instances.most || worlds >= identity::MOST_WORLDS {
        return Err(words.instances_busy.clone());
    }
    let id = AreaInstanceId(instances.opened);
    instances.opened += 1;
    instances.live.insert(
        id,
        AreaInstance {
            id,
            area,
            holder,
            rewards: entry.rewards.clone(),
            return_to: entry.return_to,
            opened: settling.now,
            progress: vec![0; rules.goals.len()],
            inside: BTreeSet::new(),
            entering: BTreeSet::from([by.clone()]),
            dropped: BTreeSet::new(),
            empty_since: None,
            cleared: None,
            sent_out: BTreeMap::new(),
            over: false,
        },
    );
    Ok(id)
}

/// Has `character` step through into the instance `run`, at `to`: on foot, whatever they rode.
fn step_in(world: &mut World, character: Entity, to: Place, run: AreaInstanceWorld) {
    let here = world
        .get_resource::<AreaInstanceWorld>()
        .map(|here| here.id);
    if here == Some(run.id) {
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
            changed.extend(instance.inside.iter().cloned());
        }
    }
    for account in changed {
        settling.society.changed(&account);
    }
}

/// Clears the instance `id` once every goal is met, once: each participant gets the experience
/// whole and items rolled for them alone, into their bag as far as it has room, the rest sent to
/// them by the game. One whose player is gone gets every item sent.
fn clear(settling: &mut Settling, content: &Content, id: AreaInstanceId) {
    let settings = content.built::<GameSettings>();
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
    let parties = &settling.society.parties;
    let participants: Vec<AccountId> = instance
        .inside
        .iter()
        .chain(
            instance
                .dropped
                .iter()
                .filter(|account| instance.holder.lets_in(parties, account)),
        )
        .cloned()
        .collect();
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
    let said = (
        settings
            .words
            .instance_rewards_title
            .replace("{area}", &def.name),
        settings
            .words
            .instance_rewards_text
            .replace("{area}", &def.name),
    );
    let mut given: BTreeMap<AccountId, AreaInstanceGiven> = BTreeMap::new();
    for (account, stacks) in rolled {
        // Whoever is not at their character is not there to be handed anything.
        let attended = settling
            .reach(&account)
            .is_some_and(|(world, character)| player::attended(world, character));
        let gave = match settling.reach(&account) {
            Some((world, character)) => {
                class::grant_xp(world, character, instance.rewards.xp);
                let sent = match world.get_mut::<Inventory>(character) {
                    Some(mut bag) if attended => bag.give_what_fits(content, &stacks),
                    _ => stacks.clone(),
                };
                AreaInstanceGiven {
                    xp: instance.rewards.xp,
                    kept: stacks
                        .iter()
                        .filter(|stack| !sent.contains(stack))
                        .copied()
                        .collect(),
                    sent,
                }
            }
            None => AreaInstanceGiven {
                xp: 0,
                kept: Vec::new(),
                sent: stacks,
            },
        };
        if !gave.sent.is_empty() {
            inbox::from_game(
                settling.society,
                settling.unwritten,
                settling.now,
                &account,
                (&said.0, &said.1),
                gave.sent.clone(),
            );
        }
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

/// Keeps the time of the instance `id`: one cleared closes when the area says, one that only
/// lasts so long ends unrewarded, and whoever inside it is no longer theirs to be in is told and
/// given the area's time to leave.
fn time(settling: &mut Settling, content: &Content, id: AreaInstanceId) {
    let words = &content.built::<GameSettings>().words;
    let now = settling.now;
    let society = &mut *settling.society;
    let Some(instance) = society.instances.live.get_mut(&id) else {
        return;
    };
    let def = instance.area.get(content);
    let Some(rules) = &def.instance else {
        return;
    };
    let mut told: Vec<(AccountId, String)> = Vec::new();
    if !instance.over {
        let closed = instance
            .cleared
            .as_ref()
            .is_some_and(|clearing| now >= clearing.at.after(rules.closes_after));
        let ran_out = instance.cleared.is_none()
            && rules
                .lasts
                .is_some_and(|lasts| now >= instance.opened.after(lasts));
        if ran_out {
            let said = words.instance_timed_out.replace("{area}", &def.name);
            told.extend(
                instance
                    .inside
                    .iter()
                    .map(|account| (account.clone(), said.clone())),
            );
        }
        instance.over = closed || ran_out;
    }
    let mut changed: Vec<AccountId> = Vec::new();
    let inside = instance.inside.clone();
    instance
        .sent_out
        .retain(|account, _| inside.contains(account));
    for account in &inside {
        let theirs = instance.holder.lets_in(&society.parties, account);
        match (theirs, instance.sent_out.contains_key(account)) {
            (true, true) => {
                instance.sent_out.remove(account);
                changed.push(account.clone());
            }
            (false, false) => {
                instance
                    .sent_out
                    .insert(account.clone(), now.after(rules.closes_after));
                told.push((account.clone(), words.instance_not_yours.clone()));
                changed.push(account.clone());
            }
            _ => {}
        }
    }
    for account in changed {
        society.changed(&account);
    }
    for (account, said) in told {
        settling.tell(
            &account,
            Notification::feed(HistoryTopic::Notification, said),
        );
    }
}

/// Sends out of the instance `id` whoever must leave it now: everyone once it is over, and
/// whoever it is no longer theirs to be in once their time is up.
fn send_out(settling: &mut Settling, id: AreaInstanceId) {
    let now = settling.now;
    let Some(instance) = settling.society.instances.get(id) else {
        return;
    };
    let leaving: Vec<AccountId> = instance
        .inside
        .iter()
        .filter(|account| {
            instance.over
                || instance
                    .sent_out
                    .get(*account)
                    .is_some_and(|out_at| now >= *out_at)
        })
        .cloned()
        .collect();
    for account in leaving {
        if let Some((world, character)) = settling.reach(&account) {
            leave(world, character);
        }
    }
}
