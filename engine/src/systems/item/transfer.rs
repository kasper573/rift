//! Moving items between the bag and a window that takes them: a trade, a new listing, a sale, a
//! message's attachments, a prompt. Every such window moves items one way, here, so a player who
//! learns it in one knows it in all:
//!
//! - Place stack moves the whole stack, Place item moves one, and Split stack asks how many.
//! - Carrying a stack onto the window moves all of it.
//!
//! The same inputs take items back out of the window's slots, and carrying a stack from them onto
//! the bag takes all of it back.

use bevy::prelude::*;
use ui::drag;
use ui::egui::{self, Sense, Ui, Vec2};

use super::tray::Placed;
use super::widget::{self, ItemTile, SLOT, SlotLook, TileClicks, item_tile, slot};
use super::{ItemStack, card};
use crate::core::content::Content;
use crate::core::text::{StyledLine, TextPiece};
use crate::systems::hud::{self, AmountQuestion, Carried};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::mention::Mention;
use crate::systems::text::LineText;

/// A window items move into from the bag and back out of.
#[derive(Clone, Copy)]
pub struct ItemTransfer {
    /// Which window it is, among those items move between.
    pub key: &'static str,
    /// Whether the window takes items from the bag now.
    pub open: fn(&World) -> bool,
    /// What moving an item in is called, as a verb: "offer", "attach", "place".
    pub verb: &'static str,
    /// Moves `count` of the stack in the bag's slot `slot` in, never more than is left of it.
    /// Whether the window took it.
    pub put: fn(&mut World, u32, u32) -> bool,
    /// Moves `count` of the stack at `index` of the window's slots back out. Whether anything
    /// moved.
    pub take: fn(&mut World, usize, u32) -> bool,
    /// How many of the stack in the bag's slot `slot` the window holds.
    pub aside: fn(&World, u32, ItemStack) -> u32,
}

pub fn item_transfer(app: &mut App, transfer: ItemTransfer) {
    widget::slot_aside_source(app, transfer.aside);
    app.world_mut()
        .get_resource_or_init::<Transfers>()
        .0
        .push(transfer);
}

/// The window that takes items now, if one does.
pub fn open(world: &World) -> Option<ItemTransfer> {
    world
        .get_resource::<Transfers>()?
        .0
        .iter()
        .copied()
        .find(|transfer| (transfer.open)(world))
}

/// The inputs that move items, each with what it does, for a list of what a page's inputs do.
pub const MOVES: [(InputAction, &str); 3] = [
    (InputAction::PlaceStack, "move the stack"),
    (InputAction::PlaceItem, "move one"),
    (InputAction::SplitStack, "move some"),
];

/// The inputs that move items, each with what it does to a stack in the bag.
pub fn bag_moves(transfer: &ItemTransfer) -> [(InputAction, String); 3] {
    let verb = transfer.verb;
    [
        (InputAction::PlaceStack, format!(" to {verb} the stack")),
        (InputAction::PlaceItem, format!(" to {verb} one")),
        (InputAction::SplitStack, format!(" to {verb} some")),
    ]
}

/// Does what `action` asks of the bag's slot `slot` for `transfer`, `held` being what is left of
/// its stack. Whether `action` is one that moves items.
pub fn from_bag(
    world: &mut World,
    transfer: ItemTransfer,
    slot: u32,
    held: ItemStack,
    action: InputAction,
) -> bool {
    let Some(count) = amount(action, held.count) else {
        return false;
    };
    match count {
        Amount::Some => {
            let title = LineText::plain(format!("{} how many ", capitalized(transfer.verb)));
            ask(world, held, title, move |world, count| {
                put(world, transfer, slot, count);
            });
        }
        Amount::Exactly(count) => put(world, transfer, slot, count),
    }
    true
}

/// Takes all of what was carried from a window's slots back into the bag.
pub(crate) fn dropped_on_bag(world: &mut World, carried: &Carried) {
    let Carried::Moved { from, index, stack } = carried else {
        return;
    };
    let transfer = world
        .get_resource::<Transfers>()
        .and_then(|transfers| transfers.0.iter().find(|transfer| transfer.key == *from))
        .copied();
    if let Some(transfer) = transfer {
        take(world, transfer, *index, stack.count);
    }
}

/// The slots of a window that holds items moved in, `slots` of them: `placed` in theirs, each
/// named `named` to whoever reads the interface, and `transfer` taking them back out, where the
/// slots aren't only to read.
pub struct TransferPane<'a> {
    pub placed: &'a [Placed],
    pub slots: u32,
    pub named: &'a str,
    pub transfer: Option<ItemTransfer>,
}

pub fn transfer_pane(ui: &mut Ui, world: &mut World, pane: TransferPane) {
    let shown = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
            ui.horizontal_wrapped(|ui| {
                for index in 0..pane.slots as usize {
                    pane_slot(ui, world, &pane, index);
                }
            });
        })
        .response;
    let open = pane.transfer.filter(|transfer| (transfer.open)(world));
    if let (Some(transfer), Some(Carried::BagSlot(slot))) = (open, drag::dropped::<Carried>(&shown))
    {
        let left = left_in(world, slot);
        put(world, transfer, slot, left);
    }
}

/// How tall a pane of `slots` slots stands `width` wide.
pub fn slots_height(slots: u32, width: f32) -> f32 {
    let across = (((width + SLOT_GAP) / (SLOT + SLOT_GAP)).floor() as u32).max(1);
    let rows = slots.div_ceil(across).max(1) as f32;
    rows * SLOT + (rows - 1.0) * SLOT_GAP
}

