use crate::core::content::ContentFile;
use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::Vocabulary;
use crate::systems::item::ItemDef;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use bevy_replication::ToClient;

use super::deliver::{self, DeliveryReason};
use super::{DroppedItem, Inventory, ItemId, ItemShown, ItemStack};
use crate::core::content::Content;
use crate::core::content::{SchemaMark, serial};
use crate::core::math::Pos;
use crate::core::script::compile::Bound;
use crate::core::script::{
    Arrow, Compiler, DONE, EffectDef, EffectStep, Eval, Fill, FlowCx, NEXT, REFUSED, ScriptFault,
    ValueType, VariableType,
};
use crate::core::script::{EffectCall, Expr};
use crate::core::text::LineText;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::aim::{self as effects, Reaching};
use crate::systems::area::Place;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::command;
use crate::systems::equipment;
use crate::systems::messages;
use crate::systems::movement::{Position, position};
use crate::systems::player::PlayedBy;
use crate::systems::visibility::Presence;

const USED: &str = "used";

/// Takes items, all or nothing, and pays what it gives: what has no room in the bag is sent as a
/// message.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct Items {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub take: Vec<ItemStackFill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub give: Vec<ItemStackFill>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// A stack of items a script names outright, or leaves to its variables to name.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemStackFill {
    pub item: Fill<ItemId>,
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: Fill<u32>,
}

fn one() -> Fill<u32> {
    Fill::Const(1)
}

fn is_one(count: &Fill<u32>) -> bool {
    matches!(count, Fill::Const(1))
}

impl ItemStackFill {
    /// The stack, when the script names both its item and its count outright.
    pub fn named(&self) -> Option<ItemStack> {
        match (&self.item, &self.count) {
            (Fill::Const(item), Fill::Const(count)) => Some(ItemStack::new(*item, *count)),
            _ => None,
        }
    }

    fn item(&self, cx: &Compiler) -> Option<ItemId> {
        cx.peek(&self.item, item_bound)
    }

    fn filled(&self, cx: &mut Compiler) -> Result<ItemStack, ScriptFault> {
        let item = cx.fill(&self.item, VariableType::Row, item_bound)?;
        let count = cx.fill(&self.count, VariableType::Integer, |bound| match bound {
            Bound::Integer(count) => u32::try_from(*count).ok(),
            _ => None,
        })?;
        Ok(ItemStack::new(item, count))
    }
}

fn item_bound(bound: &Bound) -> Option<ItemId> {
    match bound {
        Bound::Row(row) if row.table == ItemDef::TABLE => Some(ItemId::new(row.raw)),
        _ => None,
    }
}

impl Items {
    /// The stacks taken that the script names outright.
    pub fn named_take(&self) -> Vec<ItemStack> {
        self.take.iter().filter_map(ItemStackFill::named).collect()
    }

    /// The stacks given that the script names outright.
    pub fn named_give(&self) -> Vec<ItemStack> {
        self.give.iter().filter_map(ItemStackFill::named).collect()
    }
}

/// Shows an actor the card of an item, as inspecting it would.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct ShowItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub item: Fill<ItemId>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for ShowItem {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let item = cx.fill(&self.item, VariableType::Row, item_bound)?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                move |world, character| {
                    if let Some(client) = command::connection(world, character) {
                        world.write_message(ToClient {
                            client,
                            message: ItemShown { item },
                        });
                    }
                    Ok(())
                },
            )
        }))
    }
}

/// Uses an item as the player would.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct UseItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub item: ItemId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub used: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Puts an item on, swapping with the slot.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct Equip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub item: ItemId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Takes an item off, from wherever it is worn.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct Unequip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub item: ItemId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Drops items to the ground at their feet.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct DropItems {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub item: ItemId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub count: Expr,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Scatters items as loot at a place or actor.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct SpawnGroundItems {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Spot>,
    #[serde(rename = "for")]
    pub for_whom: GroundItemsFor,
    /// Without one, as long as the game's drops last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = SchemaMark::Value.with("number"))]
    pub seconds: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Removes ground items in range.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct ClearGroundItems {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Spot>,
    #[schemars(extend("default" = 3.0))]
    pub within: Expr,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Moves items from one bag to another.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("item"))]
pub struct MoveItems {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub from: Expr,
    #[schemars(extend("default" = {"type": "subject"}))]
    pub to: Expr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Spot {
    Place(Place),
    #[schemars(extend("default" = {"type": "subject"}))]
    Actor(Expr),
}

#[derive(JsonSchema, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundItemsFor {
    Subject,
    Anyone,
}

