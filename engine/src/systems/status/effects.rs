use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AspectSource, StatusCategory, StatusEffectId, boost};
use crate::core::content::Content;
use crate::core::script::{Compiler, EffectDef, EffectStep, Expr, Name, ScriptFault, ValueType};
use crate::core::time::Seconds;
use crate::systems::item::ItemId;
use crate::systems::script::effect::{self as effects, Aim, Arrow, Link};
use crate::systems::script::{
    Condition, EffectCall, OwnerKind, OwnerRow, Reach, RiftScripting, RiftScripts,
};
use crate::systems::skill::{SkillId, SkillKind};
use crate::systems::stat::{Stat, StatKind};

/// Puts a status on actors, at a power and for a time; without seconds it holds until removed.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ApplyStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub effect: StatusEffectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

/// Takes a status off actors: one effect, every status of a category, or every status at all.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct RemoveStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<StatusEffectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<StatusCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

/// A stat modifier held on the subject for as long as the aspect whose script this is stays
/// active on it: a status, a worn or carried item, a passive skill.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Boost {
    pub stat: StatKind,
    pub by: Condition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

const NEXT: Name = "next";

impl EffectDef<RiftScripting> for ApplyStatus {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(NEXT, self.next.as_deref())]
    }
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
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Applies(self.effect)]
    }
}

impl EffectDef<RiftScripting> for RemoveStatus {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(NEXT, self.next.as_deref())]
    }
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
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Cleanses]
    }
}

impl EffectDef<RiftScripting> for Boost {
    fn needs_subject(&self) -> bool {
        true
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(NEXT, self.next.as_deref())]
    }
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
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Boosts(self.stat, constant(&self.by))]
    }
}

/// The boosts an item's script holds while worn or carried, as its card lists them: the stat, and
/// the amount when the script states it outright.
pub fn boosts_of(content: &Content, item: ItemId) -> Vec<(StatKind, Option<f32>)> {
    content
        .built::<RiftScripts>()
        .may_do(OwnerRow {
            kind: OwnerKind::Item,
            row: item.raw(),
            object: None,
        })
        .reaches
        .iter()
        .filter_map(|reach| match reach {
            Reach::Boosts(stat, by) => Some((*stat, *by)),
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
        OwnerKind::StatusEffect => Ok(AspectSource::Status(row.id())),
        OwnerKind::Item => Ok(AspectSource::Item(row.id())),
        OwnerKind::Skill => {
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
