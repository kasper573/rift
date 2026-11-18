use crate::core::units::MeterPos;
use crate::core::units::Meters;
use bevy_ecs::prelude::Entity;
use bevy_ecs::prelude::World;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::actor;
use crate::systems::movement;
use crate::systems::npc::Npc;
use crate::systems::npc::body;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Present {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for Present {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Present { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| Value::Bool(actor(world, cx, &who).is_some()))
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Distance {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for Distance {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Distance { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| {
                let apart = cx
                    .owner
                    .zip(actor(world, cx, &who))
                    .and_then(|(owner, who)| distance(world, owner, who));
                Value::Num(f64::from(apart.map_or(0.0, |apart| apart.0)))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct IsNpc {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for IsNpc {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let IsNpc { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| {
                Value::Bool(
                    actor(world, cx, &who).is_some_and(|who| world.get::<Npc>(who).is_some()),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Walking {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for Walking {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Walking { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| {
                Value::Bool(actor(world, cx, &who).is_some_and(|who| movement::moving(world, who)))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Subject {}

impl QueryDef for Subject {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actor)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, _cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|_, cx| Value::Actor(cx.subject)))
    }
}

/// The npc or object whose script this is.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Owner {}

impl QueryDef for Owner {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actor)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, _cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|_, cx| Value::Actor(cx.owner)))
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Target {}

impl QueryDef for Target {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Actor)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, _cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|world, cx| {
            Value::Actor(cx.owner.and_then(|owner| body::target(world, owner)))
        }))
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Present>("present")
        .query::<Distance>("distance")
        .query::<IsNpc>("is_npc")
        .query::<Walking>("walking")
        .query::<Subject>("subject")
        .query::<Owner>("owner")
        .query::<Target>("target");
    vocabulary
        .extension::<Describers>()
        .alone::<Present>(|_q, d| LineText::plain(d.words().present()))
        .alone::<Distance>(|_q, d| LineText::plain(d.words().distance()))
        .alone::<IsNpc>(|_q, d| LineText::plain(d.words().an_npc()))
        .alone::<Walking>(|_q, d| LineText::plain(d.words().walking()))
        .alone::<Subject>(|_q, d| LineText::plain(d.words().you()))
        .alone::<Owner>(|_q, d| LineText::plain(d.words().itself()))
        .alone::<Target>(|_q, d| LineText::plain(d.words().target()));
}

fn distance(world: &World, from: Entity, to: Entity) -> Option<Meters> {
    Some(movement::position(world, from)?.distance(movement::position(world, to)?))
}
