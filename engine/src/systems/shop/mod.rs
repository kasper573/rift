pub mod counter;

use std::collections::{HashMap, VecDeque};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, serial,
};
use crate::core::script::{Answer, Arrow, Compiler, EffectDef, EffectStep, ScriptFault, Value};
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::systems::attention::AttentionId;
use crate::systems::item::{
    ExchangeRefusal, Inventory, ItemCategory, ItemDef, ItemId, ItemModuleKind, ItemStack,
};
use crate::systems::player::sender_player;
use crate::systems::script::{
    self, Condition, EffectCall, EventKind, FlowKey, Line, Owner, Reach, RiftEvent, RiftInstances,
    RiftScripting, RunnerKind, ScriptModule, ScriptedRow, condition, effect as effects,
};
use crate::systems::{dialogue, drain_requests, notification};

pub type ShopId = ContentId<ShopDef>;

const BUYBACK_KEPT: usize = 12;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::{Channel, ClientMessageAppExt};

    use crate::systems::mirror::AppMirrorExt;
    app.replicate_private::<ShopView>()
        .add_client_message::<ShopRequest>(Channel::Ordered);
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopDef {
    pub title: String,
    pub mark: AttentionId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ShopModule>,
}

impl NamedContent for ShopDef {
    fn shown_name(&self) -> &str {
        &self.title
    }
}

impl ContentFile for ShopDef {
    const TABLE: &'static str = "shop";

    fn fresh(near: &Nearby) -> Result<ShopDef, String> {
        Ok(ShopDef {
            title: near.name(),
            mark: near.file()?,
            modules: Vec::new(),
        })
    }
}

impl ModuleSet for ShopDef {
    type Module = ShopModule;

    fn modules(&self) -> &[ShopModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<ShopModule> {
        &mut self.modules
    }

    fn rule(module: &ShopModule) -> ModuleRule {
        match module {
            ShopModule::Sells(_) | ShopModule::Buys(_) | ShopModule::Script(_) => {
                ModuleRule::AnyNumber
            }
            ShopModule::Reactions(_) => ModuleRule::AtMostOne,
        }
    }
}

impl ScriptedRow for ShopDef {
    const OWNER: RunnerKind = RunnerKind::Shop;

