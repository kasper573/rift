//! Two players trading: one asks, the other accepts, and each sets out what they give in a window
//! both see from their own side. The trade goes through once both accept the same offer, both
//! offers swapped in one step; until then nothing leaves either bag, so a trade that ends any
//! other way has nothing to undo.

pub mod window;

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::area::transition::Crossing;
use crate::systems::history::HistoryTopic;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::mount::{self, RiderAct};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::payout::{self, PayoutReason};
use crate::systems::player::{self, PlayedBy, Players, RequestNeeds};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::social_request::{
    RequestUnaccepted, RequestUnsent, SocialRequest, SocialRequestAsk, SocialRequestKind,
    SocialRequestSource, SocialRequestWords,
};
use crate::systems::society::{self, Settling, SocietyAsk, SocietyView};
use crate::systems::text::LineText;
use crate::systems::{GameSettings, GameWords, movement, stat, visibility};

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<TradeView>()
        .add_mapped_client_message::<TradeRequest>();
}

/// How a game has players trade. A game without it has no trading.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeSettings {
    /// How close two players stand to ask and to keep trading.
    pub reach: Meters,
    /// How many different items each side may offer.
    pub slots: u32,
    /// How long Accept stays shut after either offer changes, so an accept already on its way
    /// never takes a deal its player has not seen.
    pub accept_waits: Seconds,
}

/// Which change of the offers a trade is at. Every change to either offer makes a new one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeVersion(pub u32);

/// What a player asks of trading.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum TradeRequest {
    /// Walks into reach of the player `with`, and asks them to trade.
    Ask {
        #[entities]
        with: Entity,
    },
    /// Adds this many of an item from the bag to what the player gives.
    Offer(ItemStack),
    /// Takes this many of an item back out of what the player gives.
    Withdraw(ItemStack),
    /// Accepts the deal as it stood at this version.
    Accept(TradeVersion),
    Unaccept,
    Cancel,
}

/// A trade as one of its players sees it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TradeView {
    /// The name of the other player.
    pub with: String,
    pub version: TradeVersion,
    /// What the player gives.
    pub mine: Vec<ItemStack>,
    /// What the other player gives.
    pub theirs: Vec<ItemStack>,
    pub i_accept: bool,
    pub they_accept: bool,
    /// When Accept opens, after the latest change of either offer.
    pub opens: UnixMillis,
}

impl TradeView {
    /// Whether the deal can be accepted at `now`: something is offered, and the latest change
    /// has had its wait.
    pub fn open_at(&self, now: UnixMillis) -> bool {
        (!self.mine.is_empty() || !self.theirs.is_empty()) && now >= self.opens
    }
}

/// Every trade under way in this world.
#[derive(Resource, Default)]
pub struct Trades(Vec<Trade>);

/// Whether `character` is trading.
pub fn trading(world: &World, character: Entity) -> bool {
    world
        .resource::<Trades>()
        .0
        .iter()
        .any(|trade| trade.side_of(character).is_some())
}

/// A trade as a kind of request: it lives as long as its card, needs both players here and
/// within reach of each other, and opens the trade the moment it is accepted.
pub fn request_source() -> SocialRequestSource {
    SocialRequestSource {
        kind: SocialRequestKind::Trade,
        lasts: |settings| settings.trade.map(|_| settings.requests.card_shows),
        reaches_offline: false,
        words: |words| SocialRequestWords {
            asks: &words.trade_asks,
            asked: &words.trade_asked,
            sent: &words.trade_sent,
            accepted: None,
            denied: Some(&words.trade_denied),
            lapsed: None,
        },
        may_send: |_, settings, _, _| match settings.trade {
            Some(_) => Ok(()),
            None => Err(RequestUnsent::Quietly),
        },
        accepted,
        stands,
    }
}

/// Hands what players of this world asked of trading to the trade, or to society.
pub fn requests(world: &mut World) {
    player::player_requests::<TradeRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            match request {
                TradeRequest::Ask { with } => return ask(world, player, with),
                TradeRequest::Offer(stack) => offer(world, player, stack)?,
                TradeRequest::Withdraw(stack) => withdraw(world, player, stack),
                TradeRequest::Accept(version) => accept(world, player, version),
                TradeRequest::Unaccept => unaccept(world, player),
                TradeRequest::Cancel => {
                    if let Some(index) = index_of(world, player) {
                        end(world, index, TradeEnd::Cancelled(player));
                    }
                }
            }
            show(world);
            Ok(())
        },
    );
}

