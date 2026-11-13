use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString, IntoStaticStr};

use super::{Condition, EffectReach, RiftScripting};
use crate::core::audio::playback::SfxId;
use crate::core::content::SchemaMark;
use crate::core::script::{
    Answer, Arrow, Compiler, EffectDef, EffectStep, Eval, FlowCx, GlobalPlayerId, NEXT, REFUSED,
    ScriptFault, Value, ValueType,
};
use crate::systems::actor::appearance::SetAppearance;
use crate::systems::actor::held::SetHeld;
use crate::systems::actor::playing::{Animate, SetPose, SetStance};
use crate::systems::area::spawnable::SpawnObject;
use crate::systems::area::warp::{Warp, WarpToActor};
use crate::systems::chat::Say;
use crate::systems::class::GrantXp;
use crate::systems::combat::effects::{Damage, Heal};
use crate::systems::command::ListCommands;
use crate::systems::crafting::OpenCrafting;
use crate::systems::item::effects::{
    ClearGroundItems, DropItems, Equip, Items, MoveItems, ShowItem, SpawnGroundItems, Unequip,
    UseItem,
};
use crate::systems::memory::{Forget, Remember};
use crate::systems::notification::{self, Notify};
use crate::systems::npc::body::{Attack, FaceToward, Stop, WalkTo, Wander};
use crate::systems::npc::summon::{Despawn, SetAttitude, SpawnNpcs};
use crate::systems::player::errand::{self, Errand, ErrandOutcome};
use crate::systems::player::{ClientId, Players};
use crate::systems::prompt::{ItemPrompt, TakeOffer};
use crate::systems::quest::effects::{
    AcceptQuest, CompleteTask, Decide, FailQuest, OfferQuest, TurnIn,
};
use crate::systems::script::{EffectCall, EffectLook, FlowKey, RiftInstances, RunnerKind};
use crate::systems::shop::OpenShop;
use crate::systems::skill::{ResetSkills, UseSkill};
use crate::systems::stat;
use crate::systems::status::{ApplyStatus, Boost, RemoveStatus};
use crate::systems::vfx::PlayVfx;

macro_rules! effects {
    ($($variant:ident($ty:ty)),* $(,)?) => {
        #[derive(JsonSchema, Clone, Serialize, Deserialize, EnumDiscriminants)]
        #[serde(tag = "type", rename_all = "snake_case")]
        #[strum_discriminants(
            name(RiftEffectKind),
            derive(EnumString, IntoStaticStr),
            strum(serialize_all = "snake_case")
        )]
        pub enum RiftEffect {
            $($variant($ty),)*
        }

        impl EffectDef<RiftScripting> for RiftEffect {
            fn owners(&self) -> &'static [RunnerKind] {
                match self { $(RiftEffect::$variant(e) => e.owners(),)* }
            }
            fn needs_subject(&self) -> bool {
                match self { $(RiftEffect::$variant(e) => e.needs_subject(),)* }
            }
            fn outputs(&self) -> Vec<(&'static str, &Arrow)> {
                match self { $(RiftEffect::$variant(e) => e.outputs(),)* }
            }
            fn outputs_mut(&mut self) -> Vec<(&'static str, &mut Arrow)> {
                match self { $(RiftEffect::$variant(e) => e.outputs_mut(),)* }
            }
            fn pauses(&self) -> bool {
                match self { $(RiftEffect::$variant(e) => e.pauses(),)* }
            }
            fn keeps_box(&self) -> bool {
                match self { $(RiftEffect::$variant(e) => e.keeps_box(),)* }
            }
            fn compile<'c>(&'c self, cx: &mut Compiler<'c, RiftScripting>) -> Result<EffectCall, ScriptFault> {
                match self { $(RiftEffect::$variant(e) => e.compile(cx),)* }
            }
            fn look<'c>(&'c self, cx: &mut Compiler<'c, RiftScripting>) -> Result<Option<EffectLook>, ScriptFault> {
                match self { $(RiftEffect::$variant(e) => e.look(cx),)* }
            }
            fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
                match self { $(RiftEffect::$variant(e) => e.reach(cx),)* }
            }
        }
    };
}

