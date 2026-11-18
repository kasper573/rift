pub mod query;
pub mod tray;

use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::Vocabulary;
use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, SchemaMark};
use crate::core::script::DialogueCamera;
use crate::core::script::{
    Answer, Arrow, CLOSED, Compiler, DONE, EffectDef, EffectStep, REFUSED, ScriptFault, ValueType,
};
use crate::core::script::{EffectCall, Expr, FlowKey, Instances};
use crate::core::text::LineText;
use crate::systems::aim as effects;
use crate::systems::dialogue;
use crate::systems::dialogue::span::ScriptSpan;
use crate::systems::item::{self, Inventory, ItemStack};
use crate::systems::messages::{self, messages};
use crate::systems::player::RequestNeeds;
use crate::systems::player::player_requests;
use crate::systems::window::{OpenWindow, WindowStep, WindowSteps};

messages! {
    /// What the game says in an item prompt.
    pub struct PromptMessages {
        /// The button submitting what a player placed in a prompt.
        confirm,
        /// The heading over the items a player placed in a prompt.
        placed,
        /// The heading over the player's bag beside a prompt.
        your_inventory,
        /// The button that leaves a prompt.
        leave,
        /// What a bag's slot says of an item that is placed in a prompt.
        placed_note,
        /// What a player is told as a prompt takes an offer from nobody.
        nobody_offered,
    }
}

pub const SUBMITTED: &str = "submitted";

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<PromptView>()
        .add_client_message::<PromptRequest>();
}

/// Shows `slots` empty item slots until what was placed in them is submitted or the prompt is
/// left. What was submitted is its subject's offer, which conditions read with `offered`; nothing
/// leaves the bag until `take_offer` takes it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct ItemPrompt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<ScriptSpan>,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub slots: u32,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub submitted: Arrow,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub closed: Arrow,
}

impl EffectDef for ItemPrompt {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(SUBMITTED => submitted, CLOSED => closed);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        check_slots(self.slots)?;
        let who = effects::aim(cx, &self.who)?;
        let values = ScriptSpan::values(&self.text)
            .into_iter()
            .map(|value| cx.expr(value, ValueType::Text))
            .collect::<Result<Vec<_>, _>>()?;
        let (title, spans, slots) = (self.title.clone(), self.text.clone(), self.slots);
        let camera = cx.camera();
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Done;
            };
            let spoken: Vec<_> = values.iter().map(|value| value(world, cx)).collect();
            let asked = Asked {
                title: title.clone(),
                text: (!spans.is_empty()).then(|| ScriptSpan::said(&spans, &spoken)),
                slots,
                camera,
                on_leave: CLOSED,
            };
            if open(world, player, flow, asked, PromptKind::Offer) {
                EffectStep::Pause
            } else {
                EffectStep::Done
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Prompts)]
    }
}

/// Takes from the bag what its subject last submitted to a prompt, all of it or nothing.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct TakeOffer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef for TakeOffer {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let refused = self.refused.clone();
        Ok(Box::new(move |world, _, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                let said = LineText::from(
                    messages::of(world.resource::<Content>())
                        .prompt
                        .nobody_offered(),
                );
                return effects::refuse(world, cx, &refused, said);
            };
            match take_offer(world, player) {
                Ok(()) => EffectStep::Done,
                Err(why) => effects::refuse(world, cx, &refused, why),
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(TakesOffered)]
    }
}

/// What a prompt shows its player: its words, how many slots it has, and what it is for.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PromptView {
    pub step: WindowStep,
    pub title: String,
    pub text: Option<LineText>,
    pub slots: u32,
    pub kind: PromptKind,
}

/// Whose prompt it is: the script's that asked, which is handed what is submitted, or a system's
/// that opened it inside a panel of its own and takes what is placed itself.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Offer,
    Hosted,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PromptRequest {
    Submit {
        step: WindowStep,
        offer: Vec<ItemStack>,
    },
}

/// What a character last submitted to a prompt.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Offer(pub Vec<ItemStack>);

impl Offer {
    /// How many of `item` were offered, or how many items in all without one.
    pub fn count(&self, item: Option<crate::systems::item::ItemId>) -> u32 {
        self.0
            .iter()
            .filter(|stack| item.is_none_or(|item| stack.item == item))
            .fold(0, |total, stack| total.saturating_add(stack.count))
    }
}

/// What an effect asks a prompt to show.
pub struct Asked {
    pub title: String,
    pub text: Option<LineText>,
    pub slots: u32,
    pub camera: DialogueCamera,
    /// What the flow that asked is answered when the player leaves the prompt.
    pub on_leave: &'static str,
}

#[derive(Component)]
pub struct PromptSession {
    flow: FlowKey,
    step: WindowStep,
    slots: u32,
}