/// Walks everyone on their way to ask another player to trade into reach of them, and asks once
/// there.
pub fn approach(world: &mut World, intents: &mut reach::Intents) {
    let Some(settings) = world.resource::<GameSettings>().trade else {
        return;
    };
    for (asker, with) in reach::intending(world, intents, ReachAct::Trade) {
        if !stat::stands(world, with) {
            reach::forget(world, asker);
            continue;
        }
        match reach::pursue(world, asker, settings.reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, asker),
            Pursuit::Arrived => {
                reach::forget(world, asker);
                send(world, asker, with);
            }
        }
    }
}

/// Ends every trade that can't go on: one of its players left the world or their character,
/// fell, got on a mount they can't trade from, walked out of reach, or no longer holds what they
/// offer.
pub fn hold(world: &mut World) {
    let Some(settings) = world.resource::<GameSettings>().trade else {
        return;
    };
    let mut index = 0;
    while index < world.resource::<Trades>().0.len() {
        let trade = &world.resource::<Trades>().0[index];
        match broken(world, trade, settings.reach) {
            Some(why) => end(world, index, why),
            None => index += 1,
        }
    }
    show(world);
}

/// The words a game's settings must have for trading, checked as the game loads.
pub(crate) fn words(words: &GameWords) -> [(&'static str, &String); 16] {
    [
        ("trade_asks", &words.trade_asks),
        ("trade_asked", &words.trade_asked),
        ("trade_sent", &words.trade_sent),
        ("trade_denied", &words.trade_denied),
        ("trade_busy", &words.trade_busy),
        ("trade_too_far", &words.trade_too_far),
        ("trade_cancelled_by", &words.trade_cancelled_by),
        ("trade_ended", &words.trade_ended),
        ("trade_why_apart", &words.trade_why_apart),
        ("trade_why_fell", &words.trade_why_fell),
        ("trade_why_left", &words.trade_why_left),
        ("trade_why_mounted", &words.trade_why_mounted),
        ("trade_why_offer_gone", &words.trade_why_offer_gone),
        ("trade_offer_changed", &words.trade_offer_changed),
        ("trade_overflow_title", &words.trade_overflow_title),
        ("trade_overflow_text", &words.trade_overflow_text),
    ]
}

/// A trade between two players, held by the world both stand in.
struct Trade {
    sides: [TradeSide; 2],
    version: TradeVersion,
    opens: UnixMillis,
}

struct TradeSide {
    trader: Entity,
    name: String,
    /// What they give, at most one stack of each item.
    offer: Vec<ItemStack>,
    /// Whether they accept the deal as it stands.
    accepts: bool,
}

/// Why a trade ended without going through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TradeEnd {
    Cancelled(Entity),
    Apart,
    Fell(Entity),
    Left(Entity),
    Mounted(Entity),
    OfferGone(Entity),
}

impl Trade {
    fn side_of(&self, character: Entity) -> Option<usize> {
        self.sides.iter().position(|side| side.trader == character)
    }
}

fn ask(world: &mut World, asker: Entity, with: Entity) -> Result<(), LineText> {
    if world.resource::<GameSettings>().trade.is_none()
        || asker == with
        || world.get::<PlayedBy>(with).is_none()
        || !player::may_command(world, asker)
        || !stat::stands(world, with)
        || !visibility::in_sight(world, asker, with)
        || trading(world, asker)
    {
        return Ok(());
    }
    mount::seat::act(world, asker, RiderAct::Interact)?;
    if trading(world, with) {
        return Err(busy(world, with));
    }
    reach::intend(world, asker, ReachTarget::Actor(with), ReachAct::Trade);
    Ok(())
}

