//! How effects aim at actors and players, reach players of other worlds, and refuse.

use bevy_ecs::prelude::*;

use crate::core::script::{
    Answer, Arrow, Compiler, EffectStep, Eval, Expr, FlowCx, FlowKey, GlobalPlayerId, Instances,
    REFUSED, ScriptFault, Value, ValueType,
};
use crate::core::text::LineText;
use crate::systems::notification;
use crate::systems::player::errand::{self, Errand, ErrandOutcome};
use crate::systems::player::{ClientId, PlayedBy, Players};
use crate::systems::stat;

pub fn aim<'c>(cx: &mut Compiler<'c>, who: &'c Option<Expr>) -> Result<Eval, ScriptFault> {
    cx.actor(who.as_ref())
}

pub fn aimed(world: &World, cx: &FlowCx, who: &Eval) -> Option<Entity> {
    actor(world, cx, who)
}

pub fn aimed_character(world: &World, cx: &FlowCx, who: &Eval) -> Option<Entity> {
    character(world, cx, who)
}

pub fn aim_all<'c>(cx: &mut Compiler<'c>, who: &'c Option<Expr>) -> Result<Eval, ScriptFault> {
    cx.actors(who.as_ref())
}

pub fn aimed_all(world: &World, cx: &FlowCx, who: &Eval) -> Vec<Entity> {
    actors(world, cx, who)
}

/// Who an effect acts on: someone standing in this world, a player whose character stands in
/// another, or nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aimed {
    Here(Entity),
    Elsewhere(ClientId),
    Nobody,
}

/// As `aim`, for an effect that also reaches a global player.
pub fn aim_anywhere<'c>(cx: &mut Compiler<'c>, who: &'c Option<Expr>) -> Result<Eval, ScriptFault> {
    cx.actor_anywhere(who.as_ref())
}

pub fn aimed_anywhere(world: &World, cx: &FlowCx, who: &Eval) -> Aimed {
    match who(world, cx) {
        Value::GlobalPlayer(player) => located(world, player),
        value => value
            .as_actor()
            .filter(|&who| stat::stands(world, who))
            .map_or(Aimed::Nobody, Aimed::Here),
    }
}

/// As `aimed_anywhere`, for an effect only a player's character takes.
pub fn aimed_character_anywhere(world: &World, cx: &FlowCx, who: &Eval) -> Aimed {
    match aimed_anywhere(world, cx, who) {
        Aimed::Here(actor) if !is_character(world, actor) => Aimed::Nobody,
        aimed => aimed,
    }
}

/// As `aim_all`, for an effect that also reaches a global player.
pub fn aim_all_anywhere<'c>(
    cx: &mut Compiler<'c>,
    who: &'c Option<Expr>,
) -> Result<Eval, ScriptFault> {
    cx.actors_anywhere(who.as_ref())
}

/// Everyone an effect on several acts on: actors standing in this world, or one player whose
/// character stands in another. A set of actors never holds a player of another world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AimedAll {
    Here(Vec<Entity>),
    Elsewhere(ClientId),
}

pub fn aimed_all_anywhere(world: &World, cx: &FlowCx, who: &Eval) -> AimedAll {
    match who(world, cx) {
        Value::GlobalPlayer(player) => match located(world, player) {
            Aimed::Here(actor) => AimedAll::Here(vec![actor]),
            Aimed::Elsewhere(client) => AimedAll::Elsewhere(client),
            Aimed::Nobody => AimedAll::Here(Vec::new()),
        },
        value => AimedAll::Here(
            value
                .as_actors()
                .into_iter()
                .filter(|&who| stat::stands(world, who))
                .collect(),
        ),
    }
}

/// How an effect that reaches a player anywhere goes on: the output it takes once done, the
/// arrow of its `refused` output when it has one, and what it says when it finds nobody, when
/// that is worth saying.
#[derive(Clone, Copy)]
pub struct Reaching<'a> {
    pub done: &'static str,
    pub refused: Option<&'a Arrow>,
    pub nobody: Option<&'a str>,
}

impl Reaching<'static> {
    /// An effect with no `refused` output, which goes on in silence when it finds nobody.
    pub fn quietly(done: &'static str) -> Reaching<'static> {
        Reaching {
            done,
            refused: None,
            nobody: None,
        }
    }
}

/// Does `deed` to whoever an effect aimed at and says how the effect goes on. Someone here has
/// it done at once. A player elsewhere has it done where their character stands, once the
/// server's worlds confer: the flow waits for word of how it went, and then goes on as it would
/// have here.
pub fn reach(
    world: &mut World,
    flow: FlowKey,
    cx: &FlowCx,
    aimed: Aimed,
    reaching: Reaching,
    deed: impl FnOnce(&mut World, Entity) -> Result<(), LineText> + Send + Sync + 'static,
) -> EffectStep {
    let outcome = match aimed {
        Aimed::Here(actor) => match deed(world, actor) {
            Ok(()) => ErrandOutcome::Done,
            Err(reason) => ErrandOutcome::Refused(reason),
        },
        Aimed::Nobody => ErrandOutcome::Unreached,
        Aimed::Elsewhere(client) => return carry(world, flow, cx, client, reaching, deed),
    };
    let went = Went::of(reaching, cx.subject);
    match went.output(world, outcome) {
        REFUSED => EffectStep::Output(REFUSED),
        _ => EffectStep::Done,
    }
}

