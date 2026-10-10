pub mod counter;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::content::Content;
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::data::attention::Id as AttentionId;
use crate::data::dialogue::Id as DialogueId;
use crate::data::item::Id as ItemId;
use crate::systems::attention;
use crate::systems::dialogue::{self, Asked, ChoiceTag, Line, Offer, Then};
use crate::systems::interact::Counterpart;
use crate::systems::item::{ExchangeRefusal, Inventory, ItemCategory, ItemFlag, ItemStack};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::sender_player;
use crate::systems::rule::{self, Encounter, Outcome, Requirement, RuleContext};
use crate::systems::text::{LineText, Span};

pub use crate::data::shop::Id as ShopId;

const BUYBACK_KEPT: usize = 12;
const MINUTE: f32 = 60.0;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<ShopView>()
        .add_client_message::<ShopRequest>(Channel::Ordered);
    dialogue::topic_source(app, wares);
    attention::mark_source(app, marks);
}

#[derive(Clone)]
pub struct ShopDef {
    pub title: &'static str,
    pub keeper: Counterpart,
    pub requires: &'static [&'static dyn Requirement],
    pub mark: AttentionId,
    pub ask: Option<&'static [Span]>,
    pub browsing: DialogueId,
    pub sells: &'static [ShopOffer],
    pub buys: &'static [ShopBuys],
    pub reactions: Option<ShopReactions>,
}

impl crate::core::content::ContentRow for ShopDef {
    const TABLE: &'static str = "shop";
}

pub struct ShopOffer {
    pub item: ItemId,
    pub count: u32,
    pub price: &'static [ItemStack],
    pub stock: Stock,
    pub requires: &'static [&'static dyn Requirement],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stock {
    Unlimited,
    Limited { count: u32, restock: Seconds },
}

pub struct ShopBuys {
    pub what: Buys,
    pub pays: &'static [ItemStack],
    pub requires: &'static [&'static dyn Requirement],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buys {
    Item(ItemId),
    Category(ItemCategory),
}

#[derive(Clone)]
pub struct ShopReactions {
    pub bought: &'static [Line],
    pub sold: &'static [Line],
    pub cant_afford: &'static [Line],
}

pub struct OpenShop(pub ShopId);

impl Outcome for OpenShop {
    fn apply(&self, ctx: &mut RuleContext) {
        let content = ctx.world.resource::<Content>().clone();
        let browsing = self.0.get(&content).browsing;
        if dialogue::in_conversation(ctx.world, ctx.player) {
            dialogue::goto(ctx.world, ctx.player, browsing);
        } else {
            let start = dialogue::Start {
                node: browsing,
                with: ctx.encounter.with,
                tether: ctx.encounter.tether,
            };
            dialogue::start(ctx.world, ctx.player, start);
        }
        open(ctx.world, ctx.player, self.0, ctx.encounter);
    }

