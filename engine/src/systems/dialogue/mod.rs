pub mod camera;
pub mod stage;

use std::collections::HashSet;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::ImageRef;
use crate::core::content::{Content, Fixture};
use crate::core::script::{
    self, Answer, BoxEnding, ChoiceOffer, ChoiceState, DialogueView, RevealDef, ScriptFault,
};
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::Name;
use crate::systems::actor::skeleton::GestureName;
use crate::systems::attention::{AttentionId, AttentionMark};
use crate::systems::combat::Attitude;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::movement::Position;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::Npc;
use crate::systems::player::{CommandLock, player_requests};
use crate::systems::quest::QuestId;
use crate::systems::reach::{self, Tether};
use crate::systems::script::{
    Condition, EventKind, FlowKey, RiftEffect, RiftEvent, RiftInstances, RiftScripting, ScriptLine,
    Speaker, condition,
};
use crate::systems::stat;
use crate::systems::text::LineText;
use crate::systems::visibility;
use camera::DialogueCamera;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate_private::<Conversation>()
        .add_client_message::<ConversationRequest>();
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChoiceTag {
    Quest,
    Shop,
    Dialogue,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConversationStep(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conversation {
    pub step: ConversationStep,
    pub node: String,
    #[entities]
    pub with: Option<Entity>,
    /// How the conversation is seen, unless the line on show says otherwise.
    pub camera: DialogueCamera,
    pub lines: Vec<SpokenLine>,
    pub choices: Vec<ChoiceView>,
    pub panel: Option<ConversationPanel>,
    pub refused: Option<RefusedPick>,
    pub remark: Option<Remark>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationPanel {
    QuestOffer(QuestId),
    QuestTurnIn(QuestId),
    QuestDecision(QuestId),
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
    pub gesture: Option<GestureName>,
    pub camera: Option<DialogueCamera>,
    pub text: LineText,
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
    Needs { what: condition::Wording, met: bool },
    Pays { item: ItemId, count: u32, have: u32 },
    Gets { item: ItemId, count: u32 },
}

impl ChoiceChip {
    pub fn needs(world: &World, player: Entity, requires: &Condition) -> Vec<ChoiceChip> {
        condition::parts(requires)
            .into_iter()
            .map(|part| ChoiceChip::Needs {
                what: condition::describe(world.resource::<Content>(), part),
                met: condition::holds(world, player, part),
            })
            .collect()
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ConversationRequest {
    Pick { step: ConversationStep, choice: u32 },
    Leave { step: ConversationStep },
}

pub type CardPick = Arc<dyn Fn(&mut World, Entity) -> Result<Answer, String> + Send + Sync>;

pub struct CardOffer {
    pub label: LineText,
    pub tag: ChoiceTag,
    pub icon: Option<ImageRef>,
    pub chips: Vec<ChoiceChip>,
    pub locked: bool,
    pub warn: Option<String>,
    pub pick: CardPick,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Heard(HashSet<String>);

impl Heard {
    pub fn has(&self, script: &str, node: &str) -> bool {
        self.0.contains(&heard_key(script, node))
    }

    pub fn hear(&mut self, script: &str, node: &str) {
        self.0.insert(heard_key(script, node));
    }
}

pub fn heard(world: &World, player: Entity, script: &str, node: &str) -> bool {
    world
        .get::<Heard>(player)
        .is_some_and(|heard| heard.has(script, node))
}

pub struct Revealable {
    pub needs: bool,
    pub costs: bool,
    pub gains: bool,
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChoiceReveal {
    Locked,
    Needs,
    Costs,
    Gains,
}

impl RevealDef for ChoiceReveal {
    fn shows_while_locked(self) -> bool {
        self == ChoiceReveal::Locked
    }

    fn name(self) -> &'static str {
        self.into()
    }
}

pub fn check_reveal(reveal: &[ChoiceReveal], revealable: Revealable) -> Result<(), ScriptFault> {
    for kind in reveal {
        let (present, what) = match kind {
            ChoiceReveal::Locked | ChoiceReveal::Needs => (revealable.needs, "needs"),
            ChoiceReveal::Costs => (revealable.costs, "costs"),
            ChoiceReveal::Gains => (revealable.gains, "gains"),
        };
        if !present {
            return Err(ScriptFault::new(
                "nothing to reveal",
                format!("reveals {}, {what} nothing", kind.name()),
            ));
        }
    }
    script::check_reveals_unique(reveal)
}

pub fn show(world: &mut World, flow: FlowKey, subject: Entity, view: DialogueView<RiftScripting>) {
    if !begin(world, flow, subject) {
        return;
    }
    let script = view.script.to_owned();
    world
        .entity_mut(subject)
        .entry::<Heard>()
        .or_default()
        .into_mut()
        .hear(&script, view.node);
    let lines: Vec<SpokenLine> = view
        .lines
        .iter()
        .map(|(line, values)| SpokenLine {
            by: line.by,
            gesture: line.gesture.clone(),
            camera: line.camera,
            text: LineText::spoken(&line.text, values),
        })
        .collect();
    let bystanders = bystanders(world, subject, &lines);
    for line in &lines {
        record_line(world, subject, line);
    }
    let offers: Vec<Offer> = view
        .choices
        .iter()
        .map(|choice| Offer {
            view: choice_view(world, subject, choice),
            pick: Pick::Choice {
                index: choice.index,
                requires: choice.requires.cloned(),
                costs: revealed_costs(choice),
            },
        })
        .collect();
    let panel = view
        .choices
        .iter()
        .flat_map(|choice| choice.leads.iter())
        .find_map(|effect| match effect {
            RiftEffect::Decide(decide) => Some(ConversationPanel::QuestDecision(decide.quest)),
            _ => None,
        });
    let step = next_step(world);
    if let Some(mut session) = world.get_mut::<ConversationSession>(subject) {
        session.step = step;
        session.node = view.node.to_owned();
        session.camera = view.camera;
        session.bystanders = bystanders;
        session.lines = lines;
        session.offers = offers;
        session.panel = panel;
        session.on_leave = Answer::Left;
        session.answered = false;
        session.refused = None;
        session.remark = None;
    }
    refresh_view(world, subject);
}

pub fn card(
    world: &mut World,
    flow: FlowKey,
    subject: Entity,
    offers: Vec<CardOffer>,
    on_leave: Answer,
    panel: Option<ConversationPanel>,
    camera: DialogueCamera,
) -> bool {
    if !begin(world, flow, subject) {
        return false;
    }
    let step = next_step(world);
    if let Some(mut session) = world.get_mut::<ConversationSession>(subject) {
        session.step = step;
        session.camera = camera;
        session.panel = panel;
        session.offers = offers
            .into_iter()
            .map(|offer| Offer {
                view: ChoiceView {
                    label: offer.label,
                    tag: offer.tag,
                    icon: offer.icon.map(|icon| icon.path()),
                    chips: offer.chips,
                    locked: offer.locked,
                    warn: offer.warn,
                },
                pick: Pick::Card(offer.pick),
            })
            .collect();
        session.on_leave = on_leave;
        session.answered = false;
        session.refused = None;
    }
    refresh_view(world, subject);
    true
}

pub fn close(world: &mut World, flow: FlowKey, subject: Entity, ending: BoxEnding) {
    let Some(session) = world.get::<ConversationSession>(subject) else {
        return;
    };
    if session.flow != flow {
        return;
    }
    close_for(world, subject);
    if ending == BoxEnding::Gone {
        tell_ended(world, subject, "they're gone");
    }
}

pub fn remark(world: &mut World, player: Entity, to: Entity, line: &ScriptLine) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    if session.with != Some(to) {
        return;
    }
    let nth = session.remark.as_ref().map_or(0, |remark| remark.nth) + 1;
    let line = SpokenLine {
        by: line.by,
        gesture: line.gesture.clone(),
        camera: line.camera,
        text: condition::spoken(world, player, &line.text),
    };
    record_line(world, player, &line);
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.remark = Some(Remark { nth, line });
    }
    refresh_view(world, player);
}

pub fn in_conversation(world: &World, player: Entity) -> bool {
    world.get::<ConversationSession>(player).is_some()
}

pub type Sessions = QueryState<(Entity, &'static ConversationSession)>;

/// Everyone a player talks with: whoever the conversation is with, and whoever else says its
/// lines.
pub fn counterparts<'a>(
    sessions: impl Iterator<Item = (Entity, &'a ConversationSession)>,
) -> Vec<Entity> {
    sessions
        .flat_map(|(_, session)| session.with.into_iter().chain(session.bystanders.clone()))
        .collect()
}

/// The players whose conversation is with `whom`.
pub fn talking_with<'a>(
    sessions: impl Iterator<Item = (Entity, &'a ConversationSession)>,
    whom: Entity,
) -> Vec<Entity> {
    sessions
        .filter(|(_, session)| session.with == Some(whom))
        .map(|(player, _)| player)
        .collect()
}

/// Everyone in a conversation: each player talking and everyone they talk with.
pub fn parties<'a>(
    sessions: impl Iterator<Item = (Entity, &'a ConversationSession)>,
) -> Vec<Entity> {
    sessions
        .flat_map(|(player, session)| {
            [player]
                .into_iter()
                .chain(session.with)
                .chain(session.bystanders.clone())
        })
        .collect()
}

/// Who each one in a conversation turns to, as (who, to whom): a player and whoever they talk
/// with to each other, and whoever else says its lines to the player.
pub fn turned<'a>(
    sessions: impl Iterator<Item = (Entity, &'a ConversationSession)>,
) -> Vec<(Entity, Entity)> {
    sessions
        .flat_map(|(player, session)| {
            session
                .with
                .map(|with| (player, with))
                .into_iter()
                .chain(session.with.map(|with| (with, player)))
                .chain(session.bystanders.iter().map(move |&other| (other, player)))
        })
        .collect()
}

pub fn flow_of(world: &World, player: Entity) -> Option<FlowKey> {
    world
        .get::<ConversationSession>(player)
        .map(|session| session.flow)
}

pub fn counterpart_of(world: &World, player: Entity) -> Option<Entity> {
    world
        .get::<ConversationSession>(player)
        .and_then(|session| session.with)
}

pub fn requests(world: &mut World) {
    player_requests::<ConversationRequest>(world, false, |world, player, request| {
        match request {
            ConversationRequest::Pick { step, choice } => pick(world, player, step, choice),
            ConversationRequest::Leave { step } => {
                if world
                    .get::<ConversationSession>(player)
                    .is_some_and(|session| session.step == step)
                {
                    leave(world, player);
                }
            }
        }
        Ok(())
    });
}

pub fn hold(world: &mut World, sessions: &mut QueryState<Entity, With<ConversationSession>>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    for player in players {
        if let Some(reason) = broken(world, player) {
            leave(world, player);
            tell_ended(world, player, reason);
        }
    }
}

pub fn refresh(world: &mut World, sessions: &mut QueryState<Entity, With<ConversationSession>>) {
    let players: Vec<Entity> = sessions.iter(world).collect();
    for player in players {
        let Some(session) = world.get::<ConversationSession>(player) else {
            continue;
        };
        let flow = session.flow;
        let choices: Vec<(usize, usize)> = session
            .offers
            .iter()
            .enumerate()
            .filter_map(|(index, offer)| match offer.pick {
                Pick::Choice { index: choice, .. } => Some((index, choice)),
                Pick::Card(_) => None,
            })
            .collect();
        for (index, choice) in choices {
            let locked = script::run::choice_state::<RiftScripting>(world, flow, choice)
                == ChoiceState::Locked;
            if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
                session.offers[index].view.locked = locked;
            }
        }
        refresh_view(world, player);
    }
}

pub fn speaker_name(world: &World, player: Entity, who: Speaker) -> Option<String> {
    let content = world.resource::<Content>();
    match who {
        Speaker::Npc(npc) => Some(npc.get(content).name.to_owned()),
        Speaker::Object => counterpart_of(world, player)
            .and_then(|with| world.get::<Name>(with))
            .map(|name| name.name.clone()),
        Speaker::Narrator | Speaker::Player => None,
    }
}

#[derive(Component)]
pub struct ConversationSession {
    step: ConversationStep,
    flow: FlowKey,
    node: String,
    with: Option<Entity>,
    /// Whoever says the lines on show beside the one the conversation is with.
    bystanders: Vec<Entity>,
    camera: DialogueCamera,
    tether: Option<Tether>,
    lines: Vec<SpokenLine>,
    offers: Vec<Offer>,
    panel: Option<ConversationPanel>,
    on_leave: Answer,
    answered: bool,
    refused: Option<RefusedPick>,
    refusals: u32,
    remark: Option<Remark>,
}

struct Offer {
    view: ChoiceView,
    pick: Pick,
}

#[derive(Clone)]
enum Pick {
    Choice {
        index: usize,
        requires: Option<Condition>,
        costs: Vec<ItemStack>,
    },
    Card(CardPick),
}

#[derive(Resource, Default)]
pub(crate) struct StepCounter(u32);

/// The npcs in sight of `player` that say `lines` beside the one the conversation is with: of each
/// kind that speaks, the nearest.
fn bystanders(world: &mut World, player: Entity, lines: &[SpokenLine]) -> Vec<Entity> {
    let Some(at) = world.get::<Position>(player).map(|position| position.pos) else {
        return Vec::new();
    };
    let with = counterpart_of(world, player);
    let sight = visibility::sight(world);
    let mut speakers: Vec<_> = lines
        .iter()
        .filter_map(|line| match line.by {
            Speaker::Npc(npc) => Some(npc),
            _ => None,
        })
        .filter(|&npc| {
            with.and_then(|with| world.get::<Npc>(with))
                .is_none_or(|with| with.def != npc)
        })
        .collect();
    speakers.sort_unstable();
    speakers.dedup();
    let bodies: Vec<(Entity, &Npc, &Position)> = world
        .query::<(Entity, &Npc, &Position)>()
        .iter(world)
        .collect();
    speakers
        .into_iter()
        .filter_map(|npc| {
            bodies
                .iter()
                .filter(|(body, seen, position)| {
                    seen.def == npc
                        && position.pos.distance(at) <= sight
                        && !stat::is_dead(world, *body)
                })
                .min_by(|(.., a), (.., b)| a.pos.distance(at).0.total_cmp(&b.pos.distance(at).0))
                .map(|(body, ..)| *body)
        })
        .collect()
}

fn heard_key(script: &str, node: &str) -> String {
    format!("{script}#{node}")
}

fn begin(world: &mut World, flow: FlowKey, subject: Entity) -> bool {
    if let Some(session) = world.get::<ConversationSession>(subject) {
        if session.flow == flow {
            return true;
        }
        let old = session.flow;
        close_for(world, subject);
        world
            .resource_mut::<RiftInstances>()
            .answer(old, Answer::Left);
    }
    if stat::is_dead(world, subject) {
        return false;
    }
    let with = flow.owner.entity();
    let reach = world
        .resource::<RiftInstances>()
        .entries(flow.owner, EventKind::Interacted)
        .filter_map(|event| match event {
            RiftEvent::Interacted { reach, .. } => Some(reach.0),
            _ => None,
        })
        .reduce(f32::max)
        .map_or_else(|| visibility::sight(world), Meters);
    let tether = with.and_then(|with| Tether::around(world, subject, reach, Some(with)));
    reach::forget(world, subject);
    let step = next_step(world);
    world.entity_mut(subject).insert((
        ConversationSession {
            step,
            flow,
            node: String::new(),
            with,
            bystanders: Vec::new(),
            camera: DialogueCamera::default(),
            tether,
            lines: Vec::new(),
            offers: Vec::new(),
            panel: None,
            on_leave: Answer::Left,
            answered: false,
            refused: None,
            refusals: 0,
            remark: None,
        },
        CommandLock,
    ));
    let name = with
        .and_then(|with| world.get::<Name>(with))
        .map(|name| name.name.clone());
    history::record(
        world,
        subject,
        HistoryEntry::of(HistoryTopic::Talk, LineText::default())
            .by(name)
            .mark(Some(HistoryMark::Began)),
    );
    true
}

/// Ends the conversation `player` is in, if any, telling them `reason`: a blow landing on them,
/// which no talk survives.
pub fn interrupt(world: &mut World, player: Entity, reason: &str) {
    if world.get::<ConversationSession>(player).is_none() {
        return;
    }
    leave(world, player);
    tell_ended(world, player, reason);
}

pub fn close_for(world: &mut World, player: Entity) {
    if let Some(session) = world.entity_mut(player).take::<ConversationSession>() {
        world
            .resource_mut::<RiftInstances>()
            .box_closed(session.flow);
    }
    world
        .entity_mut(player)
        .remove::<(Conversation, CommandLock)>();
}

fn leave(world: &mut World, player: Entity) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    let (flow, answer) = (session.flow, session.on_leave.clone());
    close_for(world, player);
    world.resource_mut::<RiftInstances>().answer(flow, answer);
}

fn tell_ended(world: &mut World, player: Entity, reason: &str) {
    notification::notify(
        world,
        player,
        None,
        Notification::new(
            NotificationKind::Error {
                sfx: Some(InterfaceSound::UiClose.id(world.resource::<Content>())),
            },
            LineText::plain(format!("Conversation ended: {reason}")),
        ),
    );
}

fn record_line(world: &mut World, player: Entity, line: &SpokenLine) {
    let by = speaker_name(world, player, line.by);
    history::record(
        world,
        player,
        HistoryEntry::of(HistoryTopic::Talk, line.text.clone()).by(by),
    );
}

fn choice_view(world: &World, player: Entity, choice: &ChoiceOffer<RiftScripting>) -> ChoiceView {
    let inventory = world.get::<Inventory>(player);
    let have = |item| inventory.map_or(0, |inventory| inventory.count(item));
    let mut chips = Vec::new();
    if choice.reveal.contains(&ChoiceReveal::Needs)
        && let Some(requires) = choice.requires
    {
        chips.extend(ChoiceChip::needs(world, player, requires));
    }
    let items = choice.leads.iter().find_map(|effect| match effect {
        RiftEffect::Items(items) => Some(items),
        _ => None,
    });
    if let Some(items) = items {
        if choice.reveal.contains(&ChoiceReveal::Costs) {
            for stack in items.named_take() {
                chips.push(ChoiceChip::Pays {
                    item: stack.item,
                    count: stack.count,
                    have: have(stack.item),
                });
            }
        }
        if choice.reveal.contains(&ChoiceReveal::Gains) {
            for stack in items.named_give() {
                chips.push(ChoiceChip::Gets {
                    item: stack.item,
                    count: stack.count,
                });
            }
        }
    }
    ChoiceView {
        label: LineText::spoken(choice.label, &choice.label_values),
        tag: ChoiceTag::Dialogue,
        icon: None,
        chips,
        locked: choice.locked,
        warn: choice.warn.map(str::to_owned),
    }
}

fn refresh_view(world: &mut World, player: Entity) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    let view = Conversation {
        step: session.step,
        node: session.node.clone(),
        with: session.with,
        camera: session.camera,
        lines: session.lines.clone(),
        choices: session
            .offers
            .iter()
            .map(|offer| offer.view.clone())
            .collect(),
        panel: session.panel,
        refused: session.refused.clone(),
        remark: session.remark.clone(),
    };
    if world.get::<Conversation>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

fn pick(world: &mut World, player: Entity, step: ConversationStep, choice: u32) {
    let Some(session) = world.get::<ConversationSession>(player) else {
        return;
    };
    if session.step != step || session.answered {
        return;
    }
    let Some(offer) = session.offers.get(choice as usize) else {
        return;
    };
    let flow = session.flow;
    let label = offer.view.label.clone();
    let picked = offer.pick.clone();
    if let Some(reason) = broken(world, player) {
        leave(world, player);
        tell_ended(world, player, reason);
        return;
    }
    let said = HistoryEntry::of(HistoryTopic::Talk, label)
        .by(world.get::<Name>(player).map(|name| name.name.clone()))
        .mark(Some(HistoryMark::You));
    history::record(world, player, said);
    let answer = match picked {
        Pick::Choice {
            index,
            requires,
            costs,
        } => match script::run::choice_state::<RiftScripting>(world, flow, index) {
            ChoiceState::Open => affordable(world, player, &costs).map(|()| Answer::Pick(index)),
            ChoiceState::Locked => Err(unmet(world, player, requires.as_ref())),
            ChoiceState::Gone => Err(world.resource::<GameSettings>().words.no_longer.clone()),
        },
        Pick::Card(card) => card(world, player),
    };
    match answer {
        Ok(answer) => {
            if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
                session.answered = true;
            }
            world.resource_mut::<RiftInstances>().answer(flow, answer);
        }
        Err(reason) => refuse(world, player, choice, reason),
    }
}

fn revealed_costs(choice: &ChoiceOffer<RiftScripting>) -> Vec<ItemStack> {
    if !choice.reveal.contains(&ChoiceReveal::Costs) {
        return Vec::new();
    }
    choice
        .leads
        .iter()
        .find_map(|effect| match effect {
            RiftEffect::Items(items) => Some(items.named_take()),
            _ => None,
        })
        .unwrap_or_default()
}

fn affordable(world: &World, player: Entity, costs: &[ItemStack]) -> Result<(), String> {
    if costs.is_empty() {
        return Ok(());
    }
    let content = world.resource::<Content>();
    world
        .get::<Inventory>(player)
        .cloned()
        .unwrap_or_else(Inventory::empty)
        .exchange(content, costs, &[])
        .map_err(|refusal| refusal.describe(content))
}

fn unmet(world: &World, player: Entity, requires: Option<&Condition>) -> String {
    condition::first_unmet(world, player, requires).map_or_else(
        || world.resource::<GameSettings>().words.locked.clone(),
        |unmet| format!("Needs {unmet}"),
    )
}

fn refuse(world: &mut World, player: Entity, choice: u32, reason: String) {
    notification::error(world, player, reason);
    if let Some(mut session) = world.get_mut::<ConversationSession>(player) {
        session.refusals += 1;
        session.refused = Some(RefusedPick {
            nth: session.refusals,
            choice,
        });
    }
    refresh_view(world, player);
}

pub(crate) fn talking(world: &World, player: Entity) -> Option<AttentionId> {
    in_conversation(world, player).then_some(AttentionMark::Talking.id(world.resource::<Content>()))
}

fn broken(world: &World, player: Entity) -> Option<&'static str> {
    let session = world.get::<ConversationSession>(player)?;
    if stat::is_dead(world, player) {
        return Some("you died");
    }
    if let Some(with) = session.with {
        if world.get_entity(with).is_err() || stat::is_dead(world, with) {
            return Some("they're gone");
        }
        if world.get::<Attitude>(with) == Some(&Attitude::Hostile) {
            return Some("they turned hostile");
        }
    }
    session
        .tether
        .filter(|tether| !tether.holds(world, player))
        .map(|_| "you walked out of reach")
}

fn next_step(world: &mut World) -> ConversationStep {
    let mut counter = world.resource_mut::<StepCounter>();
    counter.0 = counter.0.wrapping_add(1);
    ConversationStep(counter.0)
}
