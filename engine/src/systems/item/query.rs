use crate::core::units::MeterPos;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::core::units::Meters;
use crate::systems::aim::{actor, character};
use crate::systems::area::AreaTag;
use crate::systems::item::DroppedItem;
use crate::systems::item::Inventory;
use crate::systems::item::ItemId;
use crate::systems::mention::Mention;
use crate::systems::movement;
use crate::systems::movement::Position;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Holding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub item: ItemId,
}

impl QueryDef for Holding {
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
        let Holding { of, item } = self;
        Ok({
            let item = *item;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(character(world, cx, &of).map_or(0, |who| {
                    world
                        .get::<Inventory>(who)
                        .map_or(0, |bag| i64::from(bag.count(item)))
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct FreeSlots {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for FreeSlots {
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
        let FreeSlots { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(character(world, cx, &of).map_or(0, |who| {
                    world
                        .get::<Inventory>(who)
                        .map_or(0, |bag| i64::from(bag.free_slots()))
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct GroundItems {
    pub item: ItemId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub at: Option<Box<Expr>>,
    #[schemars(extend("default" = 6.0))]
    pub within: Box<Expr>,
}

impl QueryDef for GroundItems {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Integer)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let GroundItems { item, at, within } = self;
        Ok({
            let item = *item;
            let at = cx.opt_expr(at.as_deref(), ValueType::Actor)?;
            let within = cx.expr(within, ValueType::Number)?;
            Box::new(move |world, cx| {
                let within = Meters(within(world, cx).as_num() as f32);
                let center = match &at {
                    Some(at) => actor(world, cx, at).and_then(|who| movement::position(world, who)),
                    None => cx.owner.and_then(|owner| movement::position(world, owner)),
                };
                let Some(center) = center else {
                    return Value::Int(0);
                };
                let area = cx
                    .owner
                    .and_then(|owner| world.get::<AreaTag>(owner).map(|tag| tag.area));
                let mut count = 0;
                for entity in world.iter_entities() {
                    let Some(drop) = entity.get::<DroppedItem>() else {
                        continue;
                    };
                    if drop.item != item {
                        continue;
                    }
                    if area.is_some() && entity.get::<AreaTag>().map(|tag| tag.area) != area {
                        continue;
                    }
                    let Some(at) = entity.get::<Position>() else {
                        continue;
                    };
                    if at.pos.distance(center) <= within {
                        count += i64::from(drop.count);
                    }
                }
                Value::Int(count)
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Holding>("holding")
        .query::<FreeSlots>("free_slots")
        .query::<GroundItems>("ground_items");
    vocabulary
        .extension::<Describers>()
        .alone::<Holding>(|q, d| d.names.mentioned(Mention::Item(q.item)))
        .compared::<Holding>(|q, limit, d| match limit {
            Expr::Int(count) => {
                let item = d.names.mentioned(Mention::Item(q.item));
                Some(match u32::try_from(*count) {
                    Ok(count) => d.words().items_counted(count, item),
                    Err(_) => item,
                })
            }
            _ => None,
        })
        .alone::<FreeSlots>(|_q, d| LineText::plain(d.words().free_slots()))
        .compared::<FreeSlots>(|_q, limit, d| match limit {
            Expr::Int(slots) => Some(d.words().free_slots_of(slots.unsigned_abs())),
            _ => None,
        })
        .alone::<GroundItems>(|q, d| {
            d.words()
                .on_ground(d.names.mentioned(Mention::Item(q.item)))
        });
}
