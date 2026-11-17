use bevy::prelude::*;
use ui::drag;
use ui::egui;
use ui::egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::{controls, ink, sheet};

use crate::core::content::Content;
use crate::core::content::ImageRef;
use crate::core::gui::ImageSource;
use crate::core::math::ColorGradient;
use crate::core::text::StyledLine;
use crate::systems::equipment::UnequipRequest;
use crate::systems::hud::{self, AmountQuestion, Carried};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::item::inline::{self, InlineItem};
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::item::transfer;
use crate::systems::item::{
    DropItemRequest, Inventory, ItemDef, ItemId, ItemModuleKind, ItemStack, JoinStacksRequest,
    SplitStackRequest, card,
};
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::skill::bar;
use crate::systems::text::LineText;

pub(crate) const SLOT: f32 = controls::SLOT;
const SLOT_GAP: f32 = 4.0;
const GHOST: f32 = 0.35;

#[derive(Clone, Debug, PartialEq)]
pub enum SlotVerdict {
    Wanted(LineText),
    Refused(LineText),
}

pub type SlotVerdictSource = fn(&World, ItemStack) -> Option<SlotVerdict>;

pub fn slot_verdict_source(app: &mut App, source: SlotVerdictSource) {
    app.world_mut()
        .get_resource_or_init::<SlotVerdicts>()
        .0
        .push(source);
}

/// How many of the stack in a bag slot are set aside elsewhere, so the slot shows what is left.
pub type SlotAsideSource = fn(&World, u32, ItemStack) -> u32;

pub fn slot_aside_source(app: &mut App, source: SlotAsideSource) {
    app.world_mut()
        .get_resource_or_init::<SlotAsides>()
        .0
        .push(source);
}

pub type SlotNoteSource = fn(&World, ItemStack) -> Option<String>;

pub fn slot_note_source(app: &mut App, source: SlotNoteSource) {
    app.world_mut()
        .get_resource_or_init::<SlotNotes>()
        .0
        .push(source);
}

#[derive(Clone, Copy)]
pub struct SlotAction {
    pub action: InputAction,
    pub active: fn(&World) -> bool,
    pub act: fn(&mut World, u32),
    pub verb: &'static str,
}

pub fn slot_action(app: &mut App, slot_action: SlotAction) {
    app.world_mut()
        .get_resource_or_init::<SlotActions>()
        .0
        .push(slot_action);
}

/// Which inputs do something to what the player's character carries right now: there is an item
/// each could act on, and nothing has taken the input over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarriedInputs {
    pub uses: bool,
    pub carries: bool,
    pub drops: bool,
    pub inspects: bool,
}

impl CarriedInputs {
    pub fn of(world: &World) -> CarriedInputs {
        let Some(inventory) = session::my_character(world).and_then(|me| me.get::<Inventory>())
        else {
            return CarriedInputs::default();
        };
        let content = world.resource::<Content>();
        let map = world.resource::<InputMap>();
        let takeovers = takeovers(world);
        let free = |action| !shadowed(map, &takeovers, action);
        let carried = || inventory.slots.iter().map(|stack| stack.item.get(content));
        CarriedInputs {
            uses: free(InputAction::UseItem) && carried().any(|def| card::use_verb(def).is_some()),
            carries: !inventory.slots.is_empty(),
            drops: free(InputAction::DropItem) && carried().any(|def| !def.bound()),
            inspects: free(InputAction::InspectItem) && !inventory.slots.is_empty(),
        }
    }
}

pub const fn pane_width(columns: u32) -> f32 {
    columns as f32 * SLOT + columns.saturating_sub(1) as f32 * SLOT_GAP
}

pub struct InventoryPane<'a> {
    pub width: f32,
    /// A window that says how full the inventory is elsewhere leaves the heading out.
    pub heading: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryRoom {
    pub taken: u32,
    pub slots: u32,
}

impl InventoryRoom {
    pub fn full(self) -> bool {
        self.taken >= self.slots
    }
}

