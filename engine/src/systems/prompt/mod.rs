pub mod tray;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, SchemaMark};
use crate::core::script::{
    Answer, Arrow, Compiler, EffectDef, EffectStep, REFUSED, ScriptFault, ValueType,
};
use crate::systems::dialogue;
use crate::systems::dialogue::camera::DialogueCamera;
use crate::systems::item::{ExchangeRefusal, Inventory, ItemStack};
use crate::systems::player::player_requests;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, FlowKey, RiftInstances, RiftScripting, effect as effects,
};
use crate::systems::text::{self, LineText, Span};

pub const SUBMITTED: &str = "submitted";
pub const CLOSED: &str = "closed";

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
    pub who: Option<Condition>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<Span>,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub slots: u32,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub submitted: Arrow,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub closed: Arrow,
}

impl EffectDef<RiftScripting> for ItemPrompt {
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
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        check_slots(self.slots)?;
        let who = effects::aim(cx, &self.who)?;
        let values = text::values(&self.text)
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
                text: (!spans.is_empty()).then(|| LineText::spoken(&spans, &spoken)),
                slots,
                camera,
            };
            if open(world, player, flow, asked, PromptKind::Offer) {
                EffectStep::Pause
            } else {
                EffectStep::Done
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Prompts]
    }
}

/// Takes from the bag what its subject last submitted to a prompt, all of it or nothing.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct TakeOffer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef<RiftScripting> for TakeOffer {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!("done" => done, REFUSED => refused);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let refused = self.refused.clone();
        Ok(Box::new(move |world, _, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return effects::refuse(world, cx, &refused, "Nobody offered anything");
            };
            let outcome = take_offer(world, player);
            effects::settle(world, cx, &refused, outcome)
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::TakesOffer]
    }
}

/// The turn of a prompt a request answers, so a second press or a late packet answers a turn
/// that is already over.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromptStep(pub u32);

/// What a prompt shows its player: its words, how many slots it has, and what it is for.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PromptView {
    pub step: PromptStep,
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
        step: PromptStep,
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
}

#[derive(Component)]
pub struct PromptSession {
    flow: FlowKey,
    step: PromptStep,
    slots: u32,
}

#[derive(Resource, Default)]
pub(crate) struct PromptSteps(u32);

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
        Answer::Output(CLOSED),
        None,
        asked.camera,
    ) {
        return false;
    }
    let step = next_step(world);
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
    step: PromptStep,
    offer: &[ItemStack],
) -> Result<Option<FlowKey>, String> {
    let Some(session) = world.get::<PromptSession>(player) else {
        return Ok(None);
    };
    if session.step != step || offer.len() > session.slots as usize {
        return Ok(None);
    }
    let flow = session.flow;
    let content = world.resource::<Content>().clone();
    let covered = world
        .get::<Inventory>(player)
        .cloned()
        .unwrap_or_else(Inventory::empty)
        .exchange(&content, offer, &[]);
    if let Err(refusal) = covered {
        return Err(refusal.describe(&content));
    }
    if offer.iter().any(|stack| stack.count == 0) {
        return Ok(None);
    }
    Ok(Some(flow))
}

pub fn requests(world: &mut World) {
    player_requests::<PromptRequest>(world, false, |world, player, request| {
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
            .resource_mut::<RiftInstances>()
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
    let step = next_step(world);
    if let Some(mut session) = world.get_mut::<PromptSession>(player) {
        session.step = step;
    }
    if let Some(mut view) = world.get_mut::<PromptView>(player) {
        view.step = step;
    }
}

/// Closes the prompt of whoever is no longer in the conversation that holds it.
pub fn hold(world: &mut World, sessions: &mut QueryState<(Entity, &'static PromptSession)>) {
    let over: Vec<Entity> = sessions
        .iter(world)
        .filter(|(player, session)| dialogue::flow_of(world, *player) != Some(session.flow))
        .map(|(player, _)| player)
        .collect();
    for player in over {
        close(world, player);
    }
}

pub fn close(world: &mut World, player: Entity) {
    world
        .entity_mut(player)
        .remove::<(PromptSession, PromptView)>();
}

fn next_step(world: &mut World) -> PromptStep {
    let mut steps = world.resource_mut::<PromptSteps>();
    steps.0 = steps.0.wrapping_add(1);
    PromptStep(steps.0)
}

fn take_offer(world: &mut World, player: Entity) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let offer = world.get::<Offer>(player).cloned().unwrap_or_default();
    world
        .get_mut::<Inventory>(player)
        .ok_or(ExchangeRefusal::NoRoom { slots: 0 })
        .and_then(|mut bag| bag.exchange(&content, &offer.0, &[]))
        .map_err(|refusal| refusal.describe(&content))?;
    world.entity_mut(player).insert(Offer::default());
    Ok(())
}
