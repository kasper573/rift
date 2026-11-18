//! The trade window: the player's bag, what they give and what the other player gives, side by
//! side, with both players' accepts under them.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::controls;
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::{ink, sheet};

use super::{TradeRequest, TradeView};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::GameSettings;
use crate::systems::hud::{self, Centred, WindowKey};
use crate::systems::item::grid::{scrolled, slots_span};
use crate::systems::item::transfer::{
    self, ItemTransfer, TransferPane, TransferWindow, transfer_window,
};
use crate::systems::item::tray;
use crate::systems::item::widget::{InventoryPane, inventory_pane};
use crate::systems::item::{self, ItemId, ItemStack};
use crate::systems::messages::{self, Messages};
use crate::systems::player::session;
use crate::systems::settings::Placed;
use crate::systems::trade::TradeMessages;

pub(crate) fn plugin(app: &mut App) {
    transfer_window::<TradeGives>(app);
}

/// The trade window: a sheet that is up while the player trades. Closing it cancels the trade.
pub(crate) fn draw(world: &mut World, gui: &egui::Context) {
    let Some(now) = time::server_now(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let window = Centred {
        key: WindowKey::Trade,
        of: Placed::Trade,
        body: Vec2::new(width(), BODY_HEIGHT),
        top: 120.0,
    };
    let title = |trading: &TradeView| messages::of(&content).trade.window_title(&trading.with);
    let drawn = hud::centred(
        world,
        gui,
        window,
        view(world).cloned(),
        title,
        |ui, world, trading| body(ui, world, trading, now),
    );
    let Some(drawn) = drawn else {
        return;
    };
    let asked = if drawn.closed {
        Some(TradeRequest::Cancel)
    } else {
        drawn.inner
    };
    if let Some(asked) = asked {
        world.write_message(asked);
    }
}

/// Cancels the trade under way, if there is one: what stepping out does while the player trades.
pub fn is_open(world: &World) -> bool {
    view(world).is_some()
}

pub fn cancel(world: &mut World) -> bool {
    if !is_open(world) {
        return false;
    }
    world.write_message(TradeRequest::Cancel);
    true
}

/// The trade the player is in, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownTrade {
    pub with: String,
    pub mine: Vec<ShownTradeStack>,
    pub theirs: Vec<ShownTradeStack>,
    pub i_accept: bool,
    pub they_accept: bool,
    /// Whether Accept is open now.
    pub accept_open: bool,
    /// The sentence above Accept that says where the trade stands.
    pub status: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownTradeStack {
    pub item: ItemId,
    pub count: u32,
}

pub fn shown(world: &World) -> Option<ShownTrade> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    let messages = messages::of(world.resource::<Content>());
    let stacks = |stacks: &[ItemStack]| {
        stacks
            .iter()
            .map(|stack| ShownTradeStack {
                item: stack.item,
                count: stack.count,
            })
            .collect()
    };
    Some(ShownTrade {
        with: view.with.clone(),
        mine: stacks(&view.mine),
        theirs: stacks(&view.theirs),
        i_accept: view.i_accept,
        they_accept: view.they_accept,
        accept_open: !view.i_accept && view.open_at(now),
        status: status(messages, view, now),
    })
}

/// What the trade window holds: the bag and what both sides give, side by side, and under them
/// where the trade stands and the button that accepts it.
fn body(
    ui: &mut Ui,
    world: &mut World,
    shown: &TradeView,
    now: UnixMillis,
) -> Option<TradeRequest> {
    let mut asked = None;
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let room = ui.available_rect_before_wrap().shrink2(sheet::PANE_PAD);
    let bar = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - BAR_HEIGHT), room.max);
    let panes = Rect::from_min_max(room.min, Pos2::new(room.max.x, bar.min.y));
    let ([bag_at, mine_at], theirs_at) = sheet::spaced_columns(panes, [pane(), pane()]);
    ui.scope_builder(column(bag_at), |ui| {
        sheet::section(ui, words.interface.inventory());
        scrolled(ui, "trade bag", |ui| {
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: None,
            };
            inventory_pane(ui, world, pane);
        });
    });
    ui.scope_builder(column(mine_at), |ui| {
        sheet::section(ui, words.trade.you_give());
        let mine = placed(&shown.mine);
        scrolled(ui, "trade mine", |ui| {
            let pane = TransferPane {
                placed: &mine,
                slots: slots(world),
                named: words.trade.you_give(),
                transfer: Some(ItemTransfer::of::<TradeGives>()),
            };
            transfer::transfer_pane(ui, world, pane);
        });
    });
    ui.scope_builder(column(theirs_at), |ui| {
        let gives = words.trade.gives(&shown.with);
        sheet::section(ui, &gives);
        let theirs = placed(&shown.theirs);
        scrolled(ui, "trade theirs", |ui| {
            let pane = TransferPane {
                placed: &theirs,
                slots: slots(world),
                named: words.trade.they_give(),
                transfer: None,
            };
            transfer::transfer_pane(ui, world, pane);
        });
    });
    ui.painter()
        .hline(bar.x_range(), bar.min.y, sheet::rule_stroke());
    let centered = UiBuilder::new()
        .max_rect(bar.shrink2(Vec2::new(0.0, BAR_GAP)))
        .layout(Layout::top_down(Align::Center));
    ui.scope_builder(centered, |ui| {
        sheet::label(ui, TextRole::Body, &status(words, shown, now), ink::INK2);
        ui.add_space(BAR_GAP);
        if let Some(request) = accept_button(ui, &words.trade, shown, now) {
            asked = Some(request);
        }
    });
    ui.advance_cursor_after_rect(room);
    asked
}

