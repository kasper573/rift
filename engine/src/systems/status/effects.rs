use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AspectSource, StatusCategory, StatusEffectId, boost};
use crate::core::content::Content;
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, Expr, NEXT, ScriptFault, ValueType,
};
use crate::core::units::Seconds;
use crate::systems::item::ItemId;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, OwnerRow, RiftScripting, RiftScripts, RunnerKind,
    effect as effects,
};
use crate::systems::skill::{SkillId, SkillKind};
use crate::systems::stat::{Stat, StatKind};

/// Puts a status on actors, at a power and for a time; without seconds it holds until removed.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "status"))]
pub struct ApplyStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub effect: StatusEffectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = 1.0))]
    pub power: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = 10.0))]
    pub seconds: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Takes a status off actors: one effect, every status of a category, or every status at all.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "status"))]
pub struct RemoveStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<StatusEffectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<StatusCategory>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// A stat modifier held on the subject for as long as the aspect whose script this is stays
/// active on it: a status, a worn or carried item, a passive skill.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "status"))]
pub struct Boost {
    pub stat: StatKind,
    #[schemars(extend("default" = 1.0))]
    pub by: Condition,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for ApplyStatus {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all(cx, &self.who)?;
        let power = cx.opt_expr(self.power.as_ref(), ValueType::Number)?;
        let seconds = cx.opt_expr(self.seconds.as_ref(), ValueType::Number)?;
        let effect = self.effect;
        Ok(Box::new(move |world, _, cx| {
            for actor in effects::aimed_all(world, cx, &who) {
                let power = power
                    .as_ref()
                    .map_or(1.0, |power| power(world, cx).as_num() as f32);
                let lasts = seconds
                    .as_ref()
                    .map(|seconds| Seconds(seconds(world, cx).as_num() as f32));
                super::apply(world, actor, effect, power, lasts);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Applies(self.effect)]
    }
}

impl EffectDef<RiftScripting> for RemoveStatus {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all(cx, &self.who)?;
        let (effect, category) = (self.effect, self.category);
        Ok(Box::new(move |world, _, cx| {
            for actor in effects::aimed_all(world, cx, &who) {
                match effect {
                    Some(effect) => {
                        super::remove(world, actor, effect);
                    }
                    None => {
                        super::cleanse(world, actor, category);
                    }
                }
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Cleanses]
    }
}

impl EffectDef<RiftScripting> for Boost {
    fn needs_subject(&self) -> bool {
        true
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let source = source_of(cx)?;
        let by = effects::number(cx, &self.by)?;
        let stat = self.stat;
        Ok(Box::new(move |world, _, cx| {
            if let Some(subject) = cx.subject {
                let value = by(world, cx).as_num() as f32;
                boost::hold(world, subject, source, Stat { kind: stat, value });
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Boosts(self.stat, constant(&self.by))]
    }
}

/// The boosts an item's script holds while worn or carried, as its card lists them: the stat, and
/// the amount when the script states it outright.
pub fn boosts_of(content: &Content, item: ItemId) -> Vec<(StatKind, Option<f32>)> {
    content
        .built::<RiftScripts>()
        .may_do(OwnerRow::of_row(item))
        .reaches
        .iter()
        .filter_map(|reach| match reach {
            EffectReach::Boosts(stat, by) => Some((*stat, *by)),
            _ => None,
        })
        .collect()
}

/// The aspect a boost belongs to, from the row whose script it is in.
fn source_of(cx: &Compiler<RiftScripting>) -> Result<AspectSource, ScriptFault> {
    let expected = "a status effect's, an item's or a passive skill's script";
    let Some(row) = cx.owner_row() else {
        return Err(ScriptFault::new("boost without an aspect", expected));
    };
    match row.kind {
        RunnerKind::StatusEffect => Ok(AspectSource::Status(row.id())),
        RunnerKind::Item => Ok(AspectSource::Item(row.id())),
        RunnerKind::Skill => {
            let skill: SkillId = row.id();
            if skill.get(cx.content).kind == SkillKind::Passive {
                Ok(AspectSource::Skill(skill))
            } else {
                Err(ScriptFault::new(
                    "boost in an active skill",
                    "a passive skill: an active one's script runs on use, with nothing to hold a boost",
                ))
            }
        }
        _ => Err(ScriptFault::new("boost without an aspect", expected)),
    }
}

fn constant(expr: &Condition) -> Option<f32> {
    match expr {
        Expr::Num(value) => Some(*value as f32),
        Expr::Int(value) => Some(*value as f32),
        _ => None,
    }
}