/// Has society send the request, once `asker` stands within reach of `with`.
fn send(world: &mut World, asker: Entity, with: Entity) {
    if trading(world, with) {
        let busy = busy(world, with);
        notification::error(world, asker, busy);
        return;
    }
    let client = world.get::<PlayedBy>(asker).map(|by| by.client);
    let of = world.get::<SocietyView>(with).map(|view| view.account);
    if let (Some(client), Some(of)) = (client, of) {
        let ask = SocialRequestAsk::Ask {
            kind: SocialRequestKind::Trade,
            of,
            about: None,
        };
        society::ask(world, client, SocietyAsk::Request(ask));
    }
}

fn busy(world: &World, character: Entity) -> LineText {
    let words = &world.resource::<GameSettings>().words;
    LineText::plain(
        words
            .trade_busy
            .replace("{name}", &name_of(world, character)),
    )
}

/// A request to trade stands while both players stand within reach of each other.
fn stands(settling: &mut Settling, request: &SocialRequest) -> bool {
    traders(settling, &request.asker, &request.asked).is_some_and(|(world, asker, asked)| {
        world
            .resource::<GameSettings>()
            .trade
            .is_some_and(|settings| within(world, asker, asked, settings.reach))
    })
}

fn accepted(
    settling: &mut Settling,
    settings: &GameSettings,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let too_far = RequestUnaccepted::Over(settings.words.trade_too_far.clone());
    let Some(trade) = settings.trade else {
        return Err(too_far);
    };
    let Some((world, asker, asked)) = traders(settling, &request.asker, &request.asked) else {
        return Err(too_far);
    };
    if !within(world, asker, asked, trade.reach) {
        return Err(too_far);
    }
    if let Some(busy) = [asker, asked]
        .into_iter()
        .find(|&trader| trading(world, trader))
    {
        return Err(RequestUnaccepted::Over(busy_words(world, busy)));
    }
    let side = |trader| TradeSide {
        trader,
        name: name_of(world, trader),
        offer: Vec::new(),
        accepts: false,
    };
    let opened = Trade {
        sides: [side(asker), side(asked)],
        version: TradeVersion(0),
        opens: world.resource::<WallClock>().now,
    };
    world.resource_mut::<Trades>().0.push(opened);
    show(world);
    Ok(())
}

fn busy_words(world: &World, character: Entity) -> String {
    busy(world, character).plain_words(world.resource::<Content>())
}

fn offer(world: &mut World, player: Entity, stack: ItemStack) -> Result<(), LineText> {
    let Some((index, side)) = place_of(world, player) else {
        return Ok(());
    };
    let content = world.resource::<Content>().clone();
    if stack.count == 0 {
        return Ok(());
    }
    if stack.item.get(&content).bound() {
        return Err(ItemStack::bound_line(stack.item, &content));
    }
    let slots = world
        .resource::<GameSettings>()
        .trade
        .map_or(0, |settings| settings.slots);
    let held = world
        .get::<Inventory>(player)
        .map_or(0, |bag| bag.count(stack.item));
    let trade = &world.resource::<Trades>().0[index];
    let offer = &trade.sides[side].offer;
    let offered = offer
        .iter()
        .find(|offered| offered.item == stack.item)
        .map(|offered| offered.count);
    let more = offered.unwrap_or(0).saturating_add(stack.count);
    if more > held || (offered.is_none() && offer.len() as u32 >= slots) {
        return Ok(());
    }
    change(world, index, side, |offer| {
        match offer.iter_mut().find(|offered| offered.item == stack.item) {
            Some(offered) => offered.count = more,
            None => offer.push(stack),
        }
    });
    Ok(())
}

fn withdraw(world: &mut World, player: Entity, stack: ItemStack) {
    let Some((index, side)) = place_of(world, player) else {
        return;
    };
    let offers = world.resource::<Trades>().0[index].sides[side]
        .offer
        .iter()
        .any(|offered| offered.item == stack.item);
    if stack.count == 0 || !offers {
        return;
    }
    change(world, index, side, |offer| {
        for offered in offer.iter_mut() {
            if offered.item == stack.item {
                offered.count = offered.count.saturating_sub(stack.count);
            }
        }
        offer.retain(|offered| offered.count > 0);
    });
}

