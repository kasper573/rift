pub mod stage;

use std::collections::HashSet;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::math::{Percent, Rng};
use crate::core::sfx::SfxId;
use crate::data;
use crate::data::attention::Id as AttentionId;
use crate::systems::actor::Name;
use crate::systems::actor::bust::Face;
use crate::systems::attention;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::{self, CommandLock, sender_player};
use crate::systems::reach::{self, Tether};
use crate::systems::rule::{self, Encounter, Outcome, Requirement, RuleContext, Terms};
use crate::systems::stat;
use crate::systems::text::{self, LineText, Span};

pub use crate::data::dialogue::Id as DialogueId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Conversation>()
        .add_client_message::<ConversationRequest>(Channel::Ordered)
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
    pub text: &'static [Span],
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    Npc(data::npc::Id),
    Prop(data::prop::Id),
    Player,
    Narrator,
}

pub struct Choice {
    pub label: &'static [Span],
    pub requires: &'static [&'static dyn Requirement],
    pub costs: &'static [ItemStack],
    pub then: &'static [&'static dyn Outcome],
    pub reveal: &'static [ChoiceReveal],
    pub warn: Option<&'static str>,
}

impl Choice {
    pub const SAY: Choice = Choice {
        label: &[],
        requires: &[],
        costs: &[],
        then: &[],
        reveal: &[],
        warn: None,
    };
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceReveal {
    Locked,
    Needs,
    Costs,
    Gains,
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
    pub requires: Vec<&'static dyn Requirement>,
    pub costs: &'static [ItemStack],
    pub then: Then,
    pub reveal: &'static [ChoiceReveal],
    pub warn: Option<&'static str>,
}

impl Offer {
    pub fn of(choice: &'static Choice, world: &World, player: Entity) -> Offer {
        Offer {
            label: LineText::spoken(choice.label, world, player),
            tag: ChoiceTag::Dialogue,
            icon: None,
            requires: choice.requires.to_vec(),
            costs: choice.costs,
            then: Then::Data(choice.then),
            reveal: choice.reveal,
            warn: choice.warn,
        }
    }

