//! The trade window: the player's bag, what they give and what the other player gives, side by
//! side, with both players' accepts under them.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::controls::{self, Pill};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui};
use ui::face::TextRole;
use ui::{dialog, face, ink, sheet};

use super::{TradeRequest, TradeView};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::actor::Name;
use crate::systems::hud::{self, AmountQuestion, HudWidget};
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::item::widget::{
    InventoryPane, SlotAction, SlotVerdict, StackPane, StackPaneDone, inventory_pane, pane_width,
    slot_action, slot_aside_source, slot_verdict_source, stack_pane,
};
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::text::LineText;

pub const TRADE_WINDOW: &str = "trade_window";

pub(crate) fn plugin(app: &mut App) {
    slot_verdict_source(app, verdict);
    slot_aside_source(app, aside);
    slot_action(
        app,
        SlotAction {
            action: InputAction::Select,
            active: trading,
            act: offer_all,
            verb: "offer it",
        },
    );
    slot_action(
        app,
        SlotAction {
            action: InputAction::SplitStack,
            active: trading,
            act: offer_some,
            verb: "offer some of it",
        },
    );
}

/// The trade window, open while the player trades.
pub struct TradeWindowWidget;

impl HudWidget for TradeWindowWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        Pos2::new(screen.center().x - width() / 2.0, screen.top() + 120.0)
    }

    fn hidden(&self, world: &World) -> bool {
        view(world).is_none()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let shown = view(world)?.clone();
        let now = time::server_now(world)?;
        let content = world.resource::<Content>().clone();
        let mut asked = None;
        dialog::note(ui, width(), |ui| {
            ui.label(face::bold(format!("Trading with {}", shown.with)));
            if let Some(changed) = &shown.changed {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    controls::pill(ui, "Changed", Pill::Gold);
                    let cleared = if changed.cleared {
                        " Both accepts were cleared."
                    } else {
                        ""
                    };
                    let said = format!("{} changed the offer.{cleared}", shown.with);
                    ui.label(face::body(said));
                });
            }
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(pane());
                    sheet::section(ui, "Inventory");
                    egui::ScrollArea::vertical()
                        .id_salt("trade bag")
                        .max_height(pane_width(BAG_ROWS))
                        .show(ui, |ui| {
                            let pane = InventoryPane {
                                width: pane(),
                                heading: None,
                            };
                            inventory_pane(ui, world, pane);
                        });
                });
                ui.add_space(GUTTER);
                ui.vertical(|ui| {
                    ui.set_width(pane());
                    sheet::section(ui, "You give");
                    let pane = StackPane {
                        width: pane(),
                        stacks: &shown.mine,
                        slots: slots(world),
                        named: "You give",
                        pick: Some("take it back"),
                        split: None,
                    };
                    match stack_pane(ui, world, pane) {
                        Some(StackPaneDone::Picked(index)) => {
                            asked = shown.mine.get(index).copied().map(TradeRequest::Withdraw);
                        }
                        Some(StackPaneDone::Dropped(slot)) => asked = offer_of(world, slot),
                        None => {}
                    }
                    listed(ui, &content, &shown.mine, &[], &[]);
                });
                ui.add_space(GUTTER);
                ui.vertical(|ui| {
                    ui.set_width(pane());
                    sheet::section(ui, &format!("{} gives", shown.with));
                    let pane = StackPane {
                        width: pane(),
                        stacks: &shown.theirs,
                        slots: slots(world),
                        named: "They give",
                        pick: None,
                        split: None,
                    };
                    stack_pane(ui, world, pane);
                    let (added, removed) = shown
                        .changed
                        .as_ref()
                        .map(|changed| (changed.added.as_slice(), changed.removed.as_slice()))
                        .unwrap_or_default();
                    listed(ui, &content, &shown.theirs, added, removed);
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(request) = accept_button(ui, &shown, now) {
                        asked = Some(request);
                    }
                    let keys = world.resource::<InputMap>();
                    let esc = keys.name(InputAction::Dismiss);
                    if controls::keyed(ui, "Cancel", esc.as_deref(), true).clicked() {
                        asked = Some(TradeRequest::Cancel);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let me = my_name(world);
                        accepts(ui, &me, shown.i_accept);
                        ui.label(face::body("·").color(ink::INK3));
                        accepts(ui, &shown.with, shown.they_accept);
                    });
                });
            });
        });
        if let Some(asked) = asked {
            world.write_message(asked);
        }
        None
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
    /// What the other player gave before of each item they now give less of.
    pub removed: Vec<ShownTradeStack>,
    pub i_accept: bool,
    pub they_accept: bool,
    /// Whether Accept is open now.
    pub accept_open: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownTradeStack {
    pub item: ItemId,
    pub count: u32,
    /// Whether the other player's latest change gave more of it.
    pub new: bool,
}