/// Changes what side `side` of trade `index` gives: a new version of the deal, which nobody has
/// accepted yet, and which can't be accepted until its wait is over. The other player is told it
/// changed.
fn change(world: &mut World, index: usize, side: usize, edit: impl FnOnce(&mut Vec<ItemStack>)) {
    let now = world.resource::<WallClock>().now;
    let waits = world
        .resource::<GameSettings>()
        .trade
        .map_or(Seconds(0.0), |settings| settings.accept_waits);
    let mut trades = world.resource_mut::<Trades>();
    let trade = &mut trades.0[index];
    let before = trade.sides[side].offer.clone();
    edit(&mut trade.sides[side].offer);
    if trade.sides[side].offer == before {
        return;
    }
    for side in &mut trade.sides {
        side.accepts = false;
    }
    trade.version = TradeVersion(trade.version.0.wrapping_add(1));
    trade.opens = now.after(waits);
    let name = trade.sides[side].name.clone();
    let other = trade.sides[1 - side].trader;
    let words = &world.resource::<GameSettings>().words;
    let said = LineText::plain(words.trade_offer_changed.replace("{name}", &name));
    let changed = Notification::new(
        NotificationKind::Narration {
            label: "Trade".into(),
            sfx: None,
        },
        said,
    );
    notification::notify(world, other, None, changed);
}

fn accept(world: &mut World, player: Entity, version: TradeVersion) {
    let Some((index, side)) = place_of(world, player) else {
        return;
    };
    let now = world.resource::<WallClock>().now;
    let mut trades = world.resource_mut::<Trades>();
    let trade = &mut trades.0[index];
    let offered = trade.sides.iter().any(|side| !side.offer.is_empty());
    if trade.version != version || now < trade.opens || !offered {
        return;
    }
    trade.sides[side].accepts = true;
    let both = trade.sides.iter().all(|side| side.accepts);
    if both {
        swap(world, index);
    }
}

fn unaccept(world: &mut World, player: Entity) {
    let Some((index, side)) = place_of(world, player) else {
        return;
    };
    world.resource_mut::<Trades>().0[index].sides[side].accepts = false;
}

/// Takes both offers out of both bags and pays each side what the other gave, in one step.
fn swap(world: &mut World, index: usize) {
    let trade = &world.resource::<Trades>().0[index];
    if let Some(gone) = trade
        .sides
        .iter()
        .find(|side| !holds(world, side.trader, &side.offer))
    {
        let gone = gone.trader;
        end(world, index, TradeEnd::OfferGone(gone));
        return;
    }
    let trade = world.resource_mut::<Trades>().0.remove(index);
    let content = world.resource::<Content>().clone();
    for side in &trade.sides {
        if let Some(mut bag) = world.get_mut::<Inventory>(side.trader) {
            let _ = bag.exchange(&content, &side.offer, &[]);
        }
    }
    for (side, other) in [(0, 1), (1, 0)] {
        let (side, other) = (&trade.sides[side], &trade.sides[other]);
        world.entity_mut(side.trader).remove::<TradeView>();
        let reason = PayoutReason::Traded {
            with: other.name.clone(),
        };
        let paid = payout::pay(world, side.trader, &other.offer, reason);
        if let Some(overflow) = paid.overflow(&content) {
            notification::notify(
                world,
                side.trader,
                None,
                Notification::feed(HistoryTopic::Item, overflow),
            );
        }
    }
}

/// Ends trade `index` without it going through, telling each player still here why.
fn end(world: &mut World, index: usize, why: TradeEnd) {
    let trade = world.resource_mut::<Trades>().0.remove(index);
    let words = world.resource::<GameSettings>().words.clone();
    for (side, other) in [(0, 1), (1, 0)] {
        let (side, other) = (&trade.sides[side], &trade.sides[other]);
        let Ok(mut trader) = world.get_entity_mut(side.trader) else {
            continue;
        };
        trader.remove::<TradeView>();
        let named = |who: Entity| {
            let side = trade
                .sides
                .iter()
                .find(|side| side.trader == who)
                .unwrap_or(side);
            side.name.clone()
        };
        let reason = match why {
            TradeEnd::Cancelled(by) if by == side.trader => continue,
            TradeEnd::Cancelled(_) => {
                let said = words.trade_cancelled_by.replace("{name}", &other.name);
                notification::error(world, side.trader, said);
                continue;
            }
            TradeEnd::Apart => words.trade_why_apart.clone(),
            TradeEnd::Fell(who) => words.trade_why_fell.replace("{who}", &named(who)),
            TradeEnd::Left(who) => words.trade_why_left.replace("{who}", &named(who)),
            TradeEnd::Mounted(who) => words.trade_why_mounted.replace("{who}", &named(who)),
            TradeEnd::OfferGone(who) => words.trade_why_offer_gone.replace("{who}", &named(who)),
        };
        let said = words
            .trade_ended
            .replace("{name}", &other.name)
            .replace("{why}", &reason);
        notification::error(world, side.trader, said);
    }
}

