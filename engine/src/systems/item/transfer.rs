//! Moving items between the bag and a window that takes them: a trade, a new listing, a sale, a
//! message's attachments, a prompt. Every such window moves items one way, here, so a player who
//! learns it in one knows it in all:
//!
//! - Place stack moves the whole stack, Place item moves one, and Split stack asks how many.
//! - Carrying a stack onto the window moves all of it.
//!
//! The same inputs take items back out of the window's slots, and carrying a stack from them onto
//! the bag takes all of it back.

use std::any::TypeId;
use std::marker::PhantomData;

use bevy::prelude::*;
use ui::drag;
use ui::egui::{self, Sense, Ui};

use super::grid::item_grid;
use super::tray::{ItemTray, Placed};
use super::widget::{
    self, ItemTile, SlotVerdict, TileClicks, empty_slot, item_tile, slot_verdict_source,
};
use super::{ItemStack, card, my_bag};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::text::StyledLine;
use crate::systems::hud::{self, AmountQuestion, Carried};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::ItemMessages;
use crate::systems::mention::Mention;
use crate::systems::messages;

/// A window items move into from the bag and back out of. Which window it is, is its type.
pub trait TransferWindow: 'static {
    /// Whether the window takes items from the bag now.
    fn open(world: &World) -> bool;

    /// Moves `count` of the stack in the bag's slot `slot` in, never more than is left of it.
    /// Whether the window took it.
    fn put(world: &mut World, slot: u32, count: u32) -> bool;

    /// Moves `count` of the stack at `index` of the window's slots back out. Whether anything
    /// moved.
    fn take(world: &mut World, index: usize, count: u32) -> bool;

    /// How many of the stack in the bag's slot `slot` the window holds.
    fn aside(world: &World, slot: u32, stack: ItemStack) -> u32;

    /// What the bag's slot shows of `stack` while the window is open: an item that can't change
    /// hands is refused where it lies.
    fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
        Self::open(world)
            .then(|| bound_verdict(world, stack))
            .flatten()
    }
}

/// A window whose slots are a tray the player fills from their bag: what is placed stays in the
/// bag until whoever owns the tray sends it.
pub trait TrayWindow: 'static {
    /// Whether the window takes items from the bag now.
    fn open(world: &World) -> bool;

    /// What is placed.
    fn tray(world: &World) -> &ItemTray;

    /// What is placed, where the window lets it change: none while what was placed is being sent.
    fn tray_mut(world: &mut World) -> Option<&mut ItemTray>;

    /// How many slots the tray has.
    fn slots(world: &World) -> usize;

    /// Whether the window takes the stack `held` of the bag at all. An item that can't change
    /// hands is never taken, unless the window uses items up instead of handing them over.
    fn takes(world: &World, held: ItemStack) -> bool {
        !held.bound(world.resource::<Content>())
    }

    /// What the bag's slot shows of `stack` while the window is open.
    fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
        (Self::open(world) && !Self::takes(world, stack))
            .then(|| bound_verdict(world, stack))
            .flatten()
    }
}

/// Which pane of items, by its own type, a carried stack was taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferId(TypeId);

/// A window as the shared mechanics of moving items run it.
#[derive(Clone, Copy)]
pub struct ItemTransfer {
    id: TransferId,
    open: fn(&World) -> bool,
    put: fn(&mut World, u32, u32) -> bool,
    take: fn(&mut World, usize, u32) -> bool,
}

impl ItemTransfer {
    pub fn of<W: TransferWindow>() -> ItemTransfer {
        ItemTransfer {
            id: TransferId(TypeId::of::<W>()),
            open: W::open,
            put: W::put,
            take: W::take,
        }
    }

    pub fn of_tray<W: TrayWindow>() -> ItemTransfer {
        ItemTransfer::of::<Tray<W>>()
    }
}

/// Registers a window items move into from the bag.
pub fn transfer_window<W: TransferWindow>(app: &mut App) {
    widget::slot_aside_source(app, W::aside);
    slot_verdict_source(app, W::verdict);
    app.world_mut()
        .get_resource_or_init::<Transfers>()
        .0
        .push(ItemTransfer::of::<W>());
}

/// Registers a window whose slots are a tray.
pub fn tray_window<W: TrayWindow>(app: &mut App) {
    transfer_window::<Tray<W>>(app);
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

/// What a slot shows of an item that can't change hands.
pub fn bound_verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let content = world.resource::<Content>();
    stack
        .bound(content)
        .then(|| SlotVerdict::Refused(LineText::from(messages::of(content).item.bound_slot())))
}

