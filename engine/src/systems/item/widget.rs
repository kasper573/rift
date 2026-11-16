use bevy::prelude::*;
use ui::drag;
use ui::egui;
use ui::egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::{controls, ink, paint, sheet};

use crate::core::content::Content;
use crate::core::content::ImageRef;
use crate::core::gui::ImageSource;
use crate::core::math::ColorGradient;
use crate::core::text::StyledLine;
use crate::systems::hud::{self, AmountQuestion, Carried};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::item::rarity::ItemRarityId;
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
        let takeovers = active_slot_actions(world);
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

pub fn pane_width(columns: u32) -> f32 {
    columns as f32 * SLOT + columns.saturating_sub(1) as f32 * SLOT_GAP
}

pub struct InventoryPane<'a> {
    pub width: f32,
    /// A window that says how full the inventory is elsewhere leaves the heading out.
    pub heading: Option<&'a str>,
}

/// Stacks kept apart from the bag, shown slot by slot as the bag shows its own, and split as
/// the bag's are.
pub struct StackPane<'a> {
    pub width: f32,
    pub stacks: &'a [ItemStack],
    /// How many slots it shows, filled or not.
    pub slots: u32,
    /// How a slot is named to whoever reads the interface.
    pub named: &'a str,
    /// What picking a stack does, as its hint says it.
    pub pick: &'static str,
    /// Splits this many off the stack at this index, into a stack of its own beside it.
    pub split: fn(&mut World, usize, u32),
}

/// What the player did to the slots of a stack pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackPaneDone {
    /// Picked the stack at this index.
    Picked(usize),
    /// Dropped what they carried from this slot of the bag onto the pane.
    Dropped(u32),
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
    ui.allocate_ui_with_layout(
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
}

pub fn stack_pane(ui: &mut Ui, world: &mut World, pane: StackPane) -> Option<StackPaneDone> {
    let mut picked = None;
    let shown = ui
        .allocate_ui_with_layout(
            Vec2::new(pane.width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(pane.width);
                ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
                ui.horizontal_wrapped(|ui| {
                    for index in 0..pane.slots as usize {
                        if apart_slot(ui, world, &pane, index) {
                            picked = Some(index);
                        }
                    }
                });
            },
        )
        .response;
    if let Some(index) = picked {
        return Some(StackPaneDone::Picked(index));
    }
    match drag::dropped::<Carried>(&shown) {
        Some(Carried::BagSlot(slot)) => Some(StackPaneDone::Dropped(slot)),
        _ => None,
    }
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

impl<'a> SlotLook<'a> {
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

/// An item's icon where it is listed beside its name rather than held in a slot, `size` across
/// and edged in its rarity's colours as a slot that size would be.
pub(crate) fn listed_icon(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    item: ItemId,
    size: f32,
) -> Response {
    let response = hud::icon(ui, images, &item.get(content).icon, size);
    let scale = size / controls::SLOT_ICON;
    paint::gradient_edge(
        ui.painter(),
        response
            .rect
            .expand((SLOT - controls::SLOT_ICON) / 2.0 * scale),
        controls::SLOT_EDGE * scale,
        SlotRarity::of(content, item.get(content).rarity).colors(),
    );
    response
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
    let look = SlotLook {
        count: left,
        wanted,
        faded: refused || carried || left == 0,
        quest: def.has(ItemModuleKind::QuestItem),
        ..SlotLook::of(&content, filled.stack.item)
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Inventory slot {}: {left} {}", cell.slot + 1, def.name),
        )
    });
    let candidates: Vec<InputAction> = world
        .get_resource::<SlotActions>()
        .into_iter()
        .flat_map(|actions| actions.0.iter().map(|action| action.action))
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
    if let Some(Carried::BagSlot(from)) = drag::dropped::<Carried>(&response)
        && from != cell.slot
    {
        world.write_message(JoinStacksRequest {
            from,
            onto: cell.slot,
        });
    }
    let notes = filled
        .notes
        .iter()
        .cloned()
        .map(LineText::from)
        .chain(verdict);
    let hint = StyledLine::new(map::hints(filled.hints.iter().cloned()));
    card::chat_shortcut(world, &response, &[filled.stack.item]);
    card::tooltip_noted(world, &response, filled.stack.item, notes, Some(&hint));
}

/// The slot at `index` of a stack pane. Whether its stack was picked.
fn apart_slot(ui: &mut Ui, world: &mut World, pane: &StackPane, index: usize) -> bool {
    let Some(&stack) = pane.stacks.get(index) else {
        slot(ui, world, SlotLook::EMPTY, Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Empty slot"));
        return false;
    };
    let content = world.resource::<Content>().clone();
    let def = stack.item.get(&content);
    let look = SlotLook {
        count: stack.count,
        quest: def.has(ItemModuleKind::QuestItem),
        ..SlotLook::of(&content, stack.item)
    };
    let response = slot(ui, world, look, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{}: {} {}", pane.named, stack.count, def.name),
        )
    });
    let split = pane.split;
    if map::clicked(world, InputAction::SplitStack, &response) {
        ask_split(world, stack, move |world, count| split(world, index, count));
    } else if map::clicked(world, InputAction::InspectItem, &response) {
        card::open(world, stack.item, Some(response.rect));
    }
    let splits = (stack.count > 1).then(|| (InputAction::SplitStack, SPLITS.to_owned()));
    let hints = [(InputAction::Select, format!(" to {}", pane.pick))]
        .into_iter()
        .chain(splits)
        .chain([
            (InputAction::InspectItem, INSPECTS.to_owned()),
            (InputAction::AddToChat, CHATS.to_owned()),
        ]);
    let hint = StyledLine::new(map::hints(hints));
    card::chat_shortcut(world, &response, &[stack.item]);
    card::tooltip_noted(
        world,
        &response,
        stack.item,
        slot_notes(world, stack).into_iter().map(LineText::from),
        Some(&hint),
    );
    map::clicked(world, InputAction::Select, &response)
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
    let asides = world
        .get_resource::<SlotAsides>()
        .map(|asides| asides.0.as_slice())
        .unwrap_or_default();
    let takeovers = active_slot_actions(world);
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

fn bound_note(def: &ItemDef) -> Option<String> {
    def.has(ItemModuleKind::Bound)
        .then(|| "bound · can't be sold or dropped".to_owned())
}

/// Does what the inputs `matched` ask of the bag's slot `slot`, where `held` is what is left
/// in it of what is set aside elsewhere: nothing but a look at the item, when that is none.
fn act(world: &mut World, slot: u32, held: ItemStack, matched: &[InputAction], at: Rect) {
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

/// Whether something that takes inventory slots over right now answers a binding of `action`,
/// which then does nothing of its own.
fn shadowed(map: &InputMap, takeovers: &[SlotAction], action: InputAction) -> bool {
    takeovers.iter().any(|takeover| {
        map.bindings(takeover.action)
            .iter()
            .any(|binding| map.bindings(action).contains(binding))
    })
}

fn hints(
    map: &InputMap,
    takeovers: &[SlotAction],
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
        .map(|takeover| (takeover.action, format!(" to {}", takeover.verb)))
        .chain(inspect)
        .chain(usage)
        .chain(splits)
        .chain([(InputAction::AddToChat, CHATS.to_owned())])
        .collect()
}

const INSPECTS: &str = " for more information";
const SPLITS: &str = " to split the stack";
const CHATS: &str = " to add it to chat";
