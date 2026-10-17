pub use crate::core::script::{Arrow, Link};

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Condition, Reach, RiftScripting};
use crate::core::audio::playback::SfxId;
use crate::core::script::{
    Compiler, EffectDef, EffectStep, Eval, FlowCx, Name, ScriptFault, ValueType,
};
use crate::systems::actor::Animate;
use crate::systems::area::warp::Warp;
use crate::systems::chat::Say;
use crate::systems::class::GrantXp;
use crate::systems::item::effects::{
    ClearGroundItems, DropItems, Equip, Items, MoveItems, SpawnGroundItems, Unequip, UseItem,
};
use crate::systems::memory::{Forget, Remember};
use crate::systems::notification::{self, Notify};
use crate::systems::npc::body::{Attack, FaceToward, Stop, WalkTo, Wander};
use crate::systems::npc::summon::{Despawn, SetAttitude, SpawnNpcs};
use crate::systems::quest::effects::{
    AcceptQuest, CompleteTask, Decide, FailQuest, OfferQuest, TurnIn,
};
use crate::systems::script::{EffectCall, OwnerKind};
use crate::systems::shop::OpenShop;
use crate::systems::skill::{ResetSkills, UseSkill};
use crate::systems::stat::effects::{Damage, Heal};
use crate::systems::status::{ApplyStatus, Boost, RemoveStatus};
use crate::systems::vfx::Vfx;

pub type Aim = Option<Condition>;

macro_rules! effects {
    ($($variant:ident($ty:ty)),* $(,)?) => {
        #[derive(JsonSchema, Clone, Serialize, Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum RiftEffect {
            $($variant($ty),)*
        }

        impl EffectDef<RiftScripting> for RiftEffect {
            fn owners(&self) -> &'static [OwnerKind] {
                match self { $(RiftEffect::$variant(e) => e.owners(),)* }
            }
            fn needs_subject(&self) -> bool {
                match self { $(RiftEffect::$variant(e) => e.needs_subject(),)* }
            }
            fn outputs(&self) -> Vec<(Name, Link<'_>)> {
                match self { $(RiftEffect::$variant(e) => e.outputs(),)* }
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
            fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<Reach> {
                match self { $(RiftEffect::$variant(e) => e.reach(cx),)* }
            }
        }

        impl RiftEffect {
            pub fn kind(&self) -> &'static str {
                match self { $(RiftEffect::$variant(_) => stringify!($variant),)* }
            }
        }
    };
}

effects! {
    Warp(Warp),
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
    Notify(Notify),
    Say(Say),
    Wander(Wander),
    WalkTo(WalkTo),
    Attack(Attack),
    Stop(Stop),
    Face(FaceToward),
    Animate(Animate),
    UseSkill(UseSkill),
    ResetSkills(ResetSkills),
    Vfx(Vfx),
    SetAttitude(SetAttitude),
    SpawnNpcs(SpawnNpcs),
    Despawn(Despawn),
    PlaySound(PlaySound),
}

pub fn aim<'c>(cx: &mut Compiler<'c, RiftScripting>, who: &'c Aim) -> Result<Eval, ScriptFault> {
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
    who: &'c Aim,
) -> Result<Eval, ScriptFault> {
    cx.actors(who.as_ref())
}

pub fn aimed_all(world: &World, cx: &FlowCx, who: &Eval) -> Vec<Entity> {
    super::query::actors(world, cx, who)
}

pub fn settle(
    world: &mut World,
    cx: &FlowCx,
    refused: Link,
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
    refused: Link,
    reason: impl Into<String>,
) -> EffectStep {
    if refused.is_none() {
        show_error(world, cx, reason);
    }
    EffectStep::Output("refused")
}

pub fn show_error(world: &mut World, cx: &FlowCx, reason: impl Into<String>) {
    if let Some(subject) = cx.subject
        && super::query::is_character(world, subject)
    {
        notification::error(world, subject, reason);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaySound {
    pub sfx: super::Fill<SfxId>,
    #[serde(rename = "for")]
    pub audience: Audience,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for PlaySound {
    fn needs_subject(&self) -> bool {
        self.audience == Audience::Subject
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![("next", self.next.as_deref())]
    }
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
    fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        cx.peek_audio(&self.sfx)
            .map(Reach::Plays)
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
