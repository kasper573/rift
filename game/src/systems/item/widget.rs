use crate::data::item::Id as ItemId;
use crate::systems::item::{INVENTORY_MAX, Inventory, ItemDef, ItemFlag, ItemStack, card};
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::scene::EntityScene;
use std::hash::{Hash, Hasher};
use ui::tokens::palette;
use ui::{Align, Carriable, Side, TooltipText, text_colored, tooltip, tooltip_content};

use crate::systems::hud::{
    HudAudience, SLOT_BG, SLOT_BORDER, Window, reconcile_children, slot_node,
};
use crate::systems::input::map::{ActionInput, InputAction, InputMap, input};
use ui::{RichPiece, component};

#[derive(Component, Default, Clone)]
pub(super) struct InventoryGrid;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SlotVerdict {
    Wanted(String),
    Refused(String),
}

pub type SlotVerdictSource = fn(&World, ItemStack) -> Option<SlotVerdict>;

#[derive(Resource, Default)]
struct SlotVerdicts(Vec<SlotVerdictSource>);

pub fn slot_verdict_source(app: &mut App, source: SlotVerdictSource) {
    app.world_mut()
        .get_resource_or_init::<SlotVerdicts>()
        .0
        .push(source);
}

pub type SlotNoteSource = fn(&World, ItemStack) -> Option<String>;

#[derive(Resource, Default)]
struct SlotNotes(Vec<SlotNoteSource>);

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

#[derive(Resource, Default)]
struct SlotActions(Vec<SlotAction>);

pub fn slot_action(app: &mut App, slot_action: SlotAction) {
    app.world_mut()
        .get_resource_or_init::<SlotActions>()
        .0
        .push(slot_action);
}

pub struct InventoryWindow;

impl Window for InventoryWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Players
    }
    fn title(&self) -> &'static str {
        "Inventory"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleInventory
    }
    fn icon(&self) -> &'static str {
        "icons/equipment/bag.png"
    }
    fn order(&self) -> u32 {
        0
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        crate::systems::hud::single_tab(self.title(), ui::scrolled(content()))
    }
    fn sync(&self, world: &mut World) {
        sync_inventory(world)
    }
}

fn content() -> Box<dyn Scene> {
    Box::new(bsn! {
        Node {
            width: Val::Percent(100.0),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::FlexStart,
        }
        InventoryGrid
    })
}

struct CellData {
    slot: u32,
    filled: Option<Filled>,
}

struct Filled {
    item: ItemId,
    hints: Vec<(InputAction, String)>,
    icon: Handle<Image>,
    count: u32,
    notes: Vec<String>,
    verdict: Option<SlotVerdict>,
}

pub(super) fn sync_inventory(world: &mut World) {
    let cells = inventory_cells(world);
    let mut grids = world.query_filtered::<Entity, With<InventoryGrid>>();
    let Some(grid) = grids.iter(world).next() else {
        return;
    };
    let keys: Vec<u64> = cells.iter().map(cell_key).collect();
    reconcile_children(world, grid, &keys, |index| slot(&cells[index]));
}

fn inventory_cells(world: &World) -> Vec<CellData> {
    let inventory = session::my_character(world).and_then(|me| me.get::<Inventory>());
    let max = inventory.map_or(INVENTORY_MAX, |inventory| inventory.max);
    let assets = world.resource::<AssetServer>();
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
                    let def = stack.item.get();
                    Filled {
                        item: stack.item,
                        hints: hints(map, &takeovers, def),
                        icon: assets.load(def.icon.0),
                        count: stack.count,
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

fn cell_key(cell: &CellData) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    cell.slot.hash(&mut hasher);
    if let Some(filled) = &cell.filled {
        (
            filled.item,
            &filled.hints,
            filled.count,
            &filled.notes,
            &filled.verdict,
        )
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn slot(cell: &CellData) -> Box<dyn Scene> {
    match &cell.filled {
        Some(filled) => Box::new(filled_slot(cell.slot, filled)),
        None => Box::new(empty_slot()),
    }
}

fn empty_slot() -> impl Scene {
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
    }
}

fn filled_slot(slot: u32, filled: &Filled) -> impl Scene {
    let count = if filled.count > 1 {
        filled.count.to_string()
    } else {
        String::new()
    };
    let def = filled.item.get();
    let (border, tint, verdict) = match &filled.verdict {
        None => (SLOT_BORDER, Color::WHITE, None),
        Some(SlotVerdict::Wanted(note)) => (palette::AMBER_70, Color::WHITE, Some(note.clone())),
        Some(SlotVerdict::Refused(reason)) => (
            SLOT_BORDER,
            Color::WHITE.with_alpha(0.3),
            Some(reason.clone()),
        ),
    };
    let tip = TooltipText {
        title: def.display_name.to_owned(),
        lines: std::iter::once(card::overview(def))
            .chain(filled.notes.iter().cloned())
            .chain(verdict)
            .map(|line| vec![RichPiece::text(line)])
            .collect(),
        hint: Some(hint_pieces(&filled.hints)),
    };
    let item = filled.item;
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(border))
        {tooltip(false)}
        component(Carriable { image: filled.icon.clone(), payload: slot as u64, input: InputAction::CarryItem.into() })
        on(move |click: On<Pointer<Click>>, input: ActionInput, actions: Option<Res<SlotActions>>, mut commands: Commands| {
            let candidates = actions
                .iter()
                .flat_map(|actions| actions.0.iter().map(|action| action.action))
                .chain(BUILT_IN);
            let matched: Vec<InputAction> = candidates.filter(|action| input.clicked(*action, &click)).collect();
            if !matched.is_empty() {
                commands.queue(move |world: &mut World| act(world, slot, item, &matched));
            }
        })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(filled.icon.clone()).with_color(tint))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(ui::tooltip_text(tip))} ]
            ),
            (
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(2.0),
                    bottom: Val::Px(0.0),
                }
                Pickable { should_block_lower: false, is_hoverable: false }
                Children [ {EntityScene(text_colored(count, Color::WHITE))} ]
            ),
        ]
    }
}

const BUILT_IN: [InputAction; 3] = [
    InputAction::DropItem,
    InputAction::UseItem,
    InputAction::InspectItem,
];

fn act(world: &mut World, slot: u32, item: ItemId, matched: &[InputAction]) {
    let takeover = active_slot_actions(world)
        .into_iter()
        .find(|takeover| matched.contains(&takeover.action));
    if let Some(takeover) = takeover {
        (takeover.act)(world, slot);
        return;
    }
    match BUILT_IN.into_iter().find(|action| matched.contains(action)) {
        Some(InputAction::DropItem) => session::drop_item(world, slot),
        Some(InputAction::UseItem) => session::use_item(world, slot),
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

fn hint_pieces(hints: &[(InputAction, String)]) -> Vec<RichPiece> {
    hints
        .iter()
        .enumerate()
        .flat_map(|(index, (action, purpose))| {
            let separator = (index > 0).then(|| RichPiece::text(" · "));
            separator
                .into_iter()
                .chain([input(*action), RichPiece::text(purpose.clone())])
        })
        .collect()
}