pub fn inventory_pane(ui: &mut Ui, world: &mut World, pane: InventoryPane) {
    let cells = inventory_cells(world);
    let shown = ui.allocate_ui_with_layout(
        Vec2::new(pane.width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(pane.width);
            if let Some(heading) = pane.heading {
                let InventoryRoom { taken, slots } = inventory_room(world);
                sheet::section(ui, &format!("{heading} · {taken} of {slots}"));
            }
            ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
            ui.horizontal_wrapped(|ui| {
                for cell in &cells {
                    bag_slot(ui, world, cell);
                }
            });
        },
    );
    if let Some(carried) = drag::dropped::<Carried>(&shown.response) {
        dropped_on_bag(world, carried);
    }
}

/// What is left of the stack in the bag's slot `slot`, of what the windows open hold of it.
pub(crate) fn bag_slot_left(world: &World, slot: u32) -> Option<ItemStack> {
    let stack = *session::my_character(world)?
        .get::<Inventory>()?
        .slots
        .get(slot as usize)?;
    let aside: u32 = slot_asides(world)
        .iter()
        .map(|source| source(world, slot, stack))
        .sum();
    Some(ItemStack::new(
        stack.item,
        stack.count.saturating_sub(aside),
    ))
}

pub fn inventory_room(world: &World) -> InventoryRoom {
    let inventory = session::my_character(world).and_then(|me| me.get::<Inventory>());
    InventoryRoom {
        taken: inventory.map_or(0, |inventory| inventory.slots.len() as u32),
        slots: inventory.map_or(0, |inventory| inventory.max),
    }
}

pub fn slot_notes(world: &World, stack: ItemStack) -> Vec<String> {
    let content = world.resource::<Content>();
    let notes = world
        .get_resource::<SlotNotes>()
        .map(|notes| notes.0.as_slice())
        .unwrap_or_default();
    bound_note(stack.item.get(content))
        .into_iter()
        .chain(notes.iter().filter_map(|source| source(world, stack)))
        .collect()
}

pub(crate) struct SlotLook<'a> {
    pub icon: Option<&'a ImageRef>,
    pub rarity: Option<SlotRarity<'a>>,
    pub count: u32,
    pub wanted: bool,
    pub faded: bool,
    pub refused: bool,
    pub quest: bool,
    pub selected: bool,
}

impl SlotLook<'_> {
    pub(crate) const EMPTY: SlotLook<'static> = SlotLook {
        icon: None,
        rarity: None,
        count: 0,
        wanted: false,
        faded: false,
        refused: false,
        quest: false,
        selected: false,
    };
}

/// The item tile: an item as a grid shows it, its icon in a slot edged and marked as its rarity
/// is, with how many it holds where that is more than one. Pointing at it shows the item's
/// tooltip, with the item's name, which the tile doesn't show, and clicking it opens the item's
/// card, unless the view it stands in or the row it leads takes that click.
pub(crate) struct ItemTile {
    pub stack: ItemStack,
    pub wanted: bool,
    pub faded: bool,
    pub refused: bool,
    pub selected: bool,
    /// Said under what the item is, in its tooltip.
    pub notes: Vec<LineText>,
    pub clicks: TileClicks,
}

/// Whose a click on an item tile is.
pub(crate) enum TileClicks {
    /// The tile's: it opens the item's card, and adds the item to chat when asked.
    Card,
    /// The view's, which answers the inputs `hint` names at the foot of the tooltip. The tile
    /// still adds the item to chat when asked.
    View { sense: Sense, hint: StyledLine },
    /// The row's the tile leads, beside the item's name: the row takes the click, and the
    /// tooltip leaves the name out.
    Row,
}

impl ItemTile {
    pub(crate) fn of(stack: ItemStack) -> ItemTile {
        ItemTile {
            stack,
            wanted: false,
            faded: false,
            refused: false,
            selected: false,
            notes: Vec::new(),
            clicks: TileClicks::Card,
        }
    }
}