pub fn check_slots(slots: u32) -> Result<(), ScriptFault> {
    if slots == 0 {
        return Err(ScriptFault::new(
            "no slots",
            "a prompt of one slot or more, got 0",
        ));
    }
    Ok(())
}

/// Opens a prompt of `asked` for `player` in the conversation `flow` holds, forgetting what they
/// offered before; false when no conversation can hold it.
pub fn open(
    world: &mut World,
    player: Entity,
    flow: FlowKey,
    asked: Asked,
    kind: PromptKind,
) -> bool {
    if !dialogue::card(
        world,
        flow,
        player,
        Vec::new(),
        Answer::Output(asked.on_leave),
        None,
        asked.camera,
    ) {
        return false;
    }
    let step = world.resource_mut::<WindowSteps>().deal();
    world.entity_mut(player).insert((
        PromptSession {
            flow,
            step,
            slots: asked.slots,
        },
        PromptView {
            step,
            title: asked.title,
            text: asked.text,
            slots: asked.slots,
            kind,
        },
        Offer::default(),
    ));
    true
}

/// The offer `player` submits for the turn `step` of their prompt, when the prompt is theirs to
/// answer and their bag covers every stack of it; the prompt is over once it is taken up.
pub fn submitted(
    world: &mut World,
    player: Entity,
    step: WindowStep,
    offer: &[ItemStack],
) -> Result<Option<FlowKey>, LineText> {
    let Some(session) = world.get::<PromptSession>(player) else {
        return Ok(None);
    };
    if session.step != step || offer.len() > session.slots as usize {
        return Ok(None);
    }
    let flow = session.flow;
    let content = world.resource::<Content>().clone();
    let covered = Inventory::of(world, player).exchange(&content, offer, &[]);
    if let Err(refusal) = covered {
        return Err(refusal.line(&content));
    }
    if offer.iter().any(|stack| stack.count == 0) {
        return Ok(None);
    }
    Ok(Some(flow))
}

pub fn requests(world: &mut World) {
    player_requests::<PromptRequest>(world, RequestNeeds::Nothing, |world, player, request| {
        let PromptRequest::Submit { step, offer } = request;
        if hosted(world, player) {
            return Ok(());
        }
        let flow = match submitted(world, player, step, &offer) {
            Ok(Some(flow)) => flow,
            Ok(None) => return Ok(()),
            Err(reason) => {
                ask_again(world, player);
                return Err(reason);
            }
        };
        close(world, player);
        world.entity_mut(player).insert(Offer(offer));
        world
            .resource_mut::<Instances>()
            .answer(flow, Answer::Output(SUBMITTED));
        Ok(())
    });
}

/// Whether the prompt before `player` is one a system opened inside a panel of its own.
pub fn hosted(world: &World, player: Entity) -> bool {
    world
        .get::<PromptView>(player)
        .is_some_and(|view| view.kind == PromptKind::Hosted)
}

/// Begins a new turn of `player`'s prompt, so whoever submitted knows the turn before was answered.
pub fn ask_again(world: &mut World, player: Entity) {
    let step = world.resource_mut::<WindowSteps>().deal();
    if let Some(mut session) = world.get_mut::<PromptSession>(player) {
        session.step = step;
    }
    if let Some(mut view) = world.get_mut::<PromptView>(player) {
        view.step = step;
    }
}

impl OpenWindow for PromptSession {
    fn kept(&self, world: &World, player: Entity) -> bool {
        dialogue::flow_of(world, player) == Some(self.flow)
    }

    fn close(world: &mut World, player: Entity) {
        close(world, player);
    }
}

pub fn close(world: &mut World, player: Entity) {
    world
        .entity_mut(player)
        .remove::<(PromptSession, PromptView)>();
}

fn take_offer(world: &mut World, player: Entity) -> Result<(), LineText> {
    let offer = world.get::<Offer>(player).cloned().unwrap_or_default();
    item::effects::exchange(world, player, &offer.0, &[])?;
    world.entity_mut(player).insert(Offer::default());
    Ok(())
}

/// A script that may open a prompt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prompts;

impl ReachDef for Prompts {}

/// A script that may take what was offered to it in a prompt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakesOffered;

impl ReachDef for TakesOffered {
    fn unmet(&self, all: &[Reach]) -> Option<ScriptFault> {
        offered_without_prompt(all)
    }
}

/// A script that may read what was offered to it in a prompt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadsOffer;

impl ReachDef for ReadsOffer {
    fn unmet(&self, all: &[Reach]) -> Option<ScriptFault> {
        offered_without_prompt(all)
    }
}

fn offered_without_prompt(all: &[Reach]) -> Option<ScriptFault> {
    (!all.iter().any(Reach::is::<Prompts>)).then(|| {
        ScriptFault::new(
            "offered without a prompt",
            "a script that opens a prompt before it reads or takes what was offered to it",
        )
    })
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .effect::<ItemPrompt>("item_prompt")
        .effect::<TakeOffer>("take_offer");
}
