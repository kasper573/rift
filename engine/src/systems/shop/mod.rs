pub mod counter;

use crate::core::script::EventDef;
use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::RunnerSpec;
use crate::core::script::Scripts;
use crate::core::script::ValueType;
use crate::core::script::Vocabulary;
use crate::systems::requirement;
use std::collections::{HashMap, VecDeque};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, serial,
};
use crate::core::script::DialogueCamera;
use crate::core::script::{
    Answer, Arrow, CLOSED, Compiler, EffectDef, EffectStep, ScriptFault, Value,
};
use crate::core::script::{
    EffectCall, EventKind, Expr, FlowKey, Instances, Owner, RunnerKind, ScriptModule, ScriptedRow,
    condition,
};
use crate::core::text::LineText;
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::Seconds;
use crate::systems::aim as effects;
use crate::systems::area::TimeOfDayName;
use crate::systems::attention::AttentionId;
use crate::systems::dialogue::ScriptLine;
use crate::systems::item::{
    self, ExchangeRefusal, Inventory, ItemCategory, ItemId, ItemModuleKind, ItemStack, SlotStack,
};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::player::conn_player;
use crate::systems::window::{AnswerCount, OpenWindow};
use crate::systems::{dialogue, drain_requests, notification};

pub type ShopId = ContentId<ShopDef>;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate_private::<ShopView>()
        .add_client_message::<ShopRequest>();
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopDef {
    pub title: String,
    pub mark: AttentionId,
    /// How many of the wares a player last sold here they may buy back; without any, nothing
    /// sold here comes back.
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub buys_back: u32,
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
            buys_back: 0,
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
    const OWNER: RunnerKind = SHOP_RUNNER;

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
    Sells(ShopWare),
    Buys(ShopBuys),
    Reactions(ShopReactions),
    Script(ScriptModule),
}

