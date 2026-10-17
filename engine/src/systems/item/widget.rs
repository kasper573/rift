use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::{HAIRLINE, WIDGET_RADIUS, color};
use egui::{
    Align, Color32, Id, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    Vec2,
};

use crate::core::assets::AssetRef;
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::{StyledLine, TextPiece};
use crate::systems::hud::{self, HudWindow};
use crate::systems::input::map::{self, InputAction, InputMap, input};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::{
    DropItemRequest, INVENTORY_MAX, Inventory, ItemDef, ItemFlag, ItemId, ItemStack,
    UseItemRequest, card,
};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

pub(crate) const SLOT: f32 = 36.0;
const SLOT_ICON: f32 = 32.0;
const CARRIED: f32 = 40.0;
const SLOT_GAP: f32 = 2.0;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarriedSlot(pub u32);

pub struct CarryPlugin;

impl Plugin for CarryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            EguiPrimaryContextPass,
            draw_carried
                .in_set(Band::Pointer)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

pub struct InventoryWindow;

impl HudWindow for InventoryWindow {
    fn title(&self) -> &'static str {
        "Inventory"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleInventory
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Bag
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let cells = inventory_cells(world);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
                ui.horizontal_wrapped(|ui| {
                    for cell in &cells {
                        bag_slot(ui, world, cell);
                    }
                });
            });
    }
}

pub(crate) struct SlotLook<'a> {
    pub icon: Option<&'a AssetRef>,
    pub count: u32,
    pub outline: Option<Color32>,
    pub faded: bool,
}

impl SlotLook<'_> {
    pub(crate) const EMPTY: SlotLook<'static> = SlotLook {
        icon: None,
        count: 0,
        outline: None,
        faded: false,
    };
}

pub(crate) fn slot(ui: &mut Ui, world: &mut World, look: SlotLook, sense: Sense) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SLOT), sense);
    let stroke = match look.outline {
        Some(outline) => Stroke::new(1.5, outline),
        None if response.hovered() && sense != Sense::hover() => {
            Stroke::new(1.0, color::HOVER_LINE)
        }
        None => HAIRLINE,
    };
    ui.painter().rect(
        rect,
        WIDGET_RADIUS,
        color::INPUT,
        stroke,
        StrokeKind::Inside,
    );
    if let Some(image) = look.icon.and_then(|icon| gui::image(world, icon)) {
        let tint = if look.faded {
            Color32::from_white_alpha(77)
        } else {
            Color32::WHITE
        };
        ui.place(
            Rect::from_center_size(rect.center(), Vec2::splat(SLOT_ICON)),
            image.widget(Vec2::splat(SLOT_ICON)).tint(tint),
        );
    }
    if look.count > 1 {
        let corner = Rect::from_min_max(rect.center(), rect.max - Vec2::new(3.0, 0.0));
        ui.new_child(
            UiBuilder::new()
                .max_rect(corner)
                .layout(Layout::bottom_up(Align::Max)),
        )
        .label(
            RichText::new(look.count.to_string())
                .size(11.0)
                .color(color::TEXT),
        );
    }
    response
}

#[derive(Resource, Default)]
struct SlotVerdicts(Vec<SlotVerdictSource>);

#[derive(Resource, Default)]
struct SlotNotes(Vec<SlotNoteSource>);

#[derive(Resource, Default)]
struct SlotActions(Vec<SlotAction>);

struct CellData {
    slot: u32,
    filled: Option<Filled>,
}

struct Filled {
    stack: ItemStack,
    hints: Vec<(InputAction, String)>,
    notes: Vec<String>,
    verdict: Option<SlotVerdict>,
}

