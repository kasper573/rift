use crate::systems::skill::SKILL_RUNNER;
use crate::systems::skill::SkillDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::actor;
use crate::systems::mention::Mention;
use crate::systems::requirement::Describers;
use crate::systems::skill;
use crate::systems::skill::SkillId;
use crate::systems::skill::SkillStat;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillRank {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub skill: SkillId,
}

impl QueryDef for SkillRank {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Integer)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let SkillRank { of, skill } = self;
        Ok({
            let skill = *skill;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(
                    actor(world, cx, &of)
                        .map_or(0, |who| i64::from(skill::rank_of(world, who, skill).0)),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillStatValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub skill: SkillId,
    pub stat: SkillStat,
}

impl QueryDef for SkillStatValue {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let SkillStatValue { of, skill, stat } = self;
        Ok({
            let (skill, stat) = (*skill, *stat);
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                let rank = actor(world, cx, &of)
                    .map_or(skill::Rank::NONE, |who| skill::rank_of(world, who, skill));
                Value::Num(f64::from(
                    skill.get(world.resource::<Content>()).stat(rank, stat),
                ))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillTargets {}

impl QueryDef for SkillTargets {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actors)
    }

    fn reads_subject(&self) -> bool {
        true
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok({
            let skill = skill_of(cx)?;
            Box::new(move |world, cx| Value::Actors(skill::targets(world, cx, skill)))
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillArea {}

impl QueryDef for SkillArea {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Area)
    }

    fn reads_subject(&self) -> bool {
        true
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok({
            let skill = skill_of(cx)?;
            Box::new(move |world, cx| Value::Area(skill::footprint(world, cx, skill)))
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct AimedPoint {}

impl QueryDef for AimedPoint {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Point)
    }

    fn reads_subject(&self) -> bool {
        true
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok({
            let skill = skill_of(cx)?;
            Box::new(move |world, cx| Value::Point(skill::aimed_point(world, cx, skill)))
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<SkillRank>("skill_rank")
        .query::<SkillStatValue>("skill_stat")
        .query::<SkillTargets>("skill_targets")
        .query::<SkillArea>("skill_area")
        .query::<AimedPoint>("aimed_point");
    vocabulary
        .extension::<Describers>()
        .alone::<SkillRank>(|q, d| {
            d.words()
                .skill_rank(d.names.mentioned(Mention::Skill(q.skill)))
        })
        .alone::<SkillStatValue>(|q, d| {
            d.words()
                .skill_stat(d.names.mentioned(Mention::Skill(q.skill)), q.stat.key())
        })
        .alone::<SkillTargets>(|_q, d| LineText::plain(d.words().skill_targets()))
        .alone::<SkillArea>(|_q, d| LineText::plain(d.words().skill_area()))
        .alone::<AimedPoint>(|_q, d| LineText::plain(d.words().aimed_point()));
}

fn skill_of(cx: &Compiler) -> Result<SkillId, ScriptFault> {
    cx.owner_row()
        .filter(|row| row.kind == SKILL_RUNNER)
        .map(|row| row.id::<SkillDef>())
        .ok_or_else(|| {
            ScriptFault::new(
                "wrong owner",
                "only a skill's own script knows what it was aimed at",
            )
        })
}