    fn leads_to(&self, content: &Content) -> Vec<DialogueId> {
        vec![self.0.get(content).browsing]
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShopView {
    pub shop: ShopId,
    pub offers: Vec<OfferView>,
    pub buyback: Vec<Sale>,
    pub declined: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OfferView {
    pub left: Option<u32>,
    pub restocks_in: Option<Seconds>,
    pub refusal: Option<WareRefusal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ShopRequest {
    Buy { offer: u32 },
    Sell { slot: u32, stack: ItemStack },
    Buyback { sale: u32 },
}

#[derive(Component)]
pub struct ShopVisit {
    shop: ShopId,
    encounter: Encounter,
    reactions: u32,
}

impl ShopDef {
    pub fn pays_for(
        &self,
        content: &Content,
        item: ItemId,
        declined: &[u32],
    ) -> Result<&'static [ItemStack], String> {
        let def = item.get(content);
        if def.has(ItemFlag::Quest) {
            return Err("Quest items never sell".to_owned());
        }
        if def.bound() {
            return Err("Bound items never sell".to_owned());
        }
        let (index, rule) = self
            .buys
            .iter()
            .enumerate()
            .find(|(_, rule)| match rule.what {
                Buys::Item(wanted) => wanted == item,
                Buys::Category(category) => def.category() == category,
            })
            .ok_or_else(|| format!("{} won't buy {}", self.title, def.display_name))?;
        if declined.contains(&(index as u32)) {
            return Err(format!(
                "{} won't buy {} from you",
                self.title, def.display_name
            ));
        }
        Ok(rule.pays)
    }

    pub fn wants(&self, content: &Content) -> Vec<String> {
        self.buys
            .iter()
            .map(|rule| match rule.what {
                Buys::Item(item) => item.get(content).display_name.to_owned(),
                Buys::Category(category) => category.plural().to_owned(),
            })
            .collect()
    }
}

fn open(world: &mut World, player: Entity, shop: ShopId, encounter: Encounter) {
    if !dialogue::engaged_with(world, player, encounter.with) {
        return;
    }
    world.entity_mut(player).insert(ShopVisit {
        shop,
        encounter,
        reactions: 0,
    });
    refresh_view(world, player);
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
    for request in crate::systems::requests::<ShopRequest>(world) {
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

pub fn conversation_starts(content: &Content) -> Vec<DialogueId> {
    content
        .table::<ShopDef>()
        .rows()
        .iter()
        .map(|shop| shop.browsing)
        .collect()
}

pub fn check(assets: &AssetService) {
    let content = assets.content();
    let shops = content.table::<ShopDef>();
    for (id, shop) in shops.iter() {
        if shops
            .iter()
            .any(|(other, def)| other != id && def.keeper == shop.keeper)
        {
            panic!("shop {id:?}: its keeper keeps another shop too");
        }
        for line in shop.reactions.iter().flat_map(|reactions| {
            reactions
                .bought
                .iter()
                .chain(reactions.sold)
                .chain(reactions.cant_afford)
        }) {
            dialogue::check_line(content, format!("shop {id:?}"), line);
        }
        for offer in shop.sells {
            for (buyer, def) in shops.iter() {
                if let Ok(pays) = def.pays_for(content, offer.item, &[])
                    && profits(offer, pays)
                {
                    panic!(
                        "shop {buyer:?} buys {:?} for more than shop {id:?} sells it",
                        offer.item
                    );
                }
            }
        }
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
    let spent = |item| amount(offer.price, item, 1);
    items.iter().all(|&item| earned(item) >= spent(item))
        && items.iter().any(|&item| earned(item) > spent(item))
}

fn bound(world: &World, player: Entity) -> bool {
    world
        .get::<ShopVisit>(player)
        .is_some_and(|visit| dialogue::engaged_with(world, player, visit.encounter.with))
}

fn wares(world: &World, asked: &Asked) -> Vec<Offer> {
    let content = world.resource::<Content>();
    let Some(&keeper) = asked
        .greeting(content)
        .and_then(|with| world.get::<Counterpart>(with))
    else {
        return Vec::new();
    };
    let Some((id, shop)) = content
        .table::<ShopDef>()
        .iter()
        .find(|(_, shop)| shop.keeper == keeper)
    else {
        return Vec::new();
    };
    let Some(ask) = shop
        .ask
        .filter(|_| rule::met(world, asked.player, shop.requires))
    else {
        return Vec::new();
    };
    let then: Vec<Arc<dyn Outcome>> = vec![Arc::new(OpenShop(id))];
    vec![Offer {
        label: LineText::spoken(ask, world, asked.player),
        tag: ChoiceTag::Shop,
        icon: Some(shop.mark.get(content).icon),
        requires: Vec::new(),
        costs: &[],
        then: Then::Made(then),
        reveal: &[],
        warn: None,
    }]
}

fn marks(world: &World, player: Entity, target: Entity) -> Vec<AttentionId> {
    let Some(&keeper) = world.get::<Counterpart>(target) else {
        return Vec::new();
    };
    world
        .resource::<Content>()
        .table::<ShopDef>()
        .rows()
        .iter()
        .filter(|shop| shop.keeper == keeper && rule::met(world, player, shop.requires))
        .map(|shop| shop.mark)
        .collect()
}

fn buy(world: &mut World, player: Entity, index: u32) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let Some(offer) = shop.get(&content).sells.get(index as usize) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let bought = match offer_refusal(world, player, shop, index, now) {
        Some(refusal) => Err(refusal.describe(&content)),
        None => trade(
            world,
            player,
            offer.price,
            &[ItemStack::new(offer.item, offer.count)],
        ),
    };
    if let Err(refusal) = bought {
        notification::notify(
            world,
            player,
            Notification::new(NotificationKind::error(), LineText::plain(refusal)),
        );
        react(world, player, |reactions| reactions.cant_afford);
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
    react(world, player, |reactions| reactions.bought);
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
            notification::notify(
                world,
                player,
                Notification::new(NotificationKind::error(), LineText::plain(refusal)),
            );
            return;
        }
    };
    if let Err(refusal) = trade(world, player, &[stack], &paid) {
        notification::notify(
            world,
            player,
            Notification::new(NotificationKind::error(), LineText::plain(refusal)),
        );
        return;
    }
    let mut ledger = world.entity_mut(player);
    let mut ledger = ledger.entry::<ShopLedger>().or_default().into_mut();
    let sales = ledger.sales.entry(shop).or_default();
    sales.push_front(Sale {
        item: stack.item,
        count: stack.count,
        paid,
    });
    sales.truncate(BUYBACK_KEPT);
    react(world, player, |reactions| reactions.sold);
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
        notification::notify(
            world,
            player,
            Notification::new(NotificationKind::error(), LineText::plain(refusal)),
        );
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

fn offer_refusal(
    world: &World,
    player: Entity,
    shop: ShopId,
    index: u32,
    now: UnixMillis,
) -> Option<WareRefusal> {
    let content = world.resource::<Content>();
    let offer = shop.get(content).sells.get(index as usize)?;
    if let Some(unmet) = offer
        .requires
        .iter()
        .find(|requirement| !requirement.met(world, player))
    {
        return Some(WareRefusal::Unavailable(format!(
            "Needs {}",
            unmet.describe(content)
        )));
    }
    if stock(world, player, shop, index, now).is_some_and(|(left, _)| left == 0) {
        return Some(WareRefusal::Unavailable("Sold out for now".to_owned()));
    }
    let mut inventory = world.get::<Inventory>(player)?.clone();
    inventory
        .exchange(
            content,
            offer.price,
            &[ItemStack::new(offer.item, offer.count)],
        )
        .err()
        .map(|refusal| WareRefusal::of(refusal, content))
}

fn stock(
    world: &World,
    player: Entity,
    shop: ShopId,
    index: u32,
    now: UnixMillis,
) -> Option<(u32, Option<Seconds>)> {
    let content = world.resource::<Content>();
    let Stock::Limited { count, .. } = shop.get(content).sells.get(index as usize)?.stock else {
        return None;
    };
    let restock = world
        .get::<ShopLedger>(player)
        .and_then(|ledger| ledger.stock.get(&(shop, index)).copied())
        .filter(|restock| now < restock.full_at);
    Some(match restock {
        Some(restock) => {
            let minutes = (restock.full_at.since(now).0 / MINUTE).ceil();
            (restock.left, Some(Seconds(minutes * MINUTE)))
        }
        None => (count, None),
    })
}

fn refresh_view(world: &mut World, player: Entity) {
    let content = world.resource::<Content>().clone();
    let Some(shop) = visiting(world, player) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let offers = (0..shop.get(&content).sells.len() as u32)
        .map(|index| {
            let stock = stock(world, player, shop, index, now);
            OfferView {
                left: stock.map(|(left, _)| left),
                restocks_in: stock.and_then(|(_, restocks_in)| restocks_in),
                refusal: offer_refusal(world, player, shop, index, now),
            }
        })
        .collect();
    let buyback = world
        .get::<ShopLedger>(player)
        .and_then(|ledger| ledger.sales.get(&shop))
        .map(|sales| sales.iter().cloned().collect())
        .unwrap_or_default();
    let view = ShopView {
        shop,
        offers,
        buyback,
        declined: declined(world, player, shop),
    };
    if world.get::<ShopView>(player) != Some(&view) {
        world.entity_mut(player).insert(view);
    }
}

fn declined(world: &World, player: Entity, shop: ShopId) -> Vec<u32> {
    let content = world.resource::<Content>();
    (0..)
        .zip(shop.get(content).buys)
        .filter(|(_, rule)| !rule::met(world, player, rule.requires))
        .map(|(index, _)| index)
        .collect()
}

fn react(world: &mut World, player: Entity, pick: fn(&ShopReactions) -> &'static [Line]) {
    let content = world.resource::<Content>().clone();
    let Some((shop, nth, keeper)) = world.get_mut::<ShopVisit>(player).map(|mut visit| {
        visit.reactions += 1;
        (visit.shop, visit.reactions, visit.encounter.with)
    }) else {
        return;
    };
    let (Some(lines), Some(keeper)) = (shop.get(&content).reactions.as_ref().map(pick), keeper)
    else {
        return;
    };
    if let Some(line) = lines.get(nth as usize % lines.len().max(1)) {
        dialogue::remark(world, player, keeper, line);
    }
}