    fn script(module: &ShopModule) -> Option<&ScriptModule> {
        match module {
            ShopModule::Script(script) => Some(script),
            _ => None,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShopModule {
    Sells(ShopOffer),
    Buys(ShopBuys),
    Reactions(ShopReactions),
    Script(ScriptModule),
}

impl ShopDef {
    pub fn sells(&self) -> impl Iterator<Item = &ShopOffer> + Clone {
        self.modules.iter().filter_map(|module| match module {
            ShopModule::Sells(offer) => Some(offer),
            _ => None,
        })
    }

    pub fn buys(&self) -> impl Iterator<Item = &ShopBuys> + Clone {
        self.modules.iter().filter_map(|module| match module {
            ShopModule::Buys(buys) => Some(buys),
            _ => None,
        })
    }

    pub fn reactions(&self) -> Option<&ShopReactions> {
        self.modules.iter().find_map(|module| match module {
            ShopModule::Reactions(reactions) => Some(reactions),
            _ => None,
        })
    }

    pub fn pays_for(
        &self,
        content: &Content,
        item: ItemId,
        declined: &[u32],
    ) -> Result<&[ItemStack], String> {
        let def = item.get(content);
        if def.has(ItemModuleKind::QuestItem) {
            return Err("Quest items never sell".to_owned());
        }
        if def.bound() {
            return Err("Bound items never sell".to_owned());
        }
        let (index, rule) = self
            .buys()
            .enumerate()
            .find(|(_, rule)| match rule.what {
                Buys::Item { item: wanted } => wanted == item,
                Buys::Category { category } => def.category() == category,
            })
            .ok_or_else(|| format!("{} won't buy {}", self.title, def.name))?;
        if declined.contains(&(index as u32)) {
            return Err(format!("{} won't buy {} from you", self.title, def.name));
        }
        Ok(&rule.pays)
    }

    pub fn wants(&self, content: &Content) -> Vec<String> {
        self.buys()
            .map(|rule| match rule.what {
                Buys::Item { item } => item.get(content).name.to_owned(),
                Buys::Category { category } => category.plural().to_owned(),
            })
            .collect()
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopOffer {
    pub item: ItemId,
    #[schemars(extend("x-count" = true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub price: Vec<ItemStack>,
    pub stock: Stock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Condition>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Stock {
    Unlimited,
    Limited { count: u32, restock: Seconds },
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopBuys {
    pub what: Buys,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pays: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Condition>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Buys {
    Item { item: ItemId },
    Category { category: ItemCategory },
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopReactions {
    pub bought: Vec<Line>,
    pub sold: Vec<Line>,
    pub cant_afford: Vec<Line>,
}

/// Opens the shop until it closes.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "shop"))]
pub struct OpenShop {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub shop: ShopId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub closed: Arrow,
}

impl EffectDef<RiftScripting> for OpenShop {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!("closed" => closed);
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
        let who = effects::aim(cx, &self.who)?;
        let shop = self.shop;
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Done;
            };
            if open(world, player, shop, flow) {
                EffectStep::Pause
            } else {
                EffectStep::Done
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::OpensShop(self.shop)]
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct ShopView {
    pub shop: ShopId,
    pub offers: Vec<OfferView>,
    pub buyback: Vec<BuybackView>,
    pub quotes: Vec<SaleQuote>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct SaleQuote {
    pub item: ItemId,
    pub verdict: SaleVerdict,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub enum SaleVerdict {
    Pays(Vec<ItemStack>),
    Refused(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct OfferView {
    pub left: Option<u32>,
    pub restocks_in: Option<Seconds>,
    pub refusal: Option<WareRefusal>,
    pub held: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct BuybackView {
    pub sale: Sale,
    pub refusal: Option<WareRefusal>,
    pub held: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub enum WareRefusal {
    Unaffordable(ItemStack),
    Unavailable(String),
}

impl WareRefusal {
    pub fn describe(&self, content: &Content) -> String {
        match self {
            WareRefusal::Unaffordable(short) => ExchangeRefusal::Missing(*short).describe(content),
            WareRefusal::Unavailable(reason) => reason.clone(),
        }
    }

    fn of(refusal: ExchangeRefusal, content: &Content) -> WareRefusal {
        match refusal {
            ExchangeRefusal::Missing(short) => WareRefusal::Unaffordable(short),
            ExchangeRefusal::NoRoom { .. } => WareRefusal::Unavailable(refusal.describe(content)),
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Sale {
    pub item: ItemId,
    pub count: u32,
    pub paid: Vec<ItemStack>,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShopLedger {
    stock: HashMap<(ShopId, u32), Restock>,
    sales: HashMap<ShopId, VecDeque<Sale>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Restock {
    left: u32,
    full_at: UnixMillis,
}

impl ShopLedger {
    pub fn stocked(&mut self, shop: ShopId, offer: u32, left: u32, full_at: UnixMillis) {
        self.stock.insert((shop, offer), Restock { left, full_at });
    }

    pub fn record_sale(&mut self, shop: ShopId, sale: Sale) {
        let sales = self.sales.entry(shop).or_default();
        sales.push_front(sale);
        sales.truncate(BUYBACK_KEPT);
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ShopRequest {
    Buy { offer: u32 },
    Sell { slot: u32, stack: ItemStack },
    Buyback { sale: u32 },
}

#[derive(Component)]
pub struct ShopVisit {
    shop: ShopId,
    flow: FlowKey,
    reactions: u32,
}

fn open(world: &mut World, player: Entity, shop: ShopId, flow: FlowKey) -> bool {
    if !dialogue::card(
        world,
        flow,
        player,
        Vec::new(),
        Answer::Output("closed"),
        None,
    ) {
        return false;
    }
    world.entity_mut(player).insert(ShopVisit {
        shop,
        flow,
        reactions: 0,
    });
    refresh_view(world, player);
    true
}

fn close(world: &mut World, player: Entity) {
    world.entity_mut(player).remove::<(ShopVisit, ShopView)>();
}

pub fn visiting(world: &World, player: Entity) -> Option<ShopId> {
    world.get::<ShopVisit>(player).map(|visit| visit.shop)
}

pub fn ledger(world: &World, player: Entity) -> ShopLedger {
    world.get::<ShopLedger>(player).cloned().unwrap_or_default()
}

pub fn requests(world: &mut World) {
    for request in drain_requests::<ShopRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if !bound(world, player) {
            continue;
        }
        match request.message {
            ShopRequest::Buy { offer } => buy(world, player, offer),
            ShopRequest::Sell { slot, stack } => sell(world, player, slot, stack),
            ShopRequest::Buyback { sale } => buy_back(world, player, sale),
        }
    }
}

pub fn hold(world: &mut World, visits: &mut QueryState<Entity, With<ShopVisit>>) {
    let players: Vec<Entity> = visits.iter(world).collect();
    for player in players {
        if !bound(world, player) {
            close(world, player);
        }
    }
}

pub fn refresh(world: &mut World, visits: &mut QueryState<Entity, With<ShopVisit>>) {
    let players: Vec<Entity> = visits.iter(world).collect();
    for player in players {
        refresh_view(world, player);
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let shops = loading.rows::<ShopDef>(&content);
    for &(id, shop) in &shops {
        for (index, module) in shop.modules.iter().enumerate() {
            let requires = match module {
                ShopModule::Sells(offer) => offer.requires.as_ref(),
                ShopModule::Buys(rule) => rule.requires.as_ref(),
                _ => None,
            };
            if let Some(requires) = requires
                && let Err(fault) = condition::check(&content, requires)
            {
                loading.row_error(
                    id,
                    &format!("/modules/{index}/requires"),
                    ContentProblem::new(fault.rule, "the condition", fault.expected),
                );
            }
            let ShopModule::Sells(offer) = module else {
                continue;
            };
            if content.table::<ItemDef>().get(offer.item).is_none() {
                continue;
            }
            for &(buyer, def) in &shops {
                if let Ok(pays) = def.pays_for(&content, offer.item, &[])
                    && profits(offer, pays)
                {
                    loading.row_error(
                        id,
                        &format!("/modules/{index}"),
                        ContentProblem::new(
                            "shop loses money",
                            format!(
                                "{}: {} buys {} for {}, sold for {}",
                                offer.item.path(&content),
                                buyer.path(&content),
                                offer.count,
                                stacks_text(&content, pays, offer.count),
                                stacks_text(&content, &offer.price, 1)
                            ),
                            "every shop sells an item for at least what any shop pays for it",
                        ),
                    );
                }
            }
        }
    }
}

fn stacks_text(content: &Content, stacks: &[ItemStack], times: u32) -> String {
    let parts: Vec<String> = stacks
        .iter()
        .map(|stack| {
            format!(
                "{} {}",
                stack.count.saturating_mul(times),
                stack.item.path(content)
            )
        })
        .collect();
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(" + ")
    }
}

fn profits(offer: &ShopOffer, pays: &[ItemStack]) -> bool {
    let amount = |stacks: &[ItemStack], item: ItemId, times: u32| {
        stacks
            .iter()
            .filter(|stack| stack.item == item)
            .map(|stack| stack.count.saturating_mul(times))
            .sum::<u32>()
    };
    let items: Vec<ItemId> = offer
        .price
        .iter()
        .chain(pays)
        .map(|stack| stack.item)
        .collect();
    let earned = |item| amount(pays, item, offer.count);
    let spent = |item| amount(&offer.price, item, 1);
    items.iter().all(|&item| earned(item) >= spent(item))
        && items.iter().any(|&item| earned(item) > spent(item))
}

fn bound(world: &World, player: Entity) -> bool {
    world
        .get::<ShopVisit>(player)
        .is_some_and(|visit| dialogue::flow_of(world, player) == Some(visit.flow))
}

pub(crate) fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = script::owner_of(world, target) else {
        return;
    };
    let Some(may_do) = world.resource::<RiftInstances>().may_do(owner) else {
        return;
    };
    let content = world.resource::<Content>();
    for reach in &may_do.reaches {
        if let Reach::OpensShop(shop) = reach {
            let mark = shop.get(content).mark;
            for &player in viewers {
                out(player, mark);
            }
        }
    }
}

fn traded(world: &mut World, player: Entity, shop: ShopId, kind: EventKind, stack: ItemStack) {
    world.resource_mut::<RiftInstances>().raise(
        Owner::Row(RunnerKind::Shop, shop.raw()),
        kind,
        Some(player),
        vec![
            Value::Int(i64::from(stack.item.raw())),
            Value::Int(i64::from(stack.count)),
        ],
        move |event| match event {
            RiftEvent::Bought { item } | RiftEvent::Sold { item } => {
                item.is_none_or(|item| item == stack.item)
            }
            _ => false,
        },
    );
}

fn buy(world: &mut World, player: Entity, index: u32) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let Some(offer) = shop.get(&content).sells().nth(index as usize) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let bought = match offer_refusal(world, player, shop, index, now) {
        Some(refusal) => Err(refusal.describe(&content)),
        None => trade(
            world,
            player,
            &offer.price,
            &[ItemStack::new(offer.item, offer.count)],
        ),
    };
    if let Err(refusal) = bought {
        notification::error(world, player, refusal);
        react(world, player, |reactions| &reactions.cant_afford);
        return;
    }
    if let Stock::Limited { count, restock } = offer.stock {
        let mut ledger = world.entity_mut(player);
        let mut ledger = ledger.entry::<ShopLedger>().or_default().into_mut();
        let entry = ledger.stock.entry((shop, index)).or_insert(Restock {
            left: count,
            full_at: now,
        });
        if now >= entry.full_at {
            *entry = Restock {
                left: count,
                full_at: now.after(restock),
            };
        }
        entry.left = entry.left.saturating_sub(1);
    }
    react(world, player, |reactions| &reactions.bought);
    traded(
        world,
        player,
        shop,
        EventKind::Bought,
        ItemStack::new(offer.item, offer.count),
    );
    refresh_view(world, player);
}

fn sell(world: &mut World, player: Entity, slot: u32, stack: ItemStack) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let held = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(slot as usize).copied());
    if held != Some(stack) {
        return;
    }
    let declined = declined(world, player, shop);
    let paid: Vec<ItemStack> = match shop.get(&content).pays_for(&content, stack.item, &declined) {
        Ok(pays) => pays
            .iter()
            .map(|pay| ItemStack::new(pay.item, pay.count.saturating_mul(stack.count)))
            .collect(),
        Err(refusal) => {
            notification::error(world, player, refusal);
            return;
        }
    };
    if let Err(refusal) = trade(world, player, &[stack], &paid) {
        notification::error(world, player, refusal);
        return;
    }
    let mut ledger = world.entity_mut(player);
    let mut ledger = ledger.entry::<ShopLedger>().or_default().into_mut();
    ledger.record_sale(
        shop,
        Sale {
            item: stack.item,
            count: stack.count,
            paid,
        },
    );
    react(world, player, |reactions| &reactions.sold);
    traded(world, player, shop, EventKind::Sold, stack);
    refresh_view(world, player);
}

fn buy_back(world: &mut World, player: Entity, index: u32) {
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let Some(sale) = world
        .get::<ShopLedger>(player)
        .and_then(|ledger| ledger.sales.get(&shop))
        .and_then(|sales| sales.get(index as usize))
        .cloned()
    else {
        return;
    };
    let bought = trade(
        world,
        player,
        &sale.paid,
        &[ItemStack::new(sale.item, sale.count)],
    );
    if let Err(refusal) = bought {
        notification::error(world, player, refusal);
        return;
    }
    if let Some(mut ledger) = world.get_mut::<ShopLedger>(player)
        && let Some(sales) = ledger.sales.get_mut(&shop)
    {
        sales.remove(index as usize);
    }
    refresh_view(world, player);
}

fn trade(
    world: &mut World,
    player: Entity,
    takes: &[ItemStack],
    gives: &[ItemStack],
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "You have no bag".to_owned())?;
    inventory
        .exchange(&content, takes, gives)
        .map_err(|refusal| refusal.describe(&content))
}

fn held(inventory: &Inventory, price: &[ItemStack]) -> Vec<u32> {
    price
        .iter()
        .map(|stack| inventory.count(stack.item))
        .collect()
}

fn trade_refusal(
    content: &Content,
    inventory: &Inventory,
    price: &[ItemStack],
    bought: &[ItemStack],
) -> Option<WareRefusal> {
    inventory
        .clone()
        .exchange(content, price, bought)
        .err()
        .map(|refusal| WareRefusal::of(refusal, content))
}

fn offer_refusal(
    world: &World,
    player: Entity,
    shop: ShopId,
    index: u32,
    now: UnixMillis,
) -> Option<WareRefusal> {
    let content = world.resource::<Content>();
    let offer = shop.get(content).sells().nth(index as usize)?;
    if let Some(unmet) = condition::first_unmet(world, player, &offer.requires) {
        return Some(WareRefusal::Unavailable(format!("Needs {unmet}")));
    }
    if stock(world, player, shop, index, now).is_some_and(|(left, _)| left == 0) {
        return Some(WareRefusal::Unavailable("Sold out for now".to_owned()));
    }
    trade_refusal(
        content,
        world.get::<Inventory>(player)?,
        &offer.price,
        &[ItemStack::new(offer.item, offer.count)],
    )
}

fn stock(
    world: &World,
    player: Entity,
    shop: ShopId,
    index: u32,
    now: UnixMillis,
) -> Option<(u32, Option<Seconds>)> {
    let content = world.resource::<Content>();
    let Stock::Limited { count, .. } = shop.get(content).sells().nth(index as usize)?.stock else {
        return None;
    };
    let restock = world
        .get::<ShopLedger>(player)
        .and_then(|ledger| ledger.stock.get(&(shop, index)).copied())
        .filter(|restock| now < restock.full_at);
    Some(match restock {
        Some(restock) => (
            restock.left,
            Some(restock.full_at.since(now).up_to_minute()),
        ),
        None => (count, None),
    })
}

fn refresh_view(world: &mut World, player: Entity) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let inventory = world
        .get::<Inventory>(player)
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let offers = shop
        .get(&content)
        .sells()
        .enumerate()
        .map(|(index, offer)| {
            let index = index as u32;
            let stock = stock(world, player, shop, index, now);
            OfferView {
                left: stock.map(|(left, _)| left),
                restocks_in: stock.and_then(|(_, restocks_in)| restocks_in),
                refusal: offer_refusal(world, player, shop, index, now),
                held: held(&inventory, &offer.price),
            }
        })
        .collect();
    let buyback = world
        .get::<ShopLedger>(player)
        .and_then(|ledger| ledger.sales.get(&shop))
        .map(|sales| {
            sales
                .iter()
                .map(|sale| BuybackView {
                    sale: sale.clone(),
                    refusal: trade_refusal(&content, &inventory, &sale.paid, &[]),
                    held: held(&inventory, &sale.paid),
                })
                .collect()
        })
        .unwrap_or_default();
    let quotes = sale_quotes(&content, shop, &declined(world, player, shop), &inventory);
    let view = ShopView {
        shop,
        offers,
        buyback,
        quotes,
    };
    if world.get::<ShopView>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

fn sale_quotes(
    content: &Content,
    shop: ShopId,
    declined: &[u32],
    inventory: &Inventory,
) -> Vec<SaleQuote> {
    let mut items: Vec<ItemId> = Vec::new();
    for stack in &inventory.slots {
        if !items.contains(&stack.item) {
            items.push(stack.item);
        }
    }
    items
        .into_iter()
        .map(|item| SaleQuote {
            item,
            verdict: match shop.get(content).pays_for(content, item, declined) {
                Ok(pays) => SaleVerdict::Pays(pays.to_vec()),
                Err(refusal) => SaleVerdict::Refused(refusal),
            },
        })
        .collect()
}

fn declined(world: &World, player: Entity, shop: ShopId) -> Vec<u32> {
    let content = world.resource::<Content>();
    (0..)
        .zip(shop.get(content).buys())
        .filter(|(_, rule)| {
            rule.requires
                .as_ref()
                .is_some_and(|requires| !condition::holds(world, player, requires))
        })
        .map(|(index, _)| index)
        .collect()
}

fn react(world: &mut World, player: Entity, pick: fn(&ShopReactions) -> &[Line]) {
    let content = world.resource::<Content>().clone();
    let Some((shop, nth)) = world.get_mut::<ShopVisit>(player).map(|mut visit| {
        visit.reactions += 1;
        (visit.shop, visit.reactions)
    }) else {
        return;
    };
    let (Some(lines), Some(keeper)) = (
        shop.get(&content).reactions().map(pick),
        dialogue::counterpart_of(world, player),
    ) else {
        return;
    };
    if let Some(line) = lines.get(nth as usize % lines.len().max(1)) {
        dialogue::remark(world, player, keeper, line);
    }
}