/// Why `trade` can't go on, if it can't.
fn broken(world: &World, trade: &Trade, reach: Meters) -> Option<TradeEnd> {
    for side in &trade.sides {
        let trader = side.trader;
        let here = world.get::<PlayedBy>(trader).is_some()
            && player::attended(world, trader)
            && world.get::<Crossing>(trader).is_none();
        if !here {
            return Some(TradeEnd::Left(trader));
        }
        if stat::is_dead(world, trader) {
            return Some(TradeEnd::Fell(trader));
        }
        if !mount::seat::lets(world, trader, RiderAct::Interact) {
            return Some(TradeEnd::Mounted(trader));
        }
        if !holds(world, trader, &side.offer) {
            return Some(TradeEnd::OfferGone(trader));
        }
    }
    let [one, other] = &trade.sides;
    (!within(world, one.trader, other.trader, reach)).then_some(TradeEnd::Apart)
}

/// Shows every trader their trade as it now stands.
fn show(world: &mut World) {
    let views: Vec<(Entity, TradeView)> = world
        .resource::<Trades>()
        .0
        .iter()
        .flat_map(|trade| [(0, 1), (1, 0)].map(|(side, other)| view(trade, side, other)))
        .collect();
    for (trader, view) in views {
        let Ok(mut trader) = world.get_entity_mut(trader) else {
            continue;
        };
        if trader.get::<TradeView>() != Some(&view) {
            trader.insert(view);
        }
    }
}

fn view(trade: &Trade, side: usize, other: usize) -> (Entity, TradeView) {
    let (mine, theirs) = (&trade.sides[side], &trade.sides[other]);
    let shown = TradeView {
        with: theirs.name.clone(),
        version: trade.version,
        mine: mine.offer.clone(),
        theirs: theirs.offer.clone(),
        i_accept: mine.accepts,
        they_accept: theirs.accepts,
        opens: trade.opens,
    };
    (mine.trader, shown)
}

/// Whether `trader`'s bag still holds all of `offer`.
fn holds(world: &World, trader: Entity, offer: &[ItemStack]) -> bool {
    let Some(bag) = world.get::<Inventory>(trader) else {
        return offer.is_empty();
    };
    offer
        .iter()
        .all(|stack| bag.count(stack.item) >= stack.count)
}

fn within(world: &World, one: Entity, other: Entity, reach: Meters) -> bool {
    match (
        movement::position(world, one),
        movement::position(world, other),
    ) {
        (Some(one), Some(other)) => one.distance(other) <= reach,
        _ => false,
    }
}

fn index_of(world: &World, player: Entity) -> Option<usize> {
    place_of(world, player).map(|(index, _)| index)
}

/// The trade `player` is in, and their side of it.
fn place_of(world: &World, player: Entity) -> Option<(usize, usize)> {
    world
        .resource::<Trades>()
        .0
        .iter()
        .enumerate()
        .find_map(|(index, trade)| Some((index, trade.side_of(player)?)))
}

fn name_of(world: &World, character: Entity) -> String {
    world
        .get::<Name>(character)
        .map(|name| name.name.clone())
        .unwrap_or_default()
}

/// The world both accounts' characters stand in, and the two characters.
fn traders<'s>(
    settling: &'s mut Settling,
    one: &AccountId,
    other: &AccountId,
) -> Option<(&'s mut World, Entity, Entity)> {
    let one = settling.roster.account(one)?.client;
    let (world, other) = settling.reach(other)?;
    let one = *world.resource::<Players>().0.get(&one)?;
    Some((world, one, other))
}
