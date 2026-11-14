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
use crate::systems::hud::{self, Carried};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::item::{
    DropItemRequest, Inventory, ItemDef, ItemId, ItemModuleKind, ItemStack, card,
};
use crate::systems::player::session;
use crate::systems::skill::bar;

pub(crate) const SLOT: f32 = controls::SLOT;
const SLOT_GAP: f32 = 4.0;
const GHOST: f32 = 0.35;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SlotVerdict {
    Wanted(String),
    Refused(String),
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
            drops: free(InputAction::DropItem)
                && carried().any(|def| !def.has(ItemModuleKind::Bound)),
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
    /// The slot a view lets its reader choose, where it has something to say about the choice.
    pub selected: Option<&'a mut Option<u32>>,
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

pub fn inventory_pane(ui: &mut Ui, world: &mut World, mut pane: InventoryPane) {
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
                    bag_slot(ui, world, cell, pane.selected.as_deref_mut());
                }
            });
        },
    );
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

const BUILT_IN: [InputAction; 3] = [
    InputAction::DropItem,
    InputAction::UseItem,
    InputAction::InspectItem,
];

fn bag_slot(ui: &mut Ui, world: &mut World, cell: &CellData, selected: Option<&mut Option<u32>>) {
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
    let is_selected = selected.as_deref() == Some(&Some(cell.slot));
    let look = SlotLook {
        count: left,
        wanted,
        faded: refused || carried || left == 0,
        quest: def.has(ItemModuleKind::QuestItem),
        selected: is_selected,
        ..SlotLook::of(&content, filled.stack.item)
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            true,
            is_selected,
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
        act(world, cell.slot, filled.stack.item, &matched, response.rect);
    }
    if map::drag_started(world, InputAction::CarryItem, &response) {
        drag::carry(ui.ctx(), Carried::BagSlot(cell.slot));
    }
    let notes = filled.notes.iter().cloned().chain(verdict);
    let hint = StyledLine::new(map::hints(filled.hints.iter().cloned()));
    card::tooltip_noted(world, &response, filled.stack.item, notes, Some(&hint));
    if let Some(selected) = selected
        && map::clicked(world, InputAction::Select, &response)
    {
        *selected = Some(cell.slot);
    }
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
                    Filled {
                        stack,
                        aside: asides.iter().map(|source| source(world, slot, stack)).sum(),
                        hints: hints(map, &takeovers, def),
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

fn act(world: &mut World, slot: u32, item: ItemId, matched: &[InputAction], at: Rect) {
    let takeover = active_slot_actions(world)
        .into_iter()
        .find(|takeover| matched.contains(&takeover.action));
    if let Some(takeover) = takeover {
        (takeover.act)(world, slot);
        return;
    }
    match BUILT_IN.into_iter().find(|action| matched.contains(action)) {
        Some(InputAction::DropItem) => {
            world.write_message(DropItemRequest { slot });
        }
        Some(InputAction::UseItem) => {
            bar::use_bag_slot(world, slot);
        }
        Some(InputAction::InspectItem) => card::open(world, item, Some(at)),
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

fn hints(map: &InputMap, takeovers: &[SlotAction], def: &ItemDef) -> Vec<(InputAction, String)> {
    let inspect = (!shadowed(map, takeovers, InputAction::InspectItem))
        .then(|| (InputAction::InspectItem, " for more information".to_owned()));
    let usage = card::use_verb(def)
        .filter(|_| !shadowed(map, takeovers, InputAction::UseItem))
        .map(|verb| (InputAction::UseItem, format!(" to {verb}")));
    takeovers
        .iter()
        .map(|takeover| (takeover.action, format!(" to {}", takeover.verb)))
        .chain(inspect)
        .chain(usage)
        .collect()
}