impl EffectDef for Items {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let take = self
            .take
            .iter()
            .map(|stack| stack.filled(cx))
            .collect::<Result<Vec<_>, _>>()?;
        let give = self
            .give
            .iter()
            .map(|stack| stack.filled(cx))
            .collect::<Result<Vec<_>, _>>()?;
        let refused = self.refused.clone();
        let reason = DeliveryReason::Given(cx.owner_row());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let (take, give, reason) = (take.clone(), give.clone(), reason.clone());
            let nobody = messages::of(world.resource::<Content>())
                .item
                .nobody_to_trade()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: DONE,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, actor| {
                    exchange(world, actor, &take, &[])?;
                    deliver::to_character(world, actor, &give, reason);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, cx: &Compiler) -> Vec<Reach> {
        let takes = self.take.iter().filter_map(|stack| stack.item(cx));
        let gives = self.give.iter().filter_map(|stack| stack.item(cx));
        let pays = (!self.give.is_empty()).then_some(Reach::new(PaysItems));
        takes
            .map(|item| Reach::new(Takes(item)))
            .chain(gives.map(|item| Reach::new(Gives(item))))
            .chain(pays)
            .collect()
    }
}

impl EffectDef for UseItem {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(USED => used, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let (item, refused) = (self.item, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let nobody = messages::of(world.resource::<Content>())
                .item
                .nobody_to_use()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: USED,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, actor| super::use_held(world, actor, item),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Takes(self.item))]
    }
}

impl EffectDef for Equip {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let (item, refused) = (self.item, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let nobody = messages::of(world.resource::<Content>())
                .item
                .nobody_to_equip()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: DONE,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, actor| equipment::equip_from_bag(world, actor, item),
            )
        }))
    }
}

impl EffectDef for Unequip {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let (item, refused) = (self.item, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let nobody = messages::of(world.resource::<Content>())
                .item
                .nobody_to_unequip()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: DONE,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, actor| equipment::unequip_item(world, actor, item),
            )
        }))
    }
}

impl EffectDef for DropItems {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let count = cx.expr(&self.count, ValueType::Integer)?;
        let (item, refused) = (self.item, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let count = count(world, cx).as_int().clamp(0, i64::from(u32::MAX)) as u32;
            let nobody = messages::of(world.resource::<Content>())
                .item
                .nobody_to_drop()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: DONE,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, actor| drop_held(world, actor, ItemStack::new(item, count)),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Drops(self.item))]
    }
}