/// Has `deed` done to the character of `client`, in the world it stands in, and holds the flow
/// until word comes back of how it went.
pub fn carry(
    world: &mut World,
    flow: FlowKey,
    cx: &FlowCx,
    client: ClientId,
    reaching: Reaching,
    deed: impl FnOnce(&mut World, Entity) -> Result<(), LineText> + Send + Sync + 'static,
) -> EffectStep {
    let went = Went::of(reaching, cx.subject);
    errand::send(
        world,
        Errand {
            to: client,
            deed: Box::new(deed),
            then: Box::new(move |world, outcome| {
                let output = went.output(world, outcome);
                world
                    .resource_mut::<Instances>()
                    .answer(flow, Answer::Output(output));
            }),
        },
    );
    EffectStep::Pause
}

pub fn settle(
    world: &mut World,
    cx: &FlowCx,
    refused: &Arrow,
    outcome: Result<(), LineText>,
) -> EffectStep {
    match outcome {
        Ok(()) => EffectStep::Done,
        Err(reason) => refuse(world, cx, refused, reason),
    }
}

// An unwired refusal is the player's to read; a wired one is the script's to handle.
pub fn refuse(
    world: &mut World,
    cx: &FlowCx,
    refused: &Arrow,
    reason: impl Into<LineText>,
) -> EffectStep {
    if refused.is_none() {
        show_error(world, cx, reason);
    }
    EffectStep::Output(REFUSED)
}

pub fn show_error(world: &mut World, cx: &FlowCx, reason: impl Into<LineText>) {
    tell_error(world, cx.subject, reason);
}

pub fn number<'c>(cx: &mut Compiler<'c>, expr: &'c Expr) -> Result<Eval, ScriptFault> {
    cx.expr(expr, ValueType::Number)
}

fn tell_error(world: &mut World, subject: Option<Entity>, reason: impl Into<LineText>) {
    if let Some(subject) = subject
        && is_character(world, subject)
    {
        notification::error(world, subject, reason);
    }
}

/// What a reaching effect does once it knows how its deed went, kept apart from the flow so it
/// can wait for word from another world.
struct Went {
    done: &'static str,
    refusal: Refusal,
    nobody: Option<String>,
    subject: Option<Entity>,
}

enum Refusal {
    /// The effect has no `refused` output: its subject reads why, and the flow goes on.
    Told,
    /// Its `refused` output leads nowhere: its subject reads why.
    Unwired,
    /// Its `refused` output is the script's to handle.
    Wired,
}

impl Went {
    fn of(reaching: Reaching, subject: Option<Entity>) -> Went {
        Went {
            done: reaching.done,
            refusal: match reaching.refused {
                None => Refusal::Told,
                Some(arrow) if arrow.is_none() => Refusal::Unwired,
                Some(_) => Refusal::Wired,
            },
            nobody: reaching.nobody.map(str::to_owned),
            subject,
        }
    }

    /// The output the effect takes after `outcome`, having told its subject what refused it
    /// where that is theirs to read.
    fn output(self, world: &mut World, outcome: ErrandOutcome) -> &'static str {
        let reason = match outcome {
            ErrandOutcome::Done => None,
            ErrandOutcome::Refused(reason) => Some(reason),
            ErrandOutcome::Unreached => self.nobody.map(LineText::from),
        };
        let Some(reason) = reason else {
            return self.done;
        };
        match self.refusal {
            Refusal::Told => {
                tell_error(world, self.subject, reason);
                self.done
            }
            Refusal::Unwired => {
                tell_error(world, self.subject, reason);
                REFUSED
            }
            Refusal::Wired => REFUSED,
        }
    }
}

fn located(world: &World, player: Option<GlobalPlayerId>) -> Aimed {
    let Some(player) = player else {
        return Aimed::Nobody;
    };
    let client = ClientId(player.0);
    match world.resource::<Players>().0.get(&client) {
        Some(&character) if stat::stands(world, character) => Aimed::Here(character),
        Some(_) => Aimed::Nobody,
        None => Aimed::Elsewhere(client),
    }
}

pub fn actor(world: &World, cx: &FlowCx, eval: &Eval) -> Option<Entity> {
    eval(world, cx)
        .as_actor()
        .filter(|&who| stat::stands(world, who))
}

pub fn actors(world: &World, cx: &FlowCx, eval: &Eval) -> Vec<Entity> {
    eval(world, cx)
        .as_actors()
        .into_iter()
        .filter(|&who| stat::stands(world, who))
        .collect()
}

pub fn character(world: &World, cx: &FlowCx, eval: &Eval) -> Option<Entity> {
    actor(world, cx, eval).filter(|who| is_character(world, *who))
}

pub fn is_character(world: &World, who: Entity) -> bool {
    world.get::<PlayedBy>(who).is_some()
}