/// The inputs that move items, each with what it does, for a list of what the inputs do.
pub fn moves(messages: &ItemMessages) -> [(InputAction, String); 3] {
    [
        (InputAction::PlaceStack, messages.move_stack().to_owned()),
        (InputAction::PlaceItem, messages.move_one().to_owned()),
        (InputAction::SplitStack, messages.move_some().to_owned()),
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
            let content = world.resource::<Content>();
            let title = messages::of(content)
                .item
                .move_how_many(Mention::Item(held.item).span(content));
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
        .and_then(|transfers| transfers.0.iter().find(|transfer| transfer.id == *from))
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
    let shown = item_grid(ui, pane.slots, |ui, index| {
        pane_slot(ui, world, &pane, index as usize);
    });
    let open = pane.transfer.filter(|transfer| (transfer.open)(world));
    if let (Some(transfer), Some(Carried::BagSlot(slot))) = (open, drag::dropped::<Carried>(&shown))
    {
        let left = left_in(world, slot);
        put(world, transfer, slot, left);
    }
}

/// A line that tells how items move: placed in the window's slots by the bag's, taken back by
/// their own.
pub fn how_to(content: &Content) -> StyledLine {
    messages::of(content)
        .item
        .moves_how(
            InputAction::PlaceItem,
            InputAction::SplitStack,
            InputAction::PlaceStack,
        )
        .styled(content)
}

#[derive(Resource, Default)]
struct Transfers(Vec<ItemTransfer>);

/// A window whose slots are a tray, as one that items move into.
struct Tray<W>(PhantomData<W>);

impl<W: TrayWindow> TransferWindow for Tray<W> {
    fn open(world: &World) -> bool {
        W::open(world)
    }

    fn put(world: &mut World, slot: u32, count: u32) -> bool {
        let Some(bag) = my_bag(world) else {
            return false;
        };
        let Some(held) = bag.at(slot as usize) else {
            return false;
        };
        if !W::takes(world, held) {
            return false;
        }
        let bag = bag.clone();
        let slots = W::slots(world);
        W::tray_mut(world).is_some_and(|tray| tray.place(&bag, slot, count, slots))
    }

    fn take(world: &mut World, index: usize, count: u32) -> bool {
        W::tray_mut(world).is_some_and(|tray| tray.take_back(index, count))
    }

    fn aside(world: &World, slot: u32, stack: ItemStack) -> u32 {
        my_bag(world).map_or(0, |bag| W::tray(world).aside(bag, slot, stack))
    }

    fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
        W::verdict(world, stack)
    }
}

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
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).item;
    let Some(&placed) = pane.placed.get(index) else {
        empty_slot(ui, world);
        return;
    };
    let stack = placed.stack;
    let carried = matches!(
        drag::carried::<Carried>(ui.ctx()),
        Some(Carried::Moved { from, index: at, .. })
            if Some(from) == pane.transfer.map(|transfer| transfer.id) && at == index
    );
    let moves = pane.transfer.map(|transfer| {
        [
            (
                InputAction::PlaceStack,
                messages.take_back_stack().to_owned(),
            ),
            (InputAction::PlaceItem, messages.take_back_one().to_owned()),
            (
                InputAction::SplitStack,
                messages.take_back_some().to_owned(),
            ),
        ]
        .map(|(action, said)| (transfer, action, said))
    });
    let taken: Vec<InputAction> = moves
        .iter()
        .flatten()
        .map(|(_, action, _)| *action)
        .collect();
    let map = world.resource::<InputMap>();
    let looks = card::inspect_hints(world)
        .into_iter()
        .filter(|(action, _)| !map.preempted(&taken, *action));
    let hints: Vec<(InputAction, String)> = moves
        .iter()
        .flatten()
        .map(|(_, action, said)| (*action, said.clone()))
        .chain(looks)
        .collect();
    let tile = ItemTile {
        refused: placed.short,
        faded: carried,
        notes: placed
            .short
            .then(|| LineText::from(messages.no_longer_carried()))
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
            messages.slot_placed(pane.named, stack.describe(&content)),
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
                let title = messages.take_back_how_many(Mention::Item(stack.item).span(&content));
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
        let from = transfer.id;
        drag::carry(ui.ctx(), Carried::Moved { from, index, stack });
    }
}

/// Asks how many of `stack` to move, under `title`, and hands `moved` the answer.
fn ask(
    world: &mut World,
    stack: ItemStack,
    title: LineText,
    moved: impl FnOnce(&mut World, u32) + Send + Sync + 'static,
) {
    let messages = messages::of(world.resource::<Content>()).item.clone();
    hud::ask_amount(
        world,
        AmountQuestion {
            title,
            amounts: 1..=stack.count,
            suggested: stack.count.div_ceil(2),
            says: Box::new(move |count| messages.some_of(stack.count, count)),
            answer: Box::new(move |world, count| {
                if let Some(count) = count {
                    moved(world, count);
                }
            }),
        },
    );
}