const BUILT_IN: [InputAction; 3] = [
    InputAction::DropItem,
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
    let (outline, faded, verdict) = match &filled.verdict {
        None => (None, false, None),
        Some(SlotVerdict::Wanted(note)) => (Some(color::WARN), false, Some(note.clone())),
        Some(SlotVerdict::Refused(reason)) => (None, true, Some(reason.clone())),
    };
    let look = SlotLook {
        icon: Some(&def.icon),
        count: filled.stack.count,
        outline,
        faded,
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
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
        act(world, cell.slot, filled.stack.item, &matched);
    }
    if map::drag_started(world, InputAction::CarryItem, &response) {
        egui::DragAndDrop::set_payload(ui.ctx(), CarriedSlot(cell.slot));
    }
    let lines: Vec<String> = std::iter::once(card::overview(def))
        .chain(filled.notes.iter().cloned())
        .chain(verdict)
        .collect();
    let hint = StyledLine::new(hint_pieces(&filled.hints));
    hud::tooltip(&response, world, &def.name, &lines, Some(&hint));
}

fn draw_carried(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(carried) = egui::DragAndDrop::payload::<CarriedSlot>(&gui) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let Some(stack) = session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .and_then(|inventory| inventory.slots.get(carried.0 as usize).copied())
    else {
        return;
    };
    let Some(image) = gui::image(world, &stack.item.get(&content).icon) else {
        return;
    };
    gui::at_pointer(
        &gui,
        Id::new("carried item"),
        Band::Pointer,
        -Vec2::splat(CARRIED / 2.0),
        |ui| ui.add(image.widget(Vec2::splat(CARRIED))),
    );
}

fn inventory_cells(world: &World) -> Vec<CellData> {
    let content = world.resource::<Content>();
    let inventory = session::my_character(world).and_then(|me| me.get::<Inventory>());
    let max = inventory.map_or(INVENTORY_MAX, |inventory| inventory.max);
    let verdicts = world
        .get_resource::<SlotVerdicts>()
        .map(|verdicts| verdicts.0.as_slice())
        .unwrap_or_default();
    let notes = world
        .get_resource::<SlotNotes>()
        .map(|notes| notes.0.as_slice())
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
                        hints: hints(map, &takeovers, def),
                        notes: bound_note(def)
                            .into_iter()
                            .chain(notes.iter().filter_map(|source| source(world, stack)))
                            .collect(),
                        verdict: verdicts.iter().find_map(|source| source(world, stack)),
                    }
                }),
        })
        .collect()
}

fn bound_note(def: &ItemDef) -> Option<String> {
    def.has(ItemFlag::Bound)
        .then(|| "bound · can't be sold or dropped".to_owned())
}

fn act(world: &mut World, slot: u32, item: ItemId, matched: &[InputAction]) {
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
            world.write_message(UseItemRequest { slot });
        }
        Some(InputAction::InspectItem) => card::open(world, item),
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

fn hints(map: &InputMap, takeovers: &[SlotAction], def: &ItemDef) -> Vec<(InputAction, String)> {
    let shadowed = |action: InputAction| {
        takeovers.iter().any(|takeover| {
            map.bindings(takeover.action)
                .iter()
                .any(|binding| map.bindings(action).contains(binding))
        })
    };
    let inspect = (!shadowed(InputAction::InspectItem))
        .then(|| (InputAction::InspectItem, " for more information".to_owned()));
    let usage = card::use_verb(def)
        .filter(|_| !shadowed(InputAction::UseItem))
        .map(|verb| (InputAction::UseItem, format!(" to {verb}")));
    takeovers
        .iter()
        .map(|takeover| (takeover.action, format!(" to {}", takeover.verb)))
        .chain(inspect)
        .chain(usage)
        .collect()
}

fn hint_pieces(hints: &[(InputAction, String)]) -> Vec<TextPiece> {
    hints
        .iter()
        .enumerate()
        .flat_map(|(index, (action, purpose))| {
            let separator = (index > 0).then(|| TextPiece::text(" · "));
            separator
                .into_iter()
                .chain([input(*action), TextPiece::text(purpose.clone())])
        })
        .collect()
}
