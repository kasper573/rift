pub mod text;

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::time::Seconds;
use crate::data;
use crate::data::attention::Id as AttentionId;
use crate::systems::actor::bust::Face;
use crate::systems::attention;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::notice::{self, NoticeTone};
use crate::systems::player::{self, CommandLock, sender_player};
use crate::systems::reach::{self, Tether};
use crate::systems::rule::{Outcome, Requirement, RuleContext, Terms};
use crate::systems::stat;
use text::{LineText, Span};

pub use crate::data::dialogue::Id as DialogueId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Conversation>()
        .add_client_message::<PickRequest>(Channel::Ordered)
        .add_client_message::<LeaveRequest>(Channel::Ordered)
        .init_resource::<TopicSources>()
        .init_resource::<StepCounter>();
    attention::badge_source(app, talking);
}

pub struct DialogueNode {
    pub lines: &'static [Line],
    pub enter: &'static [&'static dyn Outcome],
    pub topics: bool,
    pub choices: &'static [Choice],
}

pub struct Line {
    pub by: Speaker,
    pub face: Option<Face>,
    pub cue: bool,
    pub text: &'static [Span],
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    Npc(data::npc::Id),
    Player,
    Narrator,
}

pub struct Choice {
    pub label: &'static [Span],
    pub requires: &'static [&'static dyn Requirement],
    pub unmet: Unmet,
    pub costs: &'static [ItemStack],
    pub then: &'static [&'static dyn Outcome],
    pub warn: Option<&'static str>,
}

