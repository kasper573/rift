//! Two players trading: one asks, the other accepts, and each sets out what they give in a window
//! both see from their own side. The trade goes through once both accept the same deal, what
//! each gives swapped for what the other gives in one step; until then nothing leaves either bag, so a trade that ends any
//! other way has nothing to undo.

pub mod window;

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::{Meters, Seconds};
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::item::deliver::{self, DeliveryReason};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::messages::{self, messages};
use crate::systems::mount::RiderAct;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::{self, PlayedBy, Players, RequestNeeds};
use crate::systems::reach::{self, Lapse, Pursuit, ReachAct, ReachTarget, Tether};
use crate::systems::social_request::{
    RequestUnaccepted, RequestUnsent, SocialRequest, SocialRequestAsk, SocialRequestKind,
    SocialRequestSource, SocialRequestWords,
};
use crate::systems::society::{self, Settling, SocietyAsk, SocietyView};
use crate::systems::{GameSettings, stat, visibility};

messages! {
    /// What the game says of trades between players.
    pub struct TradeMessages {
        /// What a player asked to trade reads on the card, `{name}` asking.
        asks(name: Text),
        /// What a player reads while their request to trade with `{name}` waits.
        asked(name: Text),
        /// What a player is told as their request to trade is sent to `{name}`.
        sent(name: Text),
        /// What a player is told as `{name}` denies their request to trade.
        denied(name: Text),
        /// What a player is told asking `{name}` to trade while they trade with someone else.
        busy(name: Text),
        /// What both are told as a trade is accepted while they stand too far apart.
        too_far,
        /// What a player is told as `{name}` cancels their trade.
        cancelled_by(name: Text),
        /// What a player is told as their trade with `{name}` ends. `{why}` selects the reason
        /// (`apart`: they moved out of reach of each other; `fell`, `left`: `{who}` fell or left;
        /// `mounted`: `{who}` got on a mount they can't trade from; `item_gone`: something `{who}`
        /// gives left their bag).
        ended(name: Text, who: Text, why: Text),
        /// What a player is told as `{name}`, whom they trade with, changes what they give.
        gives_changed(name: Text),
        /// The title of the message the game sends a player whose bag had no room for what their
        /// trade with `{name}` gave them.
        overflow_title(name: Text),
        /// The text of that message.
        overflow_text,
        /// The label a notification of a change to what a player gives carries.
        label,
        /// The title of the trade window, trading with `{name}`.
        window_title(name: Text),
        /// The heading over what the player gives in a trade.
        you_give,
        /// How a screen reader names what the other player gives in a trade.
        they_give,
        /// The heading over what `{name}` gives in a trade.
        gives(name: Text),
        /// What the trade window says while neither player gives anything.
        nothing_given,
        /// What the trade window says while what a player gives just changed, `{time}` before
        /// Accept opens.
        opens_in(time: Text),
        /// What the trade window says while neither player has accepted.
        none_accepted,
        /// What the trade window says once `{name}` has accepted.
        they_accepted(name: Text),
        /// What the trade window says once the player has accepted, awaiting `{name}`.
        you_accepted(name: Text),
        /// What the trade window says once both players have accepted.
        both_accepted,
        /// The button that takes back the player's acceptance of a trade.
        undo_accept,
        /// The button that accepts a trade.
        accept,
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<TradeView>()
        .add_mapped_client_message::<TradeAsk>()
        .add_mapped_client_message::<TradeRequest>();
}

/// How a game has players trade. A game without it has no trading.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeSettings {
    /// How close two players stand to ask and to keep trading.
    pub reach: Meters,
    /// How many different items each side may give.
    pub slots: u32,
    /// How long Accept stays shut after either side changes what it gives, so an accept already on its way
    /// never takes a deal its player has not seen.
    pub accept_waits: Seconds,
}

/// Which change of what the two sides give a trade is at. Every change makes a new one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeVersion(pub u32);

/// A player's ask to walk into reach of the player `with` and ask them to trade.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct TradeAsk {
    #[entities]
    pub with: Entity,
}

/// What a player asks of a trade they are in.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum TradeRequest {
    /// Adds this many of an item from the bag to what the player gives.
    Give(ItemStack),
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
    /// When Accept opens, after the latest change of what either side gives.
    pub opens: UnixMillis,
}