pub(crate) fn item_tile(ui: &mut Ui, world: &mut World, tile: ItemTile) -> Response {
    let content = world.resource::<Content>().clone();
    let item = tile.stack.item;
    let look = SlotLook {
        wanted: tile.wanted,
        faded: tile.faded,
        refused: tile.refused,
        selected: tile.selected,
        ..SlotLook::tile(&content, tile.stack)
    };
    match tile.clicks {
        TileClicks::Card => {
            let response = slot(ui, world, look, Sense::click());
            answer_tile(world, &response, item, tile.notes);
            response
        }
        TileClicks::View { sense, hint } => {
            let response = slot(ui, world, look, sense);
            card::chat_shortcut(world, &response, &[item]);
            card::tooltip_noted(world, &response, item, tile.notes, Some(&hint));
            response
        }
        TileClicks::Row => {
            let response = slot(ui, world, look, Sense::hover());
            card::tooltip_beside_name(world, &response, item, tile.notes, None);
            response
        }
    }
}

/// An item drawn where the game isn't at hand, as on a card the editor previews, left to answer
/// the pointer once the game is.
pub enum DrawnItem {
    Inline(Response, InlineItem),
    /// An item tile whose clicks are its own.
    Tile(Response, ItemId),
}

/// Has each of `drawn` answer the pointer as its kind does: its tooltip, and its card to a click.
pub fn answer_drawn(world: &mut World, drawn: Vec<DrawnItem>) {
    for drawn in drawn {
        match drawn {
            DrawnItem::Inline(response, item) => {
                inline::inspect(world, response, item);
            }
            DrawnItem::Tile(response, item) => answer_tile(world, &response, item, []),
        }
    }
}

/// Draws `stack` as an item tile whose clicks are its own, where the game isn't at hand, from the
/// icons of `images`.
pub fn draw_tile(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    stack: ItemStack,
) -> DrawnItem {
    let response = slot(ui, images, SlotLook::tile(content, stack), Sense::click());
    DrawnItem::Tile(response, stack.item)
}

/// What a tile whose clicks are its own does to a pointer: opens the item's card to a click, adds
/// the item to chat when asked, and shows its tooltip.
fn answer_tile(
    world: &mut World,
    response: &Response,
    item: ItemId,
    notes: impl IntoIterator<Item = LineText>,
) {
    if map::clicked(world, InputAction::InspectItem, response) {
        card::open(world, item, Some(response.rect));
    }
    card::chat_shortcut(world, response, &[item]);
    let hint = StyledLine::new(card::inspect_hint());
    card::tooltip_noted(world, response, item, notes, Some(&hint));
}