impl EffectDef for SpawnGroundItems {
    fn needs_subject(&self) -> bool {
        self.for_whom == GroundItemsFor::Subject || self.at.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let at = spot(cx, self.at.as_ref())?;
        let seconds = cx.opt_expr(self.seconds.as_ref(), ValueType::Number)?;
        let (items, for_whom) = (self.items.clone(), self.for_whom);
        Ok(Box::new(move |world, _, cx| {
            let Some((area, from)) = at(world, cx) else {
                return EffectStep::Done;
            };
            let ttl = seconds
                .as_ref()
                .map(|seconds| Seconds(seconds(world, cx).as_num() as f32));
            let client = cx
                .subject
                .filter(|_| for_whom == GroundItemsFor::Subject)
                .and_then(|subject| world.get::<PlayedBy>(subject))
                .map(|owner| owner.client);
            let presence = client.map(Presence::For);
            let drops: Vec<(ItemId, u32)> = items
                .iter()
                .map(|stack| (stack.item, stack.count))
                .collect();
            super::scatter_at(world, from, area, &drops, client, presence, ttl);
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        self.items
            .iter()
            .map(|stack| Reach::new(Drops(stack.item)))
            .collect()
    }
}

impl EffectDef for ClearGroundItems {
    fn needs_subject(&self) -> bool {
        self.at.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let at = spot(cx, self.at.as_ref())?;
        let within = effects::number(cx, &self.within)?;
        Ok(Box::new(move |world, _, cx| {
            let Some((area, center)) = at(world, cx) else {
                return EffectStep::Done;
            };
            let within = Meters(within(world, cx).as_num() as f32);
            let cleared: Vec<Entity> = world
                .iter_entities()
                .filter(|entity| entity.get::<DroppedItem>().is_some())
                .filter(|entity| entity.get::<AreaTag>().is_some_and(|tag| tag.area == area))
                .filter(|entity| {
                    entity
                        .get::<Position>()
                        .is_some_and(|at| at.pos.distance(center) <= within)
                })
                .map(|entity| entity.id())
                .collect();
            for entity in cleared {
                world.despawn(entity);
            }
            EffectStep::Done
        }))
    }
}

impl EffectDef for MoveItems {
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let from = cx.expr(&self.from, ValueType::Actor)?;
        let to = cx.expr(&self.to, ValueType::Actor)?;
        let (items, refused) = (self.items.clone(), self.refused.clone());
        Ok(Box::new(move |world, _, cx| {
            let (Some(from), Some(to)) = (
                effects::aimed_character(world, cx, &from),
                effects::aimed_character(world, cx, &to),
            ) else {
                let nobody = LineText::from(
                    messages::of(world.resource::<Content>())
                        .item
                        .nobody_to_trade_between(),
                );
                return effects::refuse(world, cx, &refused, nobody);
            };
            match move_between(world, from, to, &items) {
                Ok(()) => EffectStep::Done,
                Err(reason) => effects::refuse(world, cx, &refused, reason),
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        self.items
            .iter()
            .flat_map(|stack| [Reach::new(Takes(stack.item)), Reach::new(Gives(stack.item))])
            .collect()
    }
}

pub fn exchange(
    world: &mut World,
    actor: Entity,
    take: &[ItemStack],
    give: &[ItemStack],
) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let mut inventory = world
        .get_mut::<Inventory>(actor)
        .ok_or_else(|| LineText::from(messages::of(&content).item.no_bag()))?;
    inventory
        .exchange(&content, take, give)
        .map_err(|refusal| refusal.line(&content))
}

pub fn move_between(
    world: &mut World,
    from: Entity,
    to: Entity,
    items: &[ItemStack],
) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    if from == to {
        return Err(LineText::from(
            messages::of(&content).item.nothing_to_move(),
        ));
    }
    ItemStack::refuse_bound(items, &content)?;
    let mut giving = bag_of(world, from, &content)?;
    let mut taking = bag_of(world, to, &content)?;
    giving
        .exchange(&content, items, &[])
        .map_err(|refusal| refusal.line(&content))?;
    taking
        .exchange(&content, &[], items)
        .map_err(|refusal| refusal.line(&content))?;
    world.entity_mut(from).insert(giving);
    world.entity_mut(to).insert(taking);
    Ok(())
}

/// Takes `stacks` out of `actor`'s bag to hand them to someone else, unless one of them can't
/// change hands.
pub fn give_away(world: &mut World, actor: Entity, stacks: &[ItemStack]) -> Result<(), LineText> {
    ItemStack::refuse_bound(stacks, world.resource::<Content>())?;
    exchange(world, actor, stacks, &[])
}

fn bag_of(world: &World, owner: Entity, content: &Content) -> Result<Inventory, LineText> {
    world
        .get::<Inventory>(owner)
        .cloned()
        .ok_or_else(|| LineText::from(messages::of(content).item.no_bag()))
}

type SpotEval = Box<dyn Fn(&World, &FlowCx) -> Option<(AreaId, Pos<Meters>)> + Send + Sync>;

fn spot<'c>(cx: &mut Compiler<'c>, at: Option<&'c Spot>) -> Result<SpotEval, ScriptFault> {
    Ok(match at {
        Some(Spot::Place(place)) => {
            let place = *place;
            Box::new(move |_, _| Some((place.area, place.at)))
        }
        Some(Spot::Actor(actor)) => {
            let actor: Eval = cx.expr(actor, ValueType::Actor)?;
            Box::new(move |world, cx| {
                let who = effects::aimed(world, cx, &actor)?;
                standing(world, who)
            })
        }
        None => Box::new(|world, cx| cx.owner.or(cx.subject).and_then(|who| standing(world, who))),
    })
}

fn standing(world: &World, who: Entity) -> Option<(AreaId, Pos<Meters>)> {
    Some((world.get::<AreaTag>(who)?.area, position(world, who)?))
}

fn drop_held(world: &mut World, actor: Entity, stack: ItemStack) -> Result<(), LineText> {
    give_away(world, actor, &[stack])?;
    super::scatter_drop(world, actor, &[(stack.item, stack.count)], None, None);
    Ok(())
}

/// A script that may take items from someone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Takes(pub ItemId);

impl ReachDef for Takes {}

/// A script that may give someone items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gives(pub ItemId);

impl ReachDef for Gives {}

/// A script that may drop items on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drops(pub ItemId);

impl ReachDef for Drops {}

/// A script that may pay someone items, sending what has no room in their bag as a message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaysItems;

impl ReachDef for PaysItems {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<Items>("items")
        .effect::<UseItem>("use_item")
        .effect::<Equip>("equip")
        .effect::<Unequip>("unequip")
        .effect::<DropItems>("drop_items")
        .effect::<SpawnGroundItems>("spawn_ground_items")
        .effect::<ClearGroundItems>("clear_ground_items")
        .effect::<MoveItems>("move_items")
        .effect::<ShowItem>("show_item");
}