/// How wide a pane `across` slots wide is.
pub const fn slots_width(across: u32) -> f32 {
    across as f32 * SLOT + across.saturating_sub(1) as f32 * SLOT_GAP
}

/// A line that tells how items move: placed in the window's slots by the bag's, taken back by
/// their own.
pub fn how_to(transfer: &ItemTransfer) -> StyledLine {
    StyledLine::new(vec![
        input(InputAction::PlaceStack),
        TextPiece::text(format!(" {}s the stack · ", transfer.verb)),
        input(InputAction::PlaceItem),
        TextPiece::text(" one · "),
        input(InputAction::SplitStack),
        TextPiece::text(" some · the same takes it back"),
    ])
}

#[derive(Resource, Default)]
struct Transfers(Vec<ItemTransfer>);

const SLOT_GAP: f32 = 6.0;

/// How many an input moves of a stack of `count`.
enum Amount {
    Exactly(u32),
    /// As many as the player says.
    Some,
}

fn amount(action: InputAction, count: u32) -> Option<Amount> {
    match action {
        InputAction::PlaceStack => Some(Amount::Exactly(count)),
        InputAction::PlaceItem => Some(Amount::Exactly(count.min(1))),
        InputAction::SplitStack if count > 1 => Some(Amount::Some),
        InputAction::SplitStack => Some(Amount::Exactly(count)),
        _ => None,
    }
}

fn put(world: &mut World, transfer: ItemTransfer, slot: u32, count: u32) {
    let moved = count > 0 && (transfer.put)(world, slot, count);
    sound(world, moved);
}

fn take(world: &mut World, transfer: ItemTransfer, index: usize, count: u32) {
    let moved = count > 0 && (transfer.take)(world, index, count);
    sound(world, moved);
}

fn sound(world: &mut World, moved: bool) {
    if moved {
        InterfaceSound::UiMove.play(world);
    } else {
        InterfaceSound::UiRefuse.play(world);
    }
}

/// What is left of the stack in the bag's slot `slot`, of what the windows hold.
fn left_in(world: &World, slot: u32) -> u32 {
    widget::bag_slot_left(world, slot).map_or(0, |held| held.count)
}

/// The slot at `index` of a transfer pane.
fn pane_slot(ui: &mut Ui, world: &mut World, pane: &TransferPane, index: usize) {
    let Some(&placed) = pane.placed.get(index) else {
        slot(ui, world, SlotLook::EMPTY, Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Empty slot"));
        return;
    };
    let stack = placed.stack;
    let content = world.resource::<Content>().clone();
    let carried = matches!(
        drag::carried::<Carried>(ui.ctx()),
        Some(Carried::Moved { from, index: at, .. })
            if Some(from) == pane.transfer.map(|transfer| transfer.key) && at == index
    );
    let moves = pane.transfer.map(|transfer| {
        [
            (InputAction::PlaceStack, " to take it back".to_owned()),
            (InputAction::PlaceItem, " to take one back".to_owned()),
            (InputAction::SplitStack, " to take some back".to_owned()),
        ]
        .map(|(action, said)| (transfer, action, said))
    });
    let hints = moves
        .iter()
        .flatten()
        .map(|(_, action, said)| (*action, said.clone()))
        .chain([
            (InputAction::InspectItem, " for more information".to_owned()),
            (InputAction::AddToChat, " to add it to chat".to_owned()),
        ]);
    let tile = ItemTile {
        refused: placed.short,
        faded: carried,
        notes: placed
            .short
            .then(|| LineText::plain("You no longer carry this"))
            .into_iter()
            .collect(),
        clicks: TileClicks::View {
            sense: match pane.transfer {
                Some(_) => Sense::click_and_drag(),
                None => Sense::click(),
            },
            hint: StyledLine::new(map::hints(hints)),
        },
        ..ItemTile::of(stack)
    };
    let response = item_tile(ui, world, tile);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{}: {}", pane.named, stack.describe(&content)),
        )
    });
    let mut acted = false;
    for &(transfer, action, _) in moves.iter().flatten() {
        if acted || !map::clicked(world, action, &response) {
            continue;
        }
        acted = true;
        match amount(action, stack.count) {
            Some(Amount::Exactly(count)) => take(world, transfer, index, count),
            Some(Amount::Some) => {
                let title = LineText::plain("Take back how many ");
                ask(world, stack, title, move |world, count| {
                    take(world, transfer, index, count);
                });
            }
            None => {}
        }
    }
    if !acted && map::clicked(world, InputAction::InspectItem, &response) {
        card::open(world, stack.item, Some(response.rect));
    }
    if let Some(transfer) = pane.transfer
        && map::drag_started(world, InputAction::CarryItem, &response)
    {
        let from = transfer.key;
        drag::carry(ui.ctx(), Carried::Moved { from, index, stack });
    }
}

/// Asks how many of `stack` to move, `title` then its item, and hands `moved` the answer.
fn ask(
    world: &mut World,
    stack: ItemStack,
    title: LineText,
    moved: impl FnOnce(&mut World, u32) + Send + Sync + 'static,
) {
    let named = Mention::Item(stack.item).span(world.resource::<Content>());
    hud::ask_amount(
        world,
        AmountQuestion {
            title: title.then(named),
            amounts: 1..=stack.count,
            suggested: stack.count.div_ceil(2),
            says: Box::new(move |count| format!("{count} of {}", stack.count)),
            answer: Box::new(move |world, count| {
                if let Some(count) = count {
                    moved(world, count);
                }
            }),
        },
    );
}

fn capitalized(verb: &str) -> String {
    let mut letters = verb.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}