impl ShopDef {
    pub fn sells(&self) -> impl Iterator<Item = &ShopWare> + Clone {
        self.modules.iter().filter_map(|module| match module {
            ShopModule::Sells(ware) => Some(ware),
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
    ) -> Result<&[ItemStack], SaleRefusal> {
        let def = item.get(content);
        if def.has(ItemModuleKind::QuestItem) {
            return Err(SaleRefusal::QuestItem);
        }
        if def.bound() {
            return Err(SaleRefusal::Bound);
        }
        let (index, rule) = self
            .buys()
            .enumerate()
            .find(|(_, rule)| match rule.what {
                Buys::Item { item: wanted } => wanted == item,
                Buys::Category { category } => def.category() == category,
            })
            .ok_or(SaleRefusal::NoBuyer)?;
        if declined.contains(&(index as u32)) {
            return Err(SaleRefusal::Declined);
        }
        Ok(&rule.pays)
    }

    pub fn currencies(&self) -> Vec<ItemId> {
        let mut currencies = Vec::new();
        let prices = self.sells().flat_map(|ware| &ware.price);
        let pays = self.buys().flat_map(|rule| &rule.pays);
        for stack in prices.chain(pays) {
            if !currencies.contains(&stack.item) {
                currencies.push(stack.item);
            }
        }
        currencies
    }

    pub fn wants(&self, content: &Content) -> Vec<LineText> {
        self.buys()
            .map(|rule| match rule.what {
                Buys::Item { item } => LineText::from(Mention::Item(item).span(content)),
                Buys::Category { category } => LineText::from(
                    messages::of(content)
                        .item
                        .category_plural(<&str>::from(category)),
                ),
            })
            .collect()
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopWare {
    pub item: ItemId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub price: Vec<ItemStack>,
    pub stock: ShopStock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShopStock {
    Unlimited,
    Limited { count: u32, restock: Seconds },
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopBuys {
    pub what: Buys,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pays: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Buys {
    Item { item: ItemId },
    Category { category: ItemCategory },
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ShopReactions {
    pub bought: Vec<ScriptLine>,
    pub sold: Vec<ScriptLine>,
    pub cant_afford: Vec<ScriptLine>,
}

/// Opens the shop until it closes.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("shop"))]
pub struct OpenShop {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub shop: ShopId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub closed: Arrow,
}

impl EffectDef for OpenShop {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(CLOSED => closed);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (shop, camera) = (self.shop, cx.camera());
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Done;
            };
            if open(world, player, shop, flow, camera) {
                EffectStep::Pause
            } else {
                EffectStep::Done
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(OpensShop(self.shop))]
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShopView {
    pub shop: ShopId,
    pub wares: Vec<WareView>,
    pub buyback: Vec<BuybackView>,
    pub quotes: Vec<SaleQuote>,
    pub answered: AnswerCount,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub struct SaleQuote {
    pub item: ItemId,
    pub verdict: SaleVerdict,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Hash)]
pub enum SaleVerdict {
    Pays(Vec<ItemStack>),
    Refused(SaleRefusal),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaleRefusal {
    QuestItem,
    Bound,
    NoBuyer,
    Declined,
}

impl SaleRefusal {
    pub fn line(self, content: &Content, shop: &ShopDef, item: ItemId) -> LineText {
        let words = &messages::of(content).shop;
        let named = Mention::Item(item).span(content);
        match self {
            SaleRefusal::QuestItem => words.quest_items_never_sell().into(),
            SaleRefusal::Bound => ItemStack::bound_line(item, content),
            SaleRefusal::NoBuyer => words.wont_buy(named, shop.title.as_str()),
            SaleRefusal::Declined => words.wont_buy_from_you(named, shop.title.as_str()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WareView {
    pub left: Option<u32>,
    pub restocks_in: Option<Seconds>,
    pub refusal: Option<WareRefusal>,
    pub held: Vec<u32>,
    /// The times of day the ware is for sale by, for a player to see why it comes and goes.
    pub times: Vec<TimeOfDayName>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BuybackView {
    pub sale: Sale,
    pub refusal: Option<WareRefusal>,
    pub held: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum WareRefusal {
    Unaffordable(ItemStack),
    NoRoom { slots: u32 },
    SoldOut,
    Unavailable(LineText),
}

impl WareRefusal {
    pub fn line(&self, content: &Content) -> LineText {
        match self {
            WareRefusal::Unaffordable(short) => ExchangeRefusal::Missing(*short).line(content),
            WareRefusal::NoRoom { slots } => {
                ExchangeRefusal::NoRoom { slots: *slots }.line(content)
            }
            WareRefusal::SoldOut => messages::of(content).shop.sold_out_for_now().into(),
            WareRefusal::Unavailable(requirement) => {
                messages::of(content).shop.needs(requirement.clone())
            }
        }
    }

    fn of(refusal: ExchangeRefusal) -> WareRefusal {
        match refusal {
            ExchangeRefusal::Missing(short) => WareRefusal::Unaffordable(short),
            ExchangeRefusal::NoRoom { slots } => WareRefusal::NoRoom { slots },
        }
    }
}

/// What a shop that pays `pays` for one of an item pays for `count` of it.
pub fn paid_for(pays: &[ItemStack], count: u32) -> Vec<ItemStack> {
    pays.iter()
        .map(|pay| ItemStack::new(pay.item, pay.count.saturating_mul(count)))
        .collect()
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
    pub fn stocked(&mut self, shop: ShopId, ware: u32, left: u32, full_at: UnixMillis) {
        self.stock.insert((shop, ware), Restock { left, full_at });
    }

    /// What is left of every limited ware bought from, by shop and ware, and when it is whole
    /// again.
    pub fn stock(&self) -> impl Iterator<Item = (ShopId, u32, u32, UnixMillis)> + '_ {
        self.stock
            .iter()
            .map(|(&(shop, ware), restock)| (shop, ware, restock.left, restock.full_at))
    }

    /// The sales each shop keeps for buying back, the earliest first.
    pub fn sales(&self) -> impl Iterator<Item = (ShopId, impl Iterator<Item = &Sale>)> {
        self.sales
            .iter()
            .map(|(&shop, sales)| (shop, sales.iter().rev()))
    }

    /// Records a sale at `shop`, which keeps `kept` of the latest for buying back.
    pub fn record_sale(&mut self, shop: ShopId, sale: Sale, kept: u32) {
        if kept == 0 {
            return;
        }
        let sales = self.sales.entry(shop).or_default();
        sales.push_front(sale);
        sales.truncate(kept as usize);
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ShopRequest {
    Buy {
        ware: u32,
    },
    /// Sells every one of `stacks` or none of them, each out of the slot of the bag it names.
    Sell {
        stacks: Vec<SlotStack>,
    },
    Buyback {
        sale: u32,
    },
}

#[derive(Component)]
pub struct ShopVisit {
    shop: ShopId,
    flow: FlowKey,
    reactions: u32,
    answered: AnswerCount,
}

fn open(
    world: &mut World,
    player: Entity,
    shop: ShopId,
    flow: FlowKey,
    camera: DialogueCamera,
) -> bool {
    if !dialogue::card(
        world,
        flow,
        player,
        Vec::new(),
        Answer::Output(CLOSED),
        None,
        camera,
    ) {
        return false;
    }
    world.entity_mut(player).insert(ShopVisit {
        shop,
        flow,
        reactions: 0,
        answered: AnswerCount::default(),
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
        let Some(player) = conn_player(world, request.client) else {
            continue;
        };
        if !bound(world, player) {
            continue;
        }
        match request.message {
            ShopRequest::Buy { ware } => buy(world, player, ware),
            ShopRequest::Sell { stacks } => sell(world, player, &stacks),
            ShopRequest::Buyback { sale } => buy_back(world, player, sale),
        }
        if let Some(mut visit) = world.get_mut::<ShopVisit>(player) {
            visit.answered.raise();
        }
        refresh_view(world, player);
    }
}

impl OpenWindow for ShopVisit {
    fn kept(&self, world: &World, player: Entity) -> bool {
        dialogue::flow_of(world, player) == Some(self.flow)
    }

    fn close(world: &mut World, player: Entity) {
        close(world, player);
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
                ShopModule::Sells(ware) => ware.requires.as_ref(),
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
            if let ShopModule::Reactions(reactions) = module {
                for (to, lines) in [
                    ("bought", &reactions.bought),
                    ("sold", &reactions.sold),
                    ("cant_afford", &reactions.cant_afford),
                ] {
                    for (line, fault) in lines
                        .iter()
                        .enumerate()
                        .filter_map(|(line, said)| Some((line, said.fault(&content)?)))
                    {
                        loading.row_error(
                            id,
                            &format!("/modules/{index}/{to}/{line}/by/gesture"),
                            ContentProblem::new(fault.rule, "the line", fault.expected),
                        );
                    }
                }
            }
            let ShopModule::Sells(ware) = module else {
                continue;
            };
            for &(buyer, def) in &shops {
                if let Ok(pays) = def.pays_for(&content, ware.item, &[])
                    && profits(ware, pays)
                {
                    loading.row_error(
                        id,
                        &format!("/modules/{index}"),
                        ContentProblem::new(
                            "shop loses money",
                            format!(
                                "{}: {} buys {} for {}, sold for {}",
                                ware.item.path(&content),
                                buyer.path(&content),
                                ware.count,
                                stacks_text(&content, pays, ware.count),
                                stacks_text(&content, &ware.price, 1)
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

fn profits(ware: &ShopWare, pays: &[ItemStack]) -> bool {
    let amount = |stacks: &[ItemStack], item: ItemId, times: u32| {
        stacks
            .iter()
            .filter(|stack| stack.item == item)
            .map(|stack| stack.count.saturating_mul(times))
            .sum::<u32>()
    };
    let items: Vec<ItemId> = ware
        .price
        .iter()
        .chain(pays)
        .map(|stack| stack.item)
        .collect();
    let earned = |item| amount(pays, item, ware.count);
    let spent = |item| amount(&ware.price, item, 1);
    items.iter().all(|&item| earned(item) >= spent(item))
        && items.iter().any(|&item| earned(item) > spent(item))
}

fn bound(world: &World, player: Entity) -> bool {
    world
        .get::<ShopVisit>(player)
        .is_some_and(|visit| visit.kept(world, player))
}

pub(crate) fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = crate::systems::area::script::owner_of(world, target) else {
        return;
    };
    let Some(may_do) = world.resource::<Instances>().may_do(owner) else {
        return;
    };
    let content = world.resource::<Content>();
    for OpensShop(shop) in may_do.reaching::<OpensShop>() {
        let mark = shop.get(content).mark;
        for &player in viewers {
            out(player, mark);
        }
    }
}

fn announce(world: &mut World, player: Entity, shop: ShopId, kind: EventKind, stack: ItemStack) {
    world.resource_mut::<Instances>().raise(
        Owner::Row(SHOP_RUNNER, shop.raw()),
        kind,
        Some(player),
        vec![
            Value::Int(i64::from(stack.item.raw())),
            Value::Int(i64::from(stack.count)),
        ],
        move |event| {
            event
                .downcast_ref::<Bought>()
                .map(|trade| trade.item)
                .or_else(|| event.downcast_ref::<Sold>().map(|trade| trade.item))
                .is_some_and(|item| item.is_none_or(|item| item == stack.item))
        },
    );
}

fn buy(world: &mut World, player: Entity, index: u32) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let Some(ware) = shop.get(&content).sells().nth(index as usize) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let bought = match ware_refusal(world, player, shop, index, now) {
        Some(refusal) => Err(refusal.line(&content)),
        None => item::effects::exchange(
            world,
            player,
            &ware.price,
            &[ItemStack::new(ware.item, ware.count)],
        ),
    };
    if let Err(refusal) = bought {
        notification::error(world, player, refusal);
        react(world, player, |reactions| &reactions.cant_afford);
        return;
    }
    if let ShopStock::Limited { count, restock } = ware.stock {
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
    announce(
        world,
        player,
        shop,
        EventKind::of::<Bought>(),
        ItemStack::new(ware.item, ware.count),
    );
}

fn sell(world: &mut World, player: Entity, stacks: &[SlotStack]) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let Some(mut inventory) = world.get::<Inventory>(player).cloned() else {
        return;
    };
    if stacks.is_empty() || !inventory.take_out(stacks) {
        return;
    }
    let declined = declined(world, player, shop);
    let mut sales = Vec::new();
    for SlotStack { stack, .. } in stacks {
        match shop.get(&content).pays_for(&content, stack.item, &declined) {
            Ok(pays) => sales.push(Sale {
                item: stack.item,
                count: stack.count,
                paid: paid_for(pays, stack.count),
            }),
            Err(refusal) => {
                let line = refusal.line(&content, shop.get(&content), stack.item);
                notification::error(world, player, line);
                return;
            }
        }
    }
    let paid: Vec<ItemStack> = sales
        .iter()
        .flat_map(|sale| sale.paid.iter().copied())
        .collect();
    if let Err(refusal) = inventory.exchange(&content, &[], &paid) {
        notification::error(world, player, refusal.line(&content));
        return;
    }
    if let Some(mut held) = world.get_mut::<Inventory>(player) {
        *held = inventory;
    }
    let kept = shop.get(&content).buys_back;
    let mut ledger = world.entity_mut(player);
    let mut ledger = ledger.entry::<ShopLedger>().or_default().into_mut();
    for sale in &sales {
        ledger.record_sale(shop, sale.clone(), kept);
    }
    react(world, player, |reactions| &reactions.sold);
    for sale in sales {
        let sold = ItemStack::new(sale.item, sale.count);
        announce(world, player, shop, EventKind::of::<Sold>(), sold);
    }
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
    let bought = item::effects::exchange(
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
}

pub(super) fn held(inventory: &Inventory, price: &[ItemStack]) -> Vec<u32> {
    price
        .iter()
        .map(|stack| inventory.count(stack.item))
        .collect()
}

pub(super) fn exchange_refusal(
    content: &Content,
    inventory: &Inventory,
    price: &[ItemStack],
    bought: &[ItemStack],
) -> Option<WareRefusal> {
    inventory
        .clone()
        .exchange(content, price, bought)
        .err()
        .map(WareRefusal::of)
}

fn ware_refusal(
    world: &World,
    player: Entity,
    shop: ShopId,
    index: u32,
    now: UnixMillis,
) -> Option<WareRefusal> {
    let content = world.resource::<Content>();
    let ware = shop.get(content).sells().nth(index as usize)?;
    if let Some(unmet) = requirement::first_unmet(world, player, &ware.requires) {
        return Some(WareRefusal::Unavailable(unmet));
    }
    if stock(world, player, shop, index, now).is_some_and(|(left, _)| left == 0) {
        return Some(WareRefusal::SoldOut);
    }
    exchange_refusal(
        content,
        world.get::<Inventory>(player)?,
        &ware.price,
        &[ItemStack::new(ware.item, ware.count)],
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
    let ShopStock::Limited { count, .. } = shop.get(content).sells().nth(index as usize)?.stock
    else {
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
    let Some(visit) = world.get::<ShopVisit>(player) else {
        return;
    };
    let (shop, answered) = (visit.shop, visit.answered);
    let now = world.resource::<WallClock>().now;
    let inventory = Inventory::of(world, player);
    let wares = shop
        .get(&content)
        .sells()
        .enumerate()
        .map(|(index, ware)| {
            let index = index as u32;
            let stock = stock(world, player, shop, index, now);
            WareView {
                left: stock.map(|(left, _)| left),
                restocks_in: stock.and_then(|(_, restocks_in)| restocks_in),
                refusal: ware_refusal(world, player, shop, index, now),
                held: held(&inventory, &ware.price),
                times: ware
                    .requires
                    .as_ref()
                    .map_or_else(Vec::new, crate::systems::area::sky::times),
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
                    refusal: exchange_refusal(
                        &content,
                        &inventory,
                        &sale.paid,
                        &[ItemStack::new(sale.item, sale.count)],
                    ),
                    held: held(&inventory, &sale.paid),
                })
                .collect()
        })
        .unwrap_or_default();
    let quotes = sale_quotes(&content, shop, &declined(world, player, shop), &inventory);
    let view = ShopView {
        shop,
        wares,
        buyback,
        quotes,
        answered,
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
    for (_, stack) in inventory.stacks() {
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

fn react(world: &mut World, player: Entity, pick: fn(&ShopReactions) -> &[ScriptLine]) {
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

messages! {
    /// What the game says at a shop's counter.
    pub struct ShopMessages {
        /// What a ware says of the requirement `{requirement}` a player does not meet.
        needs[requirement: Line],
        /// The tab and the button that buy a ware.
        buy,
        /// The tab and the button that sell what a player placed.
        sell,
        /// The button that buys back what a player sold.
        buy_back,
        /// The tab holding what a player sold, `{count}` of them.
        buy_back_tab(count: Number),
        /// The heading over what a player placed to sell.
        to_sell,
        /// What a shop pays, `{price}`, for what a player placed.
        sale_total[price: Line],
        /// What the sale bar says when nothing is placed to sell.
        click_to_sell,
        /// What the buy back tab says of what a player sold.
        kept_until_leaving,
        /// What the buy back tab says when nothing was sold.
        nothing_sold_yet,
        /// What a tab says when a shop has nothing to show.
        nothing_for_sale,
        /// The button of a ware the player can't pay for.
        cant_afford,
        /// The button of a ware the player's bag has no room for.
        inventory_full,
        /// What a ware that is out of stock says, and the button's name for it.
        sold_out,
        /// The button of a ware that is not available to the player.
        unavailable,
        /// What a ware out of stock says, `{time}` before it is restocked.
        sold_out_back_in(time: Text),
        /// What is said beside the button of a ware out of stock, `{time}` before it is restocked.
        back_in(time: Text),
        /// What a ware in limited stock says, `{left}` of it left and `{time}` before it is restocked.
        left_restocks(left: Number, time: Text),
        /// What a ware in limited stock says, `{left}` of it left.
        left(left: Number),
        /// How full the player's bag is on the counter, `{taken}` of `{slots}` slots taken.
        bag_room(slots: Number, taken: Number),
        /// How full the player's bag is on the counter once it is full, `{taken}` of `{slots}` slots taken.
        bag_room_full(slots: Number, taken: Number),
        /// What a shop buys, `{wants}`.
        buys[wants: Line],
        /// What a shop pays, `{price}`, for an item.
        sells_for[price: Line],
        /// What a player is told of the item `{item}` that the shop `{shop}` has no use for.
        wont_buy[item: Line, shop: Text],
        /// What a player is told of the item `{item}` that the shop `{shop}` won't buy from them.
        wont_buy_from_you[item: Line, shop: Text],
        /// What a player is told of a quest item they try to sell.
        quest_items_never_sell,
        /// What a player is told buying a ware that is out of stock.
        sold_out_for_now,
    }
}

pub const SHOP_RUNNER: RunnerKind = RunnerKind::new("shop");

/// A script that may open a shop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpensShop(pub ShopId);

impl ReachDef for OpensShop {}

const TRADE_FIELDS: &[(&str, ValueType)] =
    &[("item", ValueType::Integer), ("count", ValueType::Integer)];

/// A player bought from the shop, optionally only the item `item`.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Bought {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
}

/// A player sold to the shop, optionally only the item `item`.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Sold {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
}

macro_rules! trade_events {
    ($($event:ident),*) => {$(
        impl EventDef for $event {
            fn owners(&self) -> &'static [RunnerKind] {
                &[SHOP_RUNNER]
            }

            fn fields(&self) -> &'static [(&'static str, ValueType)] {
                TRADE_FIELDS
            }
        }
    )*};
}

trade_events!(Bought, Sold);

/// What a whole game must do with its shops: open every one.
pub fn check_reached(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    let opened: Vec<ShopId> = scripts
        .reaching::<OpensShop>()
        .into_iter()
        .map(|reached| reached.reach.0)
        .collect();
    for (id, _) in loading.rows::<ShopDef>(&content) {
        if !opened.contains(&id) {
            loading.row_error(
                id,
                "",
                ContentProblem::new(
                    "never opened",
                    format!("shop/{}", id.path(&content)),
                    "a script that may reach open_shop for it",
                ),
            );
        }
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .runner(
            SHOP_RUNNER,
            RunnerSpec::named_row::<ShopDef>()
                .stateless()
                .starting_with_world(),
        )
        .binder(|loading, content, bind| bind.bind_table::<ShopDef>(loading, content))
        .event::<Bought>("bought")
        .event::<Sold>("sold")
        .effect::<OpenShop>("open_shop");
}