    fn reveals(&self, reveal: ChoiceReveal) -> bool {
        self.reveal.contains(&reveal)
    }
}

pub struct Asked {
    pub player: Entity,
    pub with: Option<Entity>,
    pub node: DialogueId,
}

impl Asked {
    pub fn greeting(&self) -> Option<Entity> {
        self.with.filter(|_| self.node.get().topics)
    }
}

pub type TopicSource = fn(&World, &Asked) -> Vec<Offer>;

#[derive(Resource, Default)]
pub struct TopicSources(Vec<TopicSource>);

pub fn topic_source(app: &mut App, source: TopicSource) {
    app.world_mut()
        .resource_mut::<TopicSources>()
        .0
        .push(source);
}

#[derive(Clone)]
pub struct Start {
    pub node: DialogueId,
    pub with: Option<Entity>,
    pub tether: Option<Tether>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConversationStep(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conversation {
    pub step: ConversationStep,
    pub node: DialogueId,
    #[entities]
    pub with: Option<Entity>,
    pub lines: Vec<SpokenLine>,
    pub choices: Vec<ChoiceView>,
    pub refused: Option<RefusedPick>,
    pub remark: Option<Remark>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RefusedPick {
    pub nth: u32,
    pub choice: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Remark {
    pub nth: u32,
    pub line: SpokenLine,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpokenLine {
    pub by: Speaker,
    pub face: Option<Face>,
    pub text: LineText,
}

impl SpokenLine {
    pub fn spoken(line: &Line, world: &World, player: Entity) -> SpokenLine {
        SpokenLine {
            by: line.by,
            face: line.face,
            text: LineText::spoken(line.text, world, player),
        }
    }
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

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ConversationRequest {
    Pick { step: ConversationStep, choice: u32 },
    Leave { step: ConversationStep },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Left,
    OutOfReach,
    Died,
    Gone,
    Replaced,
}

pub struct GotoNode(pub DialogueId);

impl Outcome for GotoNode {
    fn apply(&self, ctx: &mut RuleContext) {
        goto(ctx.world, ctx.player, self.0);
    }

    fn leads_to(&self) -> Vec<DialogueId> {
        vec![self.0]
    }
}

pub struct StartConversation {
    pub node: DialogueId,
}

impl Outcome for StartConversation {
    fn apply(&self, ctx: &mut RuleContext) {
        let start = Start {
            node: self.node,
            with: ctx.encounter.with,
            tether: ctx.encounter.tether,
        };
        self::start(ctx.world, ctx.player, start);
    }

    fn leads_to(&self) -> Vec<DialogueId> {
        vec![self.node]
    }
}

pub struct Gamble {
    pub odds: Percent,
    pub won: &'static [&'static dyn Outcome],
    pub lost: &'static [&'static dyn Outcome],
}

impl Outcome for Gamble {
    fn apply(&self, ctx: &mut RuleContext) {
        let won = ctx
            .world
            .resource_scope(|_, mut rng: Mut<Rng>| self.odds.rolled(&mut rng));
        let terms = Terms {
            requires: &[],
            costs: &[],
            outcomes: if won { self.won } else { self.lost },
        };
        if let Err(refusal) = terms.settle(ctx.world, ctx.player, ctx.encounter) {
            notification::notify(
                ctx.world,
                ctx.player,
                Notification::new(NotificationKind::error(), LineText::plain(refusal.0)),
            );
        }
    }

    fn leads_to(&self) -> Vec<DialogueId> {
        self.won
            .iter()
            .chain(self.lost)
            .flat_map(|outcome| outcome.leads_to())
            .collect()
    }

    fn check(&self, assets: &AssetService) {
        for outcome in self.won.iter().chain(self.lost) {
            outcome.check(assets);
        }
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
    end(world, player, Ending::Replaced);
    begin(world, player, start);
}

pub fn goto(world: &mut World, player: Entity, node: DialogueId) {
    let step = next_step(world);
    let Some(mut session) = world.get_mut::<ConversationSession>(player) else {
        return;
    };
    session.step = step;
    session.node = node;
    session.remark = None;
    session.refused = None;
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
            .and_then(|with| world.get::<Name>(with))
            .map(|name| format!(" with {}", name.name))
            .unwrap_or_default();
        notification::notify(
            world,
            player,
            Notification::new(
                NotificationKind::Error {
                    sfx: Some(SfxId::UiClose),
                },
                LineText::plain(format!("Conversation{with} ended: {reason}")),
            ),
        );
    }
}

pub fn remark(world: &mut World, player: Entity, to: Entity, line: &'static Line) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    if session.with != Some(to) {
        return;
    }
    let nth = session.remark.as_ref().map_or(0, |remark| remark.nth) + 1;
    let line = SpokenLine::spoken(line, world, player);
    record_line(world, player, &line);
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.remark = Some(Remark { nth, line });
    }
    refresh_view(world, player);
}

pub fn in_conversation(world: &World, player: Entity) -> bool {
    world.get::<ConversationSession>(player).is_some()
}

pub fn engaged_with(world: &World, player: Entity, with: Option<Entity>) -> bool {
    world
        .get::<ConversationSession>(player)
        .is_some_and(|session| session.with == with)
        && broken(world, player).is_none()
}

pub fn requests(world: &mut World) {
    for request in crate::systems::requests::<ConversationRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        match request.message {
            ConversationRequest::Pick { step, choice } => pick(world, player, step, choice),
            ConversationRequest::Leave { step } => {
                if world
                    .get::<ConversationSession>(player)
                    .is_some_and(|session| session.step == step)
                {
                    end(world, player, Ending::Left);
                }
            }
        }
    }
}

pub fn hold(world: &mut World, sessions: &mut QueryState<Entity, With<ConversationSession>>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    for player in players {
        if let Some(ending) = broken(world, player) {
            end(world, player, ending);
        }
    }
}

pub fn refresh(world: &mut World, sessions: &mut QueryState<Entity, With<ConversationSession>>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    for player in players {
        refresh_view(world, player);
    }
}

#[derive(Component)]
pub struct ConversationSession {
    step: ConversationStep,
    node: DialogueId,
    with: Option<Entity>,
    tether: Option<Tether>,
    lines: Vec<SpokenLine>,
    offers: Vec<Offer>,
    refused: Option<RefusedPick>,
    refusals: u32,
    remark: Option<Remark>,
}

impl ConversationSession {
    fn encounter(&self) -> Encounter {
        Encounter {
            with: self.with,
            tether: self.tether,
        }
    }
}

#[derive(Resource, Default)]
struct StepCounter(u32);

impl Ending {
    fn reason(self) -> Option<&'static str> {
        match self {
            Ending::Left | Ending::Replaced => None,
            Ending::OutOfReach => Some("you walked out of reach"),
            Ending::Died => Some("you died"),
            Ending::Gone => Some("they're gone"),
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
            lines: Vec::new(),
            offers: Vec::new(),
            refused: None,
            refusals: 0,
            remark: None,
        },
        CommandLock,
    ));
    let with = start
        .with
        .and_then(|with| world.get::<Name>(with))
        .map(|name| name.name.clone())
        .or_else(|| {
            start
                .node
                .get()
                .lines
                .iter()
                .find_map(|line| speaker_name(world, player, line.by))
        });
    history::record(
        world,
        player,
        HistoryEntry::of(HistoryTopic::Talk, LineText::default())
            .by(with)
            .mark(Some(HistoryMark::Began)),
    );
    enter(world, player);
}