impl<'a> SlotLook<'a> {
    /// A slot as an item tile shows `stack` in it: the item's icon, edged and marked as its
    /// rarity is, how many where more than one, and the mark of a quest's item.
    pub(crate) fn tile(content: &'a Content, stack: ItemStack) -> SlotLook<'a> {
        SlotLook {
            count: stack.count,
            quest: stack.item.get(content).has(ItemModuleKind::QuestItem),
            ..SlotLook::of(content, stack.item)
        }
    }

    /// A slot holding `item`: its icon, edged and marked as its rarity is.
    pub(crate) fn of(content: &'a Content, item: ItemId) -> SlotLook<'a> {
        let def = item.get(content);
        SlotLook {
            icon: Some(&def.icon),
            rarity: Some(SlotRarity::of(content, def.rarity)),
            ..SlotLook::EMPTY
        }
    }

    /// The colours its slot is edged in, ghosted as its icon is.
    pub(crate) fn edge(&self) -> Option<[Color32; 2]> {
        self.rarity.map(|rarity| {
            rarity
                .colors()
                .map(|color| color.gamma_multiply(self.ghost()))
        })
    }

    fn ghost(&self) -> f32 {
        if self.faded { GHOST } else { 1.0 }
    }
}

/// What a slot shows of its item's rarity: the colours of its edge, and the icon in its corner.
#[derive(Clone, Copy)]
pub(crate) struct SlotRarity<'a> {
    pub gradient: ColorGradient,
    pub icon: Option<&'a ImageRef>,
}

impl<'a> SlotRarity<'a> {
    pub(crate) fn of(content: &'a Content, rarity: ItemRarityId) -> SlotRarity<'a> {
        let def = rarity.get(content);
        SlotRarity {
            gradient: def.gradient,
            icon: def.icon.as_ref(),
        }
    }

    pub(crate) fn colors(self) -> [Color32; 2] {
        self.gradient.colors()
    }
}

/// The rarity's icon in the corner of the slot at `rect`.
pub(crate) fn slot_pip(
    ui: &Ui,
    images: &mut (impl ImageSource + ?Sized),
    rect: Rect,
    look: &SlotLook,
) {
    if let Some(pip) = look
        .rarity
        .and_then(|rarity| rarity.icon)
        .and_then(|icon| images.image(icon))
    {
        let tint = Color32::from_white_alpha((look.ghost() * 255.0) as u8);
        controls::slot_pip(ui.painter(), rect, pip.texture, tint);
    }
}

/// A slot as the bag draws one of an item of `rarity`, edged and marked as the rarity is, and
/// holding `item`'s icon where there is an item to show in it.
pub fn shown_slot(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    rarity: ItemRarityId,
    item: Option<ItemId>,
) -> Response {
    slot(
        ui,
        images,
        SlotLook {
            icon: item.map(|item| &item.get(content).icon),
            rarity: Some(SlotRarity::of(content, rarity)),
            ..SlotLook::EMPTY
        },
        Sense::hover(),
    )
}

pub(crate) fn slot(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    look: SlotLook,
    sense: Sense,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SLOT), sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = response.hovered() && sense != Sense::hover();
    controls::slot_frame(
        ui.painter(),
        rect,
        controls::SlotLook {
            filled: look.icon.is_some(),
            hovered,
            refused: look.refused,
            edge: look.edge(),
        },
    );
    if look.selected {
        ui.painter()
            .rect_stroke(rect, 3.0, Stroke::new(2.0, ink::INK), StrokeKind::Inside);
    } else if look.wanted {
        ui.painter()
            .rect_stroke(rect, 3.0, Stroke::new(1.5, ink::OCHRE), StrokeKind::Inside);
    }
    if let Some(image) = look.icon.and_then(|icon| images.image(icon)) {
        let tint = Color32::from_white_alpha((look.ghost() * 255.0) as u8);
        ui.place(
            Rect::from_center_size(rect.center(), Vec2::splat(controls::SLOT_ICON)),
            image.widget(Vec2::splat(controls::SLOT_ICON)).tint(tint),
        );
    }
    slot_pip(ui, images, rect, &look);
    if look.count > 1 {
        controls::slot_count(ui.painter(), rect, look.count);
    }
    if look.quest {
        controls::slot_lozenge(ui.painter(), rect);
    }
    if response.has_focus() {
        controls::focus_ring(ui.painter(), rect, 3.0);
    }
    response
}

#[derive(Resource, Default)]
struct SlotVerdicts(Vec<SlotVerdictSource>);

#[derive(Resource, Default)]
struct SlotNotes(Vec<SlotNoteSource>);

#[derive(Resource, Default)]
struct SlotAsides(Vec<SlotAsideSource>);

#[derive(Resource, Default)]
struct SlotActions(Vec<SlotAction>);

struct CellData {
    slot: u32,
    filled: Option<Filled>,
}

struct Filled {
    stack: ItemStack,
    aside: u32,
    hints: Vec<(InputAction, String)>,
    notes: Vec<String>,
    verdict: Option<SlotVerdict>,
}

const TRANSFERS: [InputAction; 3] = [
    InputAction::PlaceStack,
    InputAction::PlaceItem,
    InputAction::SplitStack,
];

const BUILT_IN: [InputAction; 4] = [
    InputAction::DropItem,
    InputAction::SplitStack,
    InputAction::UseItem,
    InputAction::InspectItem,
];

fn bag_slot(ui: &mut Ui, world: &mut World, cell: &CellData) {
    let Some(filled) = &cell.filled else {
        slot(ui, world, SlotLook::EMPTY, Sense::hover());
        return;
    };
    let content = world.resource::<Content>().clone();
    let def = filled.stack.item.get(&content);
    let (wanted, refused, verdict) = match &filled.verdict {
        None => (false, false, None),
        Some(SlotVerdict::Wanted(note)) => (true, false, Some(note.clone())),
        Some(SlotVerdict::Refused(reason)) => (false, true, Some(reason.clone())),
    };
    let carried = drag::carried::<Carried>(ui.ctx()) == Some(Carried::BagSlot(cell.slot));
    let left = filled.stack.count.saturating_sub(filled.aside);
    let notes = filled
        .notes
        .iter()
        .cloned()
        .map(LineText::from)
        .chain(verdict)
        .collect();
    let tile = ItemTile {
        wanted,
        faded: refused || carried || left == 0,
        notes,
        clicks: TileClicks::View {
            sense: Sense::click_and_drag(),
            hint: StyledLine::new(map::hints(filled.hints.iter().cloned())),
        },
        ..ItemTile::of(ItemStack::new(filled.stack.item, left))
    };
    let response = item_tile(ui, world, tile);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!(
                "Inventory slot {}: {}",
                cell.slot + 1,
                described(left, filled.aside, &def.name)
            ),
        )
    });
    let candidates: Vec<InputAction> = world
        .get_resource::<SlotActions>()
        .into_iter()
        .flat_map(|actions| actions.0.iter().map(|action| action.action))
        .chain(TRANSFERS)
        .chain(BUILT_IN)
        .collect();
    let matched: Vec<InputAction> = candidates
        .into_iter()
        .filter(|action| map::clicked(world, *action, &response))
        .collect();
    if !matched.is_empty() {
        let held = ItemStack::new(filled.stack.item, left);
        act(world, cell.slot, held, &matched, response.rect);
    }
    if map::drag_started(world, InputAction::CarryItem, &response) {
        drag::carry(ui.ctx(), Carried::BagSlot(cell.slot));
    }
    match drag::dropped::<Carried>(&response) {
        Some(Carried::BagSlot(from)) if from != cell.slot => {
            world.write_message(JoinStacksRequest {
                from,
                onto: cell.slot,
            });
        }
        Some(carried) => dropped_on_bag(world, carried),
        None => {}
    }
}