pub fn shown(world: &World) -> Option<ShownTrade> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    let (added, removed) = view
        .changed
        .as_ref()
        .map(|changed| (changed.added.clone(), changed.removed.clone()))
        .unwrap_or_default();
    let stacks = |stacks: &[ItemStack], new: &[ItemId]| {
        stacks
            .iter()
            .map(|stack| ShownTradeStack {
                item: stack.item,
                count: stack.count,
                new: new.contains(&stack.item),
            })
            .collect()
    };
    Some(ShownTrade {
        with: view.with.clone(),
        mine: stacks(&view.mine, &[]),
        theirs: stacks(&view.theirs, &added),
        removed: stacks(&removed, &[]),
        i_accept: view.i_accept,
        they_accept: view.they_accept,
        accept_open: !view.i_accept && view.open_at(now),
    })
}

const COLUMNS: u32 = 4;
const BAG_ROWS: u32 = 3;
const GUTTER: f32 = 24.0;

fn pane() -> f32 {
    pane_width(COLUMNS)
}

fn width() -> f32 {
    3.0 * pane() + 2.0 * GUTTER
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

fn my_name(world: &World) -> String {
    session::my_character(world)
        .and_then(|me| me.get::<Name>())
        .map(|name| name.name.clone())
        .unwrap_or_default()
}

/// The stacks a side gives, each with its count written out: those the latest change gave more
/// of marked new, and what it took away struck through.
fn listed(
    ui: &mut Ui,
    content: &Content,
    stacks: &[ItemStack],
    new: &[ItemId],
    gone: &[ItemStack],
) {
    ui.add_space(4.0);
    for stack in stacks {
        ui.horizontal(|ui| {
            ui.label(face::body(stack.describe(content)));
            if new.contains(&stack.item) {
                controls::pill(ui, "New", Pill::Azure);
            }
        });
    }
    for stack in gone {
        ui.label(
            face::body(stack.describe(content))
                .strikethrough()
                .color(ink::INK3),
        );
    }
}

fn accepts(ui: &mut Ui, name: &str, accepted: bool) {
    ui.label(face::body(format!("{name}:")));
    if accepted {
        ui.label(face::bold("Accepted").color(ink::VERDIGRIS));
    } else {
        ui.label(face::bold("Not yet"));
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
    let offered = !shown.mine.is_empty() || !shown.theirs.is_empty();
    let label = if offered && now < shown.opens {
        let left = shown.opens.since(now).0.ceil().max(1.0);
        format!("Accept in {left}")
    } else {
        "Accept".to_owned()
    };
    controls::faded(ui, label);
    if !offered {
        sheet::label(ui, TextRole::Small, "Offer something to trade.", ink::INK3);
    }
    None
}

/// What the player offers of the bag's slot `slot`: all of it that is not offered yet.
fn offer_of(world: &World, slot: u32) -> Option<TradeRequest> {
    let held = session::my_character(world)?
        .get::<Inventory>()?
        .slots
        .get(slot as usize)
        .copied()?;
    let left = held.count.saturating_sub(aside(world, slot, held));
    (left > 0).then(|| TradeRequest::Offer(ItemStack::new(held.item, left)))
}

fn offer_all(world: &mut World, slot: u32) {
    if let Some(offer) = offer_of(world, slot) {
        world.write_message(offer);
    }
}

/// Asks how many of what is left in the bag's slot `slot` to offer, and offers that many.
fn offer_some(world: &mut World, slot: u32) {
    let Some(TradeRequest::Offer(left)) = offer_of(world, slot) else {
        return;
    };
    if left.count < 2 {
        world.write_message(TradeRequest::Offer(left));
        return;
    }
    let named = Mention::Item(left.item).span(world.resource::<Content>());
    hud::ask_amount(
        world,
        AmountQuestion {
            title: LineText::plain("Offer how many ").then(named),
            amounts: 1..=left.count,
            suggested: left.count / 2,
            says: Box::new(move |count| format!("{count} of {}", left.count)),
            answer: Box::new(move |world, count| {
                if let Some(count) = count {
                    world.write_message(TradeRequest::Offer(ItemStack::new(left.item, count)));
                }
            }),
        },
    );
}

/// How many of the stack in the bag's slot `slot` are offered: an item's offer is counted out of
/// its slots in the bag's order.
fn aside(world: &World, slot: u32, _: ItemStack) -> u32 {
    let Some(view) = view(world) else {
        return 0;
    };
    let Some(bag) = session::my_character(world).and_then(|me| me.get::<Inventory>()) else {
        return 0;
    };
    let Some(held) = bag.slots.get(slot as usize) else {
        return 0;
    };
    let mut left: u32 = view
        .mine
        .iter()
        .filter(|offered| offered.item == held.item)
        .map(|offered| offered.count)
        .sum();
    for (index, stack) in bag.slots.iter().enumerate() {
        if stack.item != held.item {
            continue;
        }
        let taken = left.min(stack.count);
        if index == slot as usize {
            return taken;
        }
        left -= taken;
    }
    0
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    view(world)?;
    stack
        .item
        .get(world.resource::<Content>())
        .bound()
        .then(|| SlotVerdict::Refused(LineText::plain("Bound · can't be traded")))
}
