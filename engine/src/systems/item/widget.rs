use bevy::prelude::*;
use ui::drag;
use ui::egui;
use ui::egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::face::TextRole;
use ui::{controls, ink, sheet};

use crate::core::assets::ImageRef;
use crate::core::content::Content;
use crate::core::gui::ImageSource;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::hud::{self, Carried, HudWindow};
use crate::systems::input::map::{self, InputAction, InputMap, input};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::{
    DropItemRequest, INVENTORY_MAX, Inventory, ItemDef, ItemId, ItemModuleKind, ItemStack,
    UseItemRequest, card,
};
use crate::systems::player::session;

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
        sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
            ui.horizontal_wrapped(|ui| {
                for cell in &cells {
                    bag_slot(ui, world, cell);
                }
            });
            ui.add_space(6.0);
            let hint = StyledLine::new(hint_pieces(&[
                (InputAction::InspectItem, " for more information".to_owned()),
                (InputAction::UseItem, " to use or wear".to_owned()),
                (InputAction::CarryItem, " to carry".to_owned()),
            ]));
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, &hint, look, None);
        });
    }
}

pub(crate) struct SlotLook<'a> {
    pub icon: Option<&'a ImageRef>,
    pub count: u32,
    pub wanted: bool,
    pub faded: bool,
    pub quest: bool,
}

impl SlotLook<'_> {
    pub(crate) const EMPTY: SlotLook<'static> = SlotLook {
        icon: None,
        count: 0,
        wanted: false,
        faded: false,
        quest: false,
    };
}

/// `icon` in a slot as the bag draws one: how an item's icon looks to the player.
pub fn shown_slot(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    icon: &ImageRef,
) -> Response {
    slot(
        ui,
        images,
        SlotLook {
            icon: Some(icon),
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
            refused: false,
        },
    );
    if look.wanted {
        ui.painter()
            .rect_stroke(rect, 3.0, Stroke::new(1.5, ink::OCHRE), StrokeKind::Inside);
    }
    if let Some(image) = look.icon.and_then(|icon| images.image(icon)) {
        let tint = if look.faded {
            Color32::from_white_alpha((GHOST * 255.0) as u8)
        } else {
            Color32::WHITE
        };
        ui.place(
            Rect::from_center_size(rect.center(), Vec2::splat(controls::SLOT_ICON)),
            image.widget(Vec2::splat(controls::SLOT_ICON)).tint(tint),
        );
    }
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
    let (wanted, refused, verdict) = match &filled.verdict {
        None => (false, false, None),
        Some(SlotVerdict::Wanted(note)) => (true, false, Some(note.clone())),
        Some(SlotVerdict::Refused(reason)) => (false, true, Some(reason.clone())),
    };
    let carried = drag::carried::<Carried>(ui.ctx()) == Some(Carried::BagSlot(cell.slot));
    let look = SlotLook {
        icon: Some(&def.icon),
        count: filled.stack.count,
        wanted,
        faded: refused || carried,
        quest: def.has(ItemModuleKind::QuestItem),
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
        act(world, cell.slot, filled.stack.item, &matched, response.rect);
    }
    if map::drag_started(world, InputAction::CarryItem, &response) {
        drag::carry(ui.ctx(), Carried::BagSlot(cell.slot));
    }
    let lines: Vec<String> = std::iter::once(card::overview(def))
        .chain(filled.notes.iter().cloned())
        .chain(verdict)
        .collect();
    let hint = StyledLine::new(hint_pieces(&filled.hints));
    hud::tooltip(&response, world, &def.name, &lines, Some(&hint));
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
            world.write_message(UseItemRequest { slot });
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