impl Choice {
    pub const SAY: Choice = Choice {
        label: &[],
        requires: &[],
        unmet: Unmet::ShowLocked,
        costs: &[],
        then: &[],
        warn: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unmet {
    ShowLocked,
    Hide,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceTag {
    Quest,
    Shop,
    Dialogue,
}

#[derive(Clone)]
pub enum Then {
    Data(&'static [&'static dyn Outcome]),
    Made(Vec<Arc<dyn Outcome>>),
}

impl Then {
    fn outcomes(&self) -> Vec<&dyn Outcome> {
        match self {
            Then::Data(outcomes) => outcomes
                .iter()
                .map(|outcome| *outcome as &dyn Outcome)
                .collect(),
            Then::Made(outcomes) => outcomes.iter().map(|outcome| outcome.as_ref()).collect(),
        }
    }
}

#[derive(Clone)]
pub struct Offer {
    pub label: LineText,
    pub tag: ChoiceTag,
    pub icon: Option<AssetRef>,
    pub requires: &'static [&'static dyn Requirement],
    pub unmet: Unmet,
    pub costs: &'static [ItemStack],
    pub then: Then,
    pub warn: Option<&'static str>,
}

impl Offer {
    pub fn of(choice: &'static Choice) -> Offer {
        Offer {
            label: LineText::of(choice.label),
            tag: ChoiceTag::Dialogue,
            icon: None,
            requires: choice.requires,
            unmet: choice.unmet,
            costs: choice.costs,
            then: Then::Data(choice.then),
            warn: choice.warn,
        }
    }
}

pub type TopicSource = fn(&World, Entity, Entity) -> Vec<Offer>;

#[derive(Resource, Default)]
pub struct TopicSources(Vec<TopicSource>);

pub fn topic_source(app: &mut App, source: TopicSource) {
    app.world_mut()
        .resource_mut::<TopicSources>()
        .0
        .push(source);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BusyPolicy {
    Wait(Seconds),
    Replace,
    Skip,
}

#[derive(Clone)]
pub struct Start {
    pub node: DialogueId,
    pub with: Option<Entity>,
    pub tether: Option<Tether>,
    pub requires: &'static [&'static dyn Requirement],
    pub busy: BusyPolicy,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConversationStep(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conversation {
    pub step: ConversationStep,
    pub node: DialogueId,
    #[entities]
    pub with: Option<Entity>,
    pub choices: Vec<ChoiceView>,
    pub waiting: Option<WaitingView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChoiceView {
    pub label: LineText,
    pub tag: ChoiceTag,
    pub icon: Option<String>,
    pub chips: Vec<ChoiceChip>,
    pub locked: bool,
    pub warn: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ChoiceChip {
    Needs {
        what: String,
        met: bool,
    },
    Pays {
        item: data::item::Id,
        count: u32,
        have: u32,
    },
    Gets {
        item: data::item::Id,
        count: u32,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WaitingView {
    pub node: DialogueId,
    pub lasts: Seconds,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PickRequest {
    pub step: ConversationStep,
    pub choice: u32,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct LeaveRequest {
    pub step: ConversationStep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Left,
    OutOfReach,
    Died,
    Replaced,
}

pub struct GotoNode(pub DialogueId);

impl Outcome for GotoNode {
    fn apply(&self, ctx: &mut RuleContext) {
        goto(ctx.world, ctx.player, self.0);
    }

    fn leads_to(&self) -> Option<DialogueId> {
        Some(self.0)
    }
}

pub struct StartConversation {
    pub node: DialogueId,
    pub busy: BusyPolicy,
}

impl Outcome for StartConversation {
    fn apply(&self, ctx: &mut RuleContext) {
        let start = Start {
            node: self.node,
            with: ctx.speaker,
            tether: None,
            requires: &[],
            busy: self.busy,
        };
        self::start(ctx.world, ctx.player, start);
    }

    fn leads_to(&self) -> Option<DialogueId> {
        Some(self.node)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Heard(HashSet<DialogueId>);

impl Heard {
    pub fn has(&self, node: DialogueId) -> bool {
        self.0.contains(&node)
    }
}

pub fn heard(world: &World, player: Entity, node: DialogueId) -> bool {
    world
        .get::<Heard>(player)
        .is_some_and(|heard| heard.has(node))
}

pub fn start(world: &mut World, player: Entity, start: Start) {
    if !crate::systems::rule::met(world, player, start.requires) {
        return;
    }
    if world.get::<ConversationSession>(player).is_none() {
        begin(world, player, start);
        return;
    }
    match start.busy {
        BusyPolicy::Skip => {}
        BusyPolicy::Replace => {
            end(world, player, Ending::Replaced);
            begin(world, player, start);
        }
        BusyPolicy::Wait(lasts) => {
            let until = now(world) + lasts;
            world
                .entity_mut(player)
                .entry::<WaitingConversations>()
                .or_default()
                .into_mut()
                .0
                .push_back(Queued { start, until });
            refresh_view(world, player);
        }
    }
}

pub fn goto(world: &mut World, player: Entity, node: DialogueId) {
    let step = next_step(world);
    let Some(mut session) = world.get_mut::<ConversationSession>(player) else {
        return;
    };
    session.step = step;
    session.node = node;
    enter(world, player);
}

pub fn end(world: &mut World, player: Entity, ending: Ending) {
    let Some(session) = world.entity_mut(player).take::<ConversationSession>() else {
        return;
    };
    world
        .entity_mut(player)
        .remove::<(Conversation, CommandLock)>();
    if let Some(reason) = ending.reason() {
        let with = session
            .with
            .and_then(|with| world.get::<crate::systems::actor::Name>(with))
            .map(|name| format!(" with {}", name.name))
            .unwrap_or_default();
        notice::tell(
            world,
            player,
            format!("Conversation{with} ended: {reason}"),
            NoticeTone::Bad,
        );
    }
}

pub fn in_conversation(world: &World, player: Entity) -> bool {
    world.get::<ConversationSession>(player).is_some()
}

pub fn picks(world: &mut World) {
    for request in crate::systems::requests::<PickRequest>(world) {
        if let Some(player) = sender_player(world, request.client_id) {
            pick(world, player, request.message);
        }
    }
}

pub fn leaves(world: &mut World) {
    for request in crate::systems::requests::<LeaveRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if world
            .get::<ConversationSession>(player)
            .is_some_and(|session| session.step == request.message.step)
        {
            end(world, player, Ending::Left);
        }
    }
}

pub fn hold(world: &mut World, sessions: &mut QueryState<Entity, InConversationOrWaiting>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    let time = now(world);
    for player in players {
        if let Some(ending) = broken(world, player) {
            end(world, player, ending);
        }
        let expired = world
            .get_mut::<WaitingConversations>(player)
            .is_some_and(|mut waiting| {
                let before = waiting.0.len();
                waiting.0.retain(|queued| queued.until > time);
                waiting.0.len() != before
            });
        if world.get::<ConversationSession>(player).is_none() {
            let next = world
                .get_mut::<WaitingConversations>(player)
                .and_then(|mut waiting| waiting.0.pop_front());
            if let Some(queued) = next {
                let ready = crate::systems::rule::met(world, player, queued.start.requires)
                    && queued
                        .start
                        .tether
                        .is_none_or(|tether| tether.holds(world, player));
                if ready {
                    begin(world, player, queued.start);
                }
            }
        } else if expired {
            refresh_view(world, player);
        }
        if world
            .get::<WaitingConversations>(player)
            .is_some_and(|waiting| waiting.0.is_empty())
        {
            world.entity_mut(player).remove::<WaitingConversations>();
        }
    }
}

pub fn refresh(world: &mut World, sessions: &mut QueryState<Entity, With<ConversationSession>>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    for player in players {
        refresh_view(world, player);
    }
}

pub type InConversationOrWaiting = Or<(With<ConversationSession>, With<WaitingConversations>)>;

#[derive(Component)]
pub struct ConversationSession {
    step: ConversationStep,
    node: DialogueId,
    with: Option<Entity>,
    tether: Option<Tether>,
    offers: Vec<Offer>,
}

#[derive(Component, Default)]
pub struct WaitingConversations(VecDeque<Queued>);

struct Queued {
    start: Start,
    until: Seconds,
}

#[derive(Resource, Default)]
struct StepCounter(u32);

impl Ending {
    fn reason(self) -> Option<&'static str> {
        match self {
            Ending::Left | Ending::Replaced => None,
            Ending::OutOfReach => Some("you walked out of reach"),
            Ending::Died => Some("you died"),
        }
    }
}

fn begin(world: &mut World, player: Entity, start: Start) {
    if stat::is_dead(world, player) {
        return;
    }
    let step = next_step(world);
    reach::forget(world, player);
    world.entity_mut(player).insert((
        ConversationSession {
            step,
            node: start.node,
            with: start.with,
            tether: start.tether,
            offers: Vec::new(),
        },
        CommandLock,
    ));
    enter(world, player);
}

fn enter(world: &mut World, player: Entity) {
    let Some((node, with)) = world
        .get::<ConversationSession>(player)
        .map(|session| (session.node, session.with))
    else {
        return;
    };
    let def = node.get();
    world
        .entity_mut(player)
        .entry::<Heard>()
        .or_default()
        .into_mut()
        .0
        .insert(node);
    let mut ctx = RuleContext {
        world,
        player,
        speaker: with,
    };
    for outcome in def.enter {
        outcome.apply(&mut ctx);
    }
    let mut offers: Vec<Offer> = Vec::new();
    if def.topics
        && let Some(with) = with
    {
        let sources = world.resource::<TopicSources>().0.clone();
        for source in sources {
            offers.extend(source(world, player, with));
        }
    }
    offers.extend(def.choices.iter().map(Offer::of));
    offers.retain(|offer| {
        offer.unmet == Unmet::ShowLocked || crate::systems::rule::met(world, player, offer.requires)
    });
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.offers = offers;
    }
    refresh_view(world, player);
}

fn refresh_view(world: &mut World, player: Entity) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    let view = Conversation {
        step: session.step,
        node: session.node,
        with: session.with,
        choices: session
            .offers
            .iter()
            .map(|offer| choice_view(world, player, offer))
            .collect(),
        waiting: world
            .get::<WaitingConversations>(player)
            .and_then(|waiting| waiting.0.front())
            .map(|queued| WaitingView {
                node: queued.start.node,
                lasts: queued.until - now(world),
            }),
    };
    if world.get::<Conversation>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

fn choice_view(world: &World, player: Entity, offer: &Offer) -> ChoiceView {
    let outcomes = offer.then.outcomes();
    let terms = Terms {
        requires: offer.requires,
        costs: offer.costs,
        outcomes: &outcomes,
    };
    let inventory = world.get::<Inventory>(player);
    let have = |item| inventory.map_or(0, |inventory| inventory.count(item));
    let needs = offer.requires.iter().map(|requirement| ChoiceChip::Needs {
        what: requirement.describe(),
        met: requirement.met(world, player),
    });
    let pays = offer
        .costs
        .iter()
        .chain(outcomes.iter().flat_map(|outcome| outcome.takes()))
        .map(|stack| ChoiceChip::Pays {
            item: stack.item,
            count: stack.count,
            have: have(stack.item),
        });
    let gets = outcomes
        .iter()
        .flat_map(|outcome| outcome.gives())
        .map(|stack| ChoiceChip::Gets {
            item: stack.item,
            count: stack.count,
        });
    ChoiceView {
        label: offer.label.clone(),
        tag: offer.tag,
        icon: offer.icon.map(|icon| icon.0.to_owned()),
        chips: needs.chain(pays).chain(gets).collect(),
        locked: terms.refusal(world, player).is_some(),
        warn: offer.warn.map(str::to_owned),
    }
}

fn pick(world: &mut World, player: Entity, request: PickRequest) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    if session.step != request.step {
        return;
    }
    let Some(offer) = session.offers.get(request.choice as usize).cloned() else {
        return;
    };
    let speaker = session.with;
    if let Some(ending) = broken(world, player) {
        end(world, player, ending);
        return;
    }
    let outcomes = offer.then.outcomes();
    let terms = Terms {
        requires: offer.requires,
        costs: offer.costs,
        outcomes: &outcomes,
    };
    if let Err(refusal) = terms.settle(world, player, speaker) {
        notice::tell(world, player, refusal.0, NoticeTone::Bad);
        return;
    }
    if world
        .get::<ConversationSession>(player)
        .is_some_and(|session| session.step == request.step)
    {
        end(world, player, Ending::Left);
    }
}

pub fn check(starts: impl IntoIterator<Item = DialogueId>) {
    let mut seen: HashSet<DialogueId> = starts.into_iter().collect();
    let mut frontier: Vec<DialogueId> = seen.iter().copied().collect();
    while let Some(node) = frontier.pop() {
        let def = node.get();
        let next = def
            .enter
            .iter()
            .chain(def.choices.iter().flat_map(|choice| choice.then))
            .filter_map(|outcome| outcome.leads_to());
        for next in next {
            if seen.insert(next) {
                frontier.push(next);
            }
        }
    }
    for &node in <DialogueId as strum::VariantArray>::VARIANTS {
        if !seen.contains(&node) {
            panic!("dialogue {node:?}: nothing leads to it");
        }
        for line in node.get().lines {
            check_face(node, line);
        }
    }
}

fn check_face(node: DialogueId, line: &Line) {
    let Some(face) = line.face else {
        return;
    };
    let busts = match line.by {
        Speaker::Npc(npc) => npc.get().model.get().busts.as_ref(),
        Speaker::Player => player::MODEL.get().busts.as_ref(),
        Speaker::Narrator => None,
    };
    if busts.and_then(|busts| busts.face(face)).is_none() {
        panic!("dialogue {node:?}: {:?} cannot show {face:?}", line.by);
    }
}

fn talking(world: &World, player: Entity) -> Option<AttentionId> {
    in_conversation(world, player).then_some(AttentionId::Talking)
}

fn broken(world: &World, player: Entity) -> Option<Ending> {
    let session = world.get::<ConversationSession>(player)?;
    if stat::is_dead(world, player) {
        return Some(Ending::Died);
    }
    session
        .tether
        .filter(|tether| !tether.holds(world, player))
        .map(|_| Ending::OutOfReach)
}

fn next_step(world: &mut World) -> ConversationStep {
    let mut counter = world.resource_mut::<StepCounter>();
    counter.0 = counter.0.wrapping_add(1);
    ConversationStep(counter.0)
}

fn now(world: &World) -> Seconds {
    Seconds(world.resource::<Time>().elapsed_secs())
}