fn enter(world: &mut World, player: Entity) {
    let Some((node, encounter)) = world
        .get::<ConversationSession>(player)
        .map(|session| (session.node, session.encounter()))
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
        encounter,
        requires: &[],
    };
    for outcome in def.enter {
        outcome.apply(&mut ctx);
    }
    let asked = Asked {
        player,
        with: encounter.with,
        node,
    };
    let sources = world.resource::<TopicSources>().0.clone();
    let mut offers: Vec<Offer> = sources
        .iter()
        .flat_map(|source| source(world, &asked))
        .collect();
    offers.extend(
        def.choices
            .iter()
            .map(|choice| Offer::of(choice, world, player)),
    );
    offers.retain(|offer| {
        offer.reveals(ChoiceReveal::Locked) || rule::met(world, player, &offer.requires)
    });
    let lines: Vec<SpokenLine> = def
        .lines
        .iter()
        .map(|line| SpokenLine::spoken(line, world, player))
        .collect();
    for line in &lines {
        record_line(world, player, line);
    }
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.lines = lines;
        session.offers = offers;
    }
    refresh_view(world, player);
}

fn record_line(world: &mut World, player: Entity, line: &SpokenLine) {
    let by = speaker_name(world, player, line.by);
    let mark = (line.by == Speaker::Player).then_some(HistoryMark::You);
    history::record(
        world,
        player,
        HistoryEntry::of(HistoryTopic::Talk, line.text.clone())
            .by(by)
            .mark(mark),
    );
}

fn speaker_name(world: &World, player: Entity, who: Speaker) -> Option<String> {
    match who {
        Speaker::Npc(npc) => Some(npc.get().display_name.to_owned()),
        Speaker::Prop(prop) => Some(prop.get().display_name.to_owned()),
        Speaker::Player => world.get::<Name>(player).map(|name| name.name.clone()),
        Speaker::Narrator => None,
    }
}

fn refresh_view(world: &mut World, player: Entity) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    let view = Conversation {
        step: session.step,
        node: session.node,
        with: session.with,
        lines: session.lines.clone(),
        choices: session
            .offers
            .iter()
            .map(|offer| choice_view(world, player, offer))
            .collect(),
        refused: session.refused.clone(),
        remark: session.remark.clone(),
    };
    if world.get::<Conversation>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

fn choice_view(world: &World, player: Entity, offer: &Offer) -> ChoiceView {
    let outcomes = offer.then.outcomes();
    let inventory = world.get::<Inventory>(player);
    let have = |item| inventory.map_or(0, |inventory| inventory.count(item));
    let needs = offer
        .requires
        .iter()
        .filter(|_| offer.reveals(ChoiceReveal::Needs))
        .map(|requirement| ChoiceChip::Needs {
            what: requirement.describe(),
            met: requirement.met(world, player),
        });
    let pays = offer
        .costs
        .iter()
        .chain(outcomes.iter().flat_map(|outcome| outcome.takes()))
        .filter(|_| offer.reveals(ChoiceReveal::Costs))
        .map(|stack| ChoiceChip::Pays {
            item: stack.item,
            count: stack.count,
            have: have(stack.item),
        });
    let gets = outcomes
        .iter()
        .flat_map(|outcome| outcome.gives())
        .filter(|_| offer.reveals(ChoiceReveal::Gains))
        .map(|stack| ChoiceChip::Gets {
            item: stack.item,
            count: stack.count,
        });
    ChoiceView {
        label: offer.label.clone(),
        tag: offer.tag,
        icon: offer.icon.map(|icon| icon.0.to_owned()),
        chips: needs.chain(pays).chain(gets).collect(),
        locked: !rule::met(world, player, &offer.requires),
        warn: offer.warn.map(str::to_owned),
    }
}

const LOCKED: &str = "You can't do that yet.";
const NO_LONGER: &str = "You can't do that any more.";