/// Asks how many of `stack` to split off, and hands `split` the answer.
fn ask_split(
    world: &mut World,
    stack: ItemStack,
    split: impl FnOnce(&mut World, u32) + Send + Sync + 'static,
) {
    if stack.count < 2 {
        return;
    }
    let named = Mention::Item(stack.item).span(world.resource::<Content>());
    hud::ask_amount(
        world,
        AmountQuestion {
            title: LineText::plain("Split stack of ").then(named),
            amounts: 1..=stack.count - 1,
            suggested: stack.count / 2,
            says: Box::new(move |count| {
                format!(
                    "{count} in the new stack, {} left in this one.",
                    stack.count - count
                )
            }),
            answer: Box::new(move |world, count| {
                if let Some(count) = count {
                    split(world, count);
                }
            }),
        },
    );
}

fn inventory_cells(world: &World) -> Vec<CellData> {
    let content = world.resource::<Content>();
    let inventory = session::my_character(world).and_then(|me| me.get::<Inventory>());
    let max = inventory.map_or(0, |inventory| inventory.max);
    let verdicts = world
        .get_resource::<SlotVerdicts>()
        .map(|verdicts| verdicts.0.as_slice())
        .unwrap_or_default();
    let asides = slot_asides(world);
    let takeovers = takeovers(world);
    let map = world.resource::<InputMap>();
    (0..max)
        .map(|slot| CellData {
            slot,
            filled: inventory
                .and_then(|inventory| inventory.slots.get(slot as usize))
                .map(|&stack| {
                    let def = stack.item.get(content);
                    let aside: u32 = asides.iter().map(|source| source(world, slot, stack)).sum();
                    Filled {
                        stack,
                        aside,
                        hints: hints(map, &takeovers, def, stack.count.saturating_sub(aside)),
                        notes: slot_notes(world, stack),
                        verdict: verdicts.iter().find_map(|source| source(world, stack)),
                    }
                }),
        })
        .collect()
}

/// Takes back into the bag what was carried onto it from elsewhere: what is worn, and what was
/// moved into a window.
fn dropped_on_bag(world: &mut World, carried: Carried) {
    match carried {
        Carried::Worn(slot) => {
            world.write_message(UnequipRequest { slot });
        }
        carried => transfer::dropped_on_bag(world, &carried),
    }
}

fn slot_asides(world: &World) -> &[SlotAsideSource] {
    world
        .get_resource::<SlotAsides>()
        .map(|asides| asides.0.as_slice())
        .unwrap_or_default()
}