effects! {
    Warp(Warp),
    WarpToActor(WarpToActor),
    Heal(Heal),
    Damage(Damage),
    ApplyStatus(ApplyStatus),
    RemoveStatus(RemoveStatus),
    Boost(Boost),
    Items(Items),
    UseItem(UseItem),
    Equip(Equip),
    Unequip(Unequip),
    DropItems(DropItems),
    SpawnGroundItems(SpawnGroundItems),
    ClearGroundItems(ClearGroundItems),
    MoveItems(MoveItems),
    GrantXp(GrantXp),
    Remember(Remember),
    Forget(Forget),
    OfferQuest(OfferQuest),
    AcceptQuest(AcceptQuest),
    TurnIn(TurnIn),
    FailQuest(FailQuest),
    CompleteTask(CompleteTask),
    Decide(Decide),
    OpenShop(OpenShop),
    ItemPrompt(ItemPrompt),
    TakeOffer(TakeOffer),
    OpenCrafting(OpenCrafting),
    Notify(Notify),
    Say(Say),
    Wander(Wander),
    WalkTo(WalkTo),
    Attack(Attack),
    Stop(Stop),
    Face(FaceToward),
    Animate(Animate),
    SetStance(SetStance),
    SetPose(SetPose),
    SetAppearance(SetAppearance),
    SetHeld(SetHeld),
    UseSkill(UseSkill),
    ResetSkills(ResetSkills),
    Vfx(PlayVfx),
    SetAttitude(SetAttitude),
    SpawnNpcs(SpawnNpcs),
    SpawnObject(SpawnObject),
    Despawn(Despawn),
    PlaySound(PlaySound),
    ShowItem(ShowItem),
    ListCommands(ListCommands),
}

impl RiftEffect {
    pub fn kind(&self) -> RiftEffectKind {
        self.into()
    }
}

impl RiftEffectKind {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

pub fn aim<'c>(
    cx: &mut Compiler<'c, RiftScripting>,
    who: &'c Option<Condition>,
) -> Result<Eval, ScriptFault> {
    cx.actor(who.as_ref())
}

pub fn aimed(world: &World, cx: &FlowCx, who: &Eval) -> Option<Entity> {
    super::query::actor(world, cx, who)
}

pub fn aimed_character(world: &World, cx: &FlowCx, who: &Eval) -> Option<Entity> {
    super::query::character(world, cx, who)
}

pub fn aim_all<'c>(
    cx: &mut Compiler<'c, RiftScripting>,
    who: &'c Option<Condition>,
) -> Result<Eval, ScriptFault> {
    cx.actors(who.as_ref())
}

pub fn aimed_all(world: &World, cx: &FlowCx, who: &Eval) -> Vec<Entity> {
    super::query::actors(world, cx, who)
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
pub fn aim_anywhere<'c>(
    cx: &mut Compiler<'c, RiftScripting>,
    who: &'c Option<Condition>,
) -> Result<Eval, ScriptFault> {
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
        Aimed::Here(actor) if !super::query::is_character(world, actor) => Aimed::Nobody,
        aimed => aimed,
    }
}

/// As `aim_all`, for an effect that also reaches a global player.
pub fn aim_all_anywhere<'c>(
    cx: &mut Compiler<'c, RiftScripting>,
    who: &'c Option<Condition>,
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
    deed: impl FnOnce(&mut World, Entity) -> Result<(), String> + Send + Sync + 'static,
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
    deed: impl FnOnce(&mut World, Entity) -> Result<(), String> + Send + Sync + 'static,
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
                    .resource_mut::<RiftInstances>()
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
    outcome: Result<(), String>,
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
    reason: impl Into<String>,
) -> EffectStep {
    if refused.is_none() {
        show_error(world, cx, reason);
    }
    EffectStep::Output(REFUSED)
}

pub fn show_error(world: &mut World, cx: &FlowCx, reason: impl Into<String>) {
    tell_error(world, cx.subject, reason);
}

/// Plays a sound at the owner or the subject.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("audio"))]
pub struct PlaySound {
    pub sfx: super::Fill<SfxId>,
    #[serde(rename = "for")]
    pub audience: Audience,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for PlaySound {
    fn needs_subject(&self) -> bool {
        self.audience == Audience::Subject
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let sfx = cx.audio(&self.sfx)?;
        let audience = self.audience;
        Ok(Box::new(move |world, _, cx| {
            notification::play_sound(world, cx.owner, cx.subject, sfx, audience);
            EffectStep::Done
        }))
    }
    fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        cx.peek_audio(&self.sfx)
            .map(EffectReach::Plays)
            .into_iter()
            .collect()
    }
}

#[derive(JsonSchema, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Everyone,
    Subject,
}

pub fn number<'c>(
    cx: &mut Compiler<'c, RiftScripting>,
    expr: &'c Condition,
) -> Result<Eval, ScriptFault> {
    cx.expr(expr, ValueType::Number)
}

fn tell_error(world: &mut World, subject: Option<Entity>, reason: impl Into<String>) {
    if let Some(subject) = subject
        && super::query::is_character(world, subject)
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
            ErrandOutcome::Unreached => self.nobody,
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