fn pick(world: &mut World, player: Entity, step: ConversationStep, choice: u32) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    if session.step != step {
        return;
    }
    let Some(offer) = session.offers.get(choice as usize).cloned() else {
        return;
    };
    let encounter = session.encounter();
    if let Some(ending) = broken(world, player) {
        end(world, player, ending);
        return;
    }
    let outcomes = offer.then.outcomes();
    let terms = Terms {
        requires: &offer.requires,
        costs: offer.costs,
        outcomes: &outcomes,
    };
    let met = rule::met(world, player, &offer.requires);
    let said = HistoryEntry::of(HistoryTopic::Talk, offer.label.clone())
        .by(speaker_name(world, player, Speaker::Player))
        .mark(Some(HistoryMark::You));
    history::record(world, player, said);
    if let Err(refusal) = terms.settle(world, player, encounter) {
        let reason = match (
            met || offer.reveals(ChoiceReveal::Needs),
            offer.reveals(ChoiceReveal::Locked),
        ) {
            (true, _) => refusal.0,
            (false, true) => LOCKED.to_owned(),
            (false, false) => NO_LONGER.to_owned(),
        };
        refuse(world, player, choice, reason);
        return;
    }
    if world
        .get::<ConversationSession>(player)
        .is_some_and(|session| session.step == step)
    {
        end(world, player, Ending::Left);
    }
}

fn refuse(world: &mut World, player: Entity, choice: u32, reason: String) {
    notification::notify(
        world,
        player,
        Notification::new(NotificationKind::error(), LineText::plain(reason)),
    );
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.refusals += 1;
        session.refused = Some(RefusedPick {
            nth: session.refusals,
            choice,
        });
    }
    refresh_view(world, player);
}

pub fn check(assets: &AssetService, starts: impl IntoIterator<Item = DialogueId>) {
    let mut seen: HashSet<DialogueId> = starts.into_iter().collect();
    let mut frontier: Vec<DialogueId> = seen.iter().copied().collect();
    while let Some(node) = frontier.pop() {
        let def = node.get();
        let next = def
            .enter
            .iter()
            .chain(def.choices.iter().flat_map(|choice| choice.then))
            .flat_map(|outcome| outcome.leads_to());
        for next in next {
            if seen.insert(next) {
                frontier.push(next);
            }
        }
    }
    for &node in DialogueId::VARIANTS {
        if !seen.contains(&node) {
            panic!("dialogue {node:?}: nothing leads to it");
        }
        if node.get().lines.is_empty() {
            panic!("dialogue {node:?}: has no lines");
        }
        for line in node.get().lines {
            check_line(format!("dialogue {node:?}"), line);
        }
        let def = node.get();
        for (index, choice) in def.choices.iter().enumerate() {
            let owner = format!("dialogue {node:?}, choice {}", index + 1);
            for fill in text::fills(choice.label) {
                fill.check();
            }
            check_reveal(
                &owner,
                choice.reveal,
                Revealable {
                    needs: !choice.requires.is_empty(),
                    costs: !choice.costs.is_empty()
                        || choice
                            .then
                            .iter()
                            .any(|outcome| !outcome.takes().is_empty()),
                    gains: choice
                        .then
                        .iter()
                        .any(|outcome| !outcome.gives().is_empty()),
                },
            );
        }
        for outcome in def
            .enter
            .iter()
            .chain(def.choices.iter().flat_map(|choice| choice.then))
        {
            outcome.check(assets);
        }
    }
}

pub struct Revealable {
    pub needs: bool,
    pub costs: bool,
    pub gains: bool,
}

pub fn check_reveal(owner: &str, reveal: &[ChoiceReveal], revealable: Revealable) {
    for (index, &kind) in reveal.iter().enumerate() {
        if reveal[..index].contains(&kind) {
            panic!("{owner}: reveals {kind:?} twice");
        }
        let (has, what) = match kind {
            ChoiceReveal::Locked | ChoiceReveal::Needs => (revealable.needs, "needs nothing"),
            ChoiceReveal::Costs => (revealable.costs, "costs nothing"),
            ChoiceReveal::Gains => (revealable.gains, "gives nothing"),
        };
        if !has {
            panic!("{owner}: reveals {kind:?} but {what}");
        }
    }
}

pub fn check_line(owner: impl std::fmt::Display, line: &Line) {
    for fill in text::fills(line.text) {
        fill.check();
    }
    if let Speaker::Npc(npc) = line.by
        && npc.get().babble.is_none()
    {
        panic!("{owner}: {npc:?} speaks but has no babble");
    }
    let Some(face) = line.face else {
        return;
    };
    let busts = match line.by {
        Speaker::Npc(npc) => npc.get().model.get().busts.as_ref(),
        Speaker::Player => player::def().model.get().busts.as_ref(),
        Speaker::Prop(_) | Speaker::Narrator => None,
    };
    if busts.and_then(|busts| busts.face(face)).is_none() {
        panic!("{owner}: {:?} cannot show {face:?}", line.by);
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
    if let Some(with) = session.with
        && (world.get_entity(with).is_err() || stat::is_dead(world, with))
    {
        return Some(Ending::Gone);
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