fn column(rect: Rect) -> UiBuilder {
    UiBuilder::new()
        .max_rect(rect)
        .layout(Layout::top_down(Align::Min))
}

const COLUMNS: u32 = 4;
/// The room under the panes: a line of where the trade stands and Accept, each as tall as a
/// control, with gaps around them.
const BAR_HEIGHT: f32 = 3.0 * BAR_GAP + 2.0 * controls::CONTROL_HEIGHT;
const BAR_GAP: f32 = 8.0;
/// The most room a section heading takes: the space above it and a control's height.
const HEADING: f32 = 6.0 + controls::CONTROL_HEIGHT;
/// How many rows of the bag show before it scrolls.
const BAG_ROWS: u32 = 3;
const BODY_HEIGHT: f32 = 2.0 * sheet::PANE_PAD.y + HEADING + slots_span(BAG_ROWS) + BAR_HEIGHT;

fn pane() -> f32 {
    slots_span(COLUMNS)
}

fn width() -> f32 {
    3.0 * pane() + 2.0 * sheet::GUTTER
}

fn view(world: &World) -> Option<&TradeView> {
    session::my_character(world)?.get::<TradeView>()
}

fn trading(world: &World) -> bool {
    view(world).is_some()
}

fn slots(world: &World) -> u32 {
    world
        .resource::<GameSettings>()
        .trade
        .map_or(0, |settings| settings.slots)
}

/// Where the trade stands, in one sentence.
fn status(messages: &Messages, shown: &TradeView, now: UnixMillis) -> String {
    let words = &messages.trade;
    let with = &shown.with;
    if shown.mine.is_empty() && shown.theirs.is_empty() {
        return words.nothing_given().to_owned();
    }
    if now < shown.opens {
        return words.opens_in(messages.time.rough(shown.opens.since(now)));
    }
    match (shown.i_accept, shown.they_accept) {
        (false, false) => words.none_accepted().to_owned(),
        (false, true) => words.they_accepted(with),
        (true, false) => words.you_accepted(with),
        (true, true) => words.both_accepted().to_owned(),
    }
}

/// Accept, shut while nothing is given or the latest change waits out its time, and Undo
/// accept once the player accepted. What it asks, once pressed.
fn accept_button(
    ui: &mut Ui,
    words: &TradeMessages,
    shown: &TradeView,
    now: UnixMillis,
) -> Option<TradeRequest> {
    if shown.i_accept {
        return controls::button(ui, words.undo_accept())
            .clicked()
            .then_some(TradeRequest::Unaccept);
    }
    if shown.open_at(now) {
        return controls::primary(ui, words.accept())
            .clicked()
            .then_some(TradeRequest::Accept(shown.version));
    }
    controls::faded(ui, words.accept());
    None
}

/// The player's side of the trade as a window items move into.
struct TradeGives;

impl TransferWindow for TradeGives {
    fn open(world: &World) -> bool {
        trading(world)
    }

    /// Gives `count` of the bag's slot `slot`.
    fn put(world: &mut World, slot: u32, count: u32) -> bool {
        let Some(held) = item::my_bag(world).and_then(|bag| bag.slots.get(slot as usize).copied())
        else {
            return false;
        };
        if held.bound(world.resource::<Content>()) {
            return false;
        }
        world.write_message(TradeRequest::Give(ItemStack::new(held.item, count)));
        true
    }

    /// Takes `count` of the stack at `index` of what the player gives back.
    fn take(world: &mut World, index: usize, count: u32) -> bool {
        let Some(given) = view(world).and_then(|view| view.mine.get(index).copied()) else {
            return false;
        };
        world.write_message(TradeRequest::Withdraw(ItemStack::new(given.item, count)));
        true
    }

    fn aside(world: &World, slot: u32, _: ItemStack) -> u32 {
        let (Some(view), Some(bag)) = (view(world), item::my_bag(world)) else {
            return 0;
        };
        bag.taken_from(slot, &view.mine)
    }
}

fn placed(stacks: &[ItemStack]) -> Vec<tray::Placed> {
    stacks
        .iter()
        .map(|&stack| tray::Placed {
            stack,
            short: false,
        })
        .collect()
}
