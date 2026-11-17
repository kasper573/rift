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
use crate::systems::hud::{self, Sizing, WindowFrame};
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, item_transfer};
use crate::systems::item::tray;
use crate::systems::item::widget::{
    InventoryPane, SlotVerdict, inventory_pane, pane_width, slot_verdict_source,
};
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::settings::Placed;
use crate::systems::text::LineText;

pub const WINDOW_KEY: &str = "trade_window";

pub(crate) fn plugin(app: &mut App) {
    slot_verdict_source(app, verdict);
    item_transfer(app, TRANSFER);
}

/// The trade window: a sheet that is up while the player trades. Closing it cancels the trade.
pub(crate) fn draw(world: &mut World, gui: &egui::Context) {
    let trading = view(world).cloned();
    let Some(pose) = hud::sheet_pose(world, gui, WINDOW_KEY, trading.is_some()) else {
        return;
    };
    let Some(shown) = hud::lifting(world, trading.clone()) else {
        return;
    };
    let Some(now) = time::server_now(world) else {
        return;
    };
    let title = format!("Trading with {}", shown.with);
    let screen = gui.content_rect();
    let size = sheet::titled_size(Vec2::new(width() + 2.0 * sheet::PANE_PAD.x, BODY_HEIGHT));
    let frame = WindowFrame {
        key: WINDOW_KEY,
        of: Placed::Trade,
        title: Some(&title),
        start: Rect::from_min_size(
            Pos2::new(screen.center().x - size.x / 2.0, screen.top() + 120.0),
            size,
        ),
        sizing: Sizing::Fixed,
    };
    let mut stays = trading.is_some();
    let mut asked = None;
    hud::window(world, gui, frame, pose, &mut stays, |ui, world| {
        if trading.is_none() {
            ui.disable();
        }
        asked = body(ui, world, &shown, now);
    });
    if trading.is_some() && !stays {
        asked = Some(TradeRequest::Cancel);
    }
    if let Some(asked) = asked {
        world.write_message(asked);
    }
}

/// Cancels the trade under way, if there is one: what stepping out does while the player trades.
pub fn cancel(world: &mut World) -> bool {
    if view(world).is_none() {
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
        status: status(view, now),
    })
}