impl TradeView {
    /// Whether the deal can be accepted at `now`: something is given, and the latest change
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
        words: SocialRequestWords {
            asks: |messages, name| messages.trade.asks(name),
            asked: |messages, name| messages.trade.asked(name),
            sent: |messages, name| messages.trade.sent(name),
            accepted: None,
            denied: Some(|messages, name| messages.trade.denied(name)),
            lapsed: None,
        },
        may_send: |_, content, _, _| match content.built::<GameSettings>().trade {
            Some(_) => Ok(()),
            None => Err(RequestUnsent::Quietly),
        },
        accepted,
        stands,
    }
}

/// Hands what players of this world asked of trading to the trade, or to society.
pub fn requests(world: &mut World) {
    player::player_requests::<TradeAsk>(
        world,
        RequestNeeds::Seat(RiderAct::Interact),
        |world, player, TradeAsk { with }| ask(world, player, with),
    );
    player::player_requests::<TradeRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            match request {
                TradeRequest::Give(stack) => give(world, player, stack)?,
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
/// give.
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
    gives: Vec<ItemStack>,
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
    ItemGone(Entity),
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
        || !stat::stands(world, with)
        || !visibility::in_sight(world, asker, with)
        || trading(world, asker)
    {
        return Ok(());
    }
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
    let messages = messages::of(world.resource::<Content>());
    LineText::plain(messages.trade.busy(name_of(world, character)))
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
    content: &Content,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let too_far = RequestUnaccepted::Over(messages::of(content).trade.too_far().into());
    let Some(trade) = content.built::<GameSettings>().trade else {
        return Err(too_far);
    };
    let Some((world, asker, asked)) = traders(settling, &request.asker, &request.asked) else {
        return Err(too_far);
    };
    if !within(world, asker, asked, trade.reach) {
        return Err(too_far);
    }
    if let Some(occupied) = [asker, asked]
        .into_iter()
        .find(|&trader| trading(world, trader))
    {
        return Err(RequestUnaccepted::Over(busy(world, occupied)));
    }
    let side = |trader| TradeSide {
        trader,
        name: name_of(world, trader),
        gives: Vec::new(),
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

fn give(world: &mut World, player: Entity, stack: ItemStack) -> Result<(), LineText> {
    let Some((index, side)) = place_of(world, player) else {
        return Ok(());
    };
    let content = world.resource::<Content>().clone();
    if stack.count == 0 {
        return Ok(());
    }
    ItemStack::refuse_bound(&[stack], &content)?;
    let slots = world
        .resource::<GameSettings>()
        .trade
        .map_or(0, |settings| settings.slots);
    let held = world
        .get::<Inventory>(player)
        .map_or(0, |bag| bag.count(stack.item));
    let trade = &world.resource::<Trades>().0[index];
    let gives = &trade.sides[side].gives;
    let given = gives
        .iter()
        .find(|given| given.item == stack.item)
        .map(|given| given.count);
    let more = given.unwrap_or(0).saturating_add(stack.count);
    if more > held || (given.is_none() && gives.len() as u32 >= slots) {
        return Ok(());
    }
    change(world, index, side, |gives| {
        match gives.iter_mut().find(|given| given.item == stack.item) {
            Some(given) => given.count = more,
            None => gives.push(stack),
        }
    });
    Ok(())
}

fn withdraw(world: &mut World, player: Entity, stack: ItemStack) {
    let Some((index, side)) = place_of(world, player) else {
        return;
    };
    let gives_it = world.resource::<Trades>().0[index].sides[side]
        .gives
        .iter()
        .any(|given| given.item == stack.item);
    if stack.count == 0 || !gives_it {
        return;
    }
    change(world, index, side, |gives| {
        for given in gives.iter_mut() {
            if given.item == stack.item {
                given.count = given.count.saturating_sub(stack.count);
            }
        }
        gives.retain(|given| given.count > 0);
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
    let before = trade.sides[side].gives.clone();
    edit(&mut trade.sides[side].gives);
    if trade.sides[side].gives == before {
        return;
    }
    for side in &mut trade.sides {
        side.accepts = false;
    }
    trade.version = TradeVersion(trade.version.0.wrapping_add(1));
    trade.opens = now.after(waits);
    let name = trade.sides[side].name.clone();
    let other = trade.sides[1 - side].trader;
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).trade;
    let said = LineText::plain(words.gives_changed(name));
    let changed = Notification::new(
        NotificationKind::Narration {
            label: words.label().to_owned().into(),
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
    let given = trade.sides.iter().any(|side| !side.gives.is_empty());
    if trade.version != version || now < trade.opens || !given {
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

/// Takes what both sides give out of both bags and pays each side what the other gave, in one
/// step.
fn swap(world: &mut World, index: usize) {
    let trade = &world.resource::<Trades>().0[index];
    if let Some(gone) = trade
        .sides
        .iter()
        .find(|side| !holds(world, side.trader, &side.gives))
    {
        let gone = gone.trader;
        end(world, index, TradeEnd::ItemGone(gone));
        return;
    }
    let trade = world.resource_mut::<Trades>().0.remove(index);
    let content = world.resource::<Content>().clone();
    for side in &trade.sides {
        if let Some(mut bag) = world.get_mut::<Inventory>(side.trader) {
            let _ = bag.exchange(&content, &side.gives, &[]);
        }
    }
    for (side, other) in [(0, 1), (1, 0)] {
        let (side, other) = (&trade.sides[side], &trade.sides[other]);
        world.entity_mut(side.trader).remove::<TradeView>();
        let reason = DeliveryReason::Traded {
            with: other.name.clone(),
        };
        deliver::to_character(world, side.trader, &other.gives, reason);
    }
}

/// Ends trade `index` without it going through, telling each player still here why.
fn end(world: &mut World, index: usize, why: TradeEnd) {
    let trade = world.resource_mut::<Trades>().0.remove(index);
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).trade;
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
        let (reason, who) = match why {
            TradeEnd::Cancelled(by) if by == side.trader => continue,
            TradeEnd::Cancelled(_) => {
                let said = words.cancelled_by(&other.name);
                notification::error(world, side.trader, said);
                continue;
            }
            TradeEnd::Apart => ("apart", String::new()),
            TradeEnd::Fell(who) => ("fell", named(who)),
            TradeEnd::Left(who) => ("left", named(who)),
            TradeEnd::Mounted(who) => ("mounted", named(who)),
            TradeEnd::ItemGone(who) => ("item_gone", named(who)),
        };
        let said = words.ended(&other.name, who, reason);
        notification::error(world, side.trader, said);
    }
}

/// Why `trade` can't go on, if it can't.
fn broken(world: &World, trade: &Trade, reach: Meters) -> Option<TradeEnd> {
    let mut apart = false;
    for (side, other) in [(0, 1), (1, 0)] {
        let side = &trade.sides[side];
        let trader = side.trader;
        let here = world.get::<PlayedBy>(trader).is_some() && player::attended(world, trader);
        if !here {
            return Some(TradeEnd::Left(trader));
        }
        let tether = Tether::beside(world, trade.sides[other].trader, reach);
        match reach::lapse(world, trader, tether.as_ref()) {
            Some(Lapse::Fell) => return Some(TradeEnd::Fell(trader)),
            Some(Lapse::Crossing) => return Some(TradeEnd::Left(trader)),
            Some(Lapse::Mounted) => return Some(TradeEnd::Mounted(trader)),
            Some(Lapse::OutOfReach) => apart = true,
            None => {}
        }
        if !holds(world, trader, &side.gives) {
            return Some(TradeEnd::ItemGone(trader));
        }
    }
    apart.then_some(TradeEnd::Apart)
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
        mine: mine.gives.clone(),
        theirs: theirs.gives.clone(),
        i_accept: mine.accepts,
        they_accept: theirs.accepts,
        opens: trade.opens,
    };
    (mine.trader, shown)
}

/// Whether `trader`'s bag still holds all of `gives`.
fn holds(world: &World, trader: Entity, gives: &[ItemStack]) -> bool {
    let Some(bag) = world.get::<Inventory>(trader) else {
        return gives.is_empty();
    };
    gives
        .iter()
        .all(|stack| bag.count(stack.item) >= stack.count)
}

/// Whether neither of the two players has lapsed from keeping a trade open with the other.
fn within(world: &World, one: Entity, other: Entity, range: Meters) -> bool {
    [(one, other), (other, one)]
        .into_iter()
        .all(|(player, to)| {
            Tether::beside(world, to, range)
                .is_some_and(|tether| reach::lapse(world, player, Some(&tether)).is_none())
        })
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