fn bound_note(def: &ItemDef) -> Option<String> {
    def.has(ItemModuleKind::Bound)
        .then(|| "bound · can't be sold or dropped".to_owned())
}

/// Does what the inputs `matched` ask of the bag's slot `slot`, where `held` is what is left
/// in it of what is set aside elsewhere: nothing but a look at the item, when that is none.
fn act(world: &mut World, slot: u32, held: ItemStack, matched: &[InputAction], at: Rect) {
    if let Some(open) = transfer::open(world)
        && let Some(&action) = matched.iter().find(|action| TRANSFERS.contains(action))
        && transfer::from_bag(world, open, slot, held, action)
    {
        return;
    }
    let takeover = active_slot_actions(world)
        .into_iter()
        .find(|takeover| matched.contains(&takeover.action));
    if let Some(takeover) = takeover {
        (takeover.act)(world, slot);
        return;
    }
    match BUILT_IN.into_iter().find(|action| matched.contains(action)) {
        Some(InputAction::InspectItem) => card::open(world, held.item, Some(at)),
        Some(_) if held.count == 0 => {}
        Some(InputAction::DropItem) => {
            world.write_message(DropItemRequest { slot });
        }
        Some(InputAction::SplitStack) => ask_split(world, held, move |world, count| {
            world.write_message(SplitStackRequest { slot, count });
        }),
        Some(InputAction::UseItem) => {
            bar::use_bag_slot(world, slot);
        }
        _ => {}
    }
}

fn active_slot_actions(world: &World) -> Vec<SlotAction> {
    world
        .get_resource::<SlotActions>()
        .map(|actions| {
            actions
                .0
                .iter()
                .copied()
                .filter(|action| (action.active)(world))
                .collect()
        })
        .unwrap_or_default()
}

/// The inputs something takes inventory slots over for right now, each with what it does: the
/// window items move into, and what else asks for the slots.
fn takeovers(world: &World) -> Vec<(InputAction, String)> {
    let moves = transfer::open(world)
        .map(|open| transfer::bag_moves(&open))
        .into_iter()
        .flatten();
    let others = active_slot_actions(world)
        .into_iter()
        .map(|takeover| (takeover.action, format!(" to {}", takeover.verb)));
    moves.chain(others).collect()
}

/// Whether something that takes inventory slots over right now answers a binding of `action`,
/// which then does nothing of its own.
fn shadowed(map: &InputMap, takeovers: &[(InputAction, String)], action: InputAction) -> bool {
    takeovers.iter().any(|(takeover, _)| {
        map.bindings(*takeover)
            .iter()
            .any(|binding| map.bindings(action).contains(binding))
    })
}

fn hints(
    map: &InputMap,
    takeovers: &[(InputAction, String)],
    def: &ItemDef,
    held: u32,
) -> Vec<(InputAction, String)> {
    let inspect = (!shadowed(map, takeovers, InputAction::InspectItem))
        .then(|| (InputAction::InspectItem, INSPECTS.to_owned()));
    let usage = card::use_verb(def)
        .filter(|_| !shadowed(map, takeovers, InputAction::UseItem))
        .map(|verb| (InputAction::UseItem, format!(" to {verb}")));
    let splits = (held > 1 && !shadowed(map, takeovers, InputAction::SplitStack))
        .then(|| (InputAction::SplitStack, SPLITS.to_owned()));
    takeovers
        .iter()
        .cloned()
        .chain(inspect)
        .chain(usage)
        .chain(splits)
        .chain([(InputAction::AddToChat, CHATS.to_owned())])
        .collect()
}

const INSPECTS: &str = " for more information";
const SPLITS: &str = " to split the stack";
const CHATS: &str = " to add it to chat";

/// What a slot holds as its label tells it: the count still in the bag, and how much of the stack
/// the window it shows in has set aside.
fn described(left: u32, aside: u32, name: &str) -> String {
    match (left, aside) {
        (_, 0) => format!("{left} {name}"),
        (0, _) => format!("{name}, all set aside"),
        _ => format!("{left} {name}, {aside} more set aside"),
    }
}