/// What the trade window holds: the bag and both offers side by side, and under them where the
/// trade stands and the button that accepts it.
fn body(
    ui: &mut Ui,
    world: &mut World,
    shown: &TradeView,
    now: UnixMillis,
) -> Option<TradeRequest> {
    let mut asked = None;
    let room = ui.available_rect_before_wrap().shrink2(sheet::PANE_PAD);
    let bar = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - BAR_HEIGHT), room.max);
    let panes = Rect::from_min_max(room.min, Pos2::new(room.max.x, bar.min.y));
    let ([bag_at, mine_at], theirs_at) = sheet::spaced_columns(panes, [pane(), pane()]);
    ui.scope_builder(column(bag_at), |ui| {
        sheet::section(ui, "Inventory");
        let room = ui.available_rect_before_wrap();
        sheet::scroll_pane(
            ui,
            sheet::Pane::bare(room),
            egui::ScrollArea::vertical().id_salt("trade bag"),
            |ui| {
                let pane = InventoryPane {
                    width: ui.available_width(),
                    heading: None,
                };
                inventory_pane(ui, world, pane);
            },
        );
    });
    ui.scope_builder(column(mine_at), |ui| {
        sheet::section(ui, "You give");
        let mine = placed(&shown.mine);
        let pane = TransferPane {
            placed: &mine,
            slots: slots(world),
            named: "You give",
            transfer: Some(TRANSFER),
        };
        transfer::transfer_pane(ui, world, pane);
    });
    ui.scope_builder(column(theirs_at), |ui| {
        sheet::section(ui, &format!("{} gives", shown.with));
        let theirs = placed(&shown.theirs);
        let pane = TransferPane {
            placed: &theirs,
            slots: slots(world),
            named: "They give",
            transfer: None,
        };
        transfer::transfer_pane(ui, world, pane);
    });
    ui.painter()
        .hline(bar.x_range(), bar.min.y, sheet::rule_stroke());
    let centered = UiBuilder::new()
        .max_rect(bar.shrink2(Vec2::new(0.0, BAR_GAP)))
        .layout(Layout::top_down(Align::Center));
    ui.scope_builder(centered, |ui| {
        sheet::label(ui, TextRole::Body, &status(shown, now), ink::INK2);
        ui.add_space(BAR_GAP);
        if let Some(request) = accept_button(ui, shown, now) {
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
const BODY_HEIGHT: f32 = 2.0 * sheet::PANE_PAD.y + HEADING + pane_width(BAG_ROWS) + BAR_HEIGHT;

fn pane() -> f32 {
    pane_width(COLUMNS)
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
        .resource::<crate::systems::GameSettings>()
        .trade
        .map_or(0, |settings| settings.slots)
}

/// Where the trade stands, in one sentence.
fn status(shown: &TradeView, now: UnixMillis) -> String {
    let with = &shown.with;
    if shown.mine.is_empty() && shown.theirs.is_empty() {
        return "Neither of you offers anything yet.".to_owned();
    }
    if now < shown.opens {
        let left = shown.opens.since(now).0.ceil().max(1.0);
        return format!("The offer just changed, so Accept opens in {left} s.");
    }
    match (shown.i_accept, shown.they_accept) {
        (false, false) => "Neither of you has accepted the trade yet.".to_owned(),
        (false, true) => format!("{with} has accepted the trade. Awaiting your approval."),
        (true, false) => format!("You have accepted the trade. Awaiting {with}'s approval."),
        (true, true) => "You have both accepted the trade.".to_owned(),
    }
}

/// Accept, shut while nothing is offered or the latest change waits out its time, and Undo
/// accept once the player accepted. What it asks, once pressed.
fn accept_button(ui: &mut Ui, shown: &TradeView, now: UnixMillis) -> Option<TradeRequest> {
    if shown.i_accept {
        return controls::button(ui, "Undo accept")
            .clicked()
            .then_some(TradeRequest::Unaccept);
    }
    if shown.open_at(now) {
        return controls::primary(ui, "Accept")
            .clicked()
            .then_some(TradeRequest::Accept(shown.version));
    }
    controls::faded(ui, "Accept");
    None
}

/// The player's side of the trade as a window items move into.
const TRANSFER: ItemTransfer = ItemTransfer {
    key: "trade",
    open: trading,
    verb: "offer",
    put: offer,
    take: withdraw,
    aside,
};

/// Offers `count` of the bag's slot `slot`.
fn offer(world: &mut World, slot: u32, count: u32) -> bool {
    let Some(held) = session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .and_then(|bag| bag.slots.get(slot as usize).copied())
    else {
        return false;
    };
    if held.item.get(world.resource::<Content>()).bound() {
        return false;
    }
    world.write_message(TradeRequest::Offer(ItemStack::new(held.item, count)));
    true
}

/// Takes `count` of the stack at `index` of the player's offer back.
fn withdraw(world: &mut World, index: usize, count: u32) -> bool {
    let Some(offered) = view(world).and_then(|view| view.mine.get(index).copied()) else {
        return false;
    };
    world.write_message(TradeRequest::Withdraw(ItemStack::new(offered.item, count)));
    true
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

/// How many of the stack in the bag's slot `slot` are offered.
fn aside(world: &World, slot: u32, _: ItemStack) -> u32 {
    let (Some(view), Some(bag)) = (
        view(world),
        session::my_character(world).and_then(|me| me.get::<Inventory>()),
    ) else {
        return 0;
    };
    bag.taken_from(slot, &view.mine)
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    view(world)?;
    stack
        .item
        .get(world.resource::<Content>())
        .bound()
        .then(|| SlotVerdict::Refused(LineText::plain("Bound · can't be traded")))
}
