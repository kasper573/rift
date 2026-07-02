use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, Side, text_colored, tooltip, tooltip_content};
use world::systems::item::{INVENTORY_MAX, Inventory};
use world::systems::player::session;

use super::{SLOT_BG, SLOT_BORDER, Window, reconcile_children, slot_node, tooltip_label};
use ui::component;

#[derive(Component, Default, Clone)]
pub(super) struct InventoryGrid;

pub struct InventoryWindow;

impl Window for InventoryWindow {
    fn title(&self) -> &'static str {
        "Inventory"
    }
    fn toggle(&self) -> KeyCode {
        KeyCode::KeyI
    }
    fn keybind(&self) -> &'static str {
        "I"
    }
    fn icon(&self) -> &'static str {
        "icons/equipment/bag.png"
    }
    fn order(&self) -> u32 {
        0
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        super::single_tab(self.title(), ui::scrolled(content()))
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

#[derive(Component, Default, Clone)]
struct Cell {
    slot: u32,
}

struct CellData {
    slot: u32,
    filled: Option<Filled>,
}

struct Filled {
    icon: Handle<Image>,
    name: String,
    kind: u64,
    count: u32,
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
    let inventory = session::me(world).and_then(|me| me.get::<Inventory>());
    let max = inventory.map_or(INVENTORY_MAX, |inventory| inventory.max);
    let assets = world.resource::<AssetServer>();
    (0..max)
        .map(|slot| CellData {
            slot,
            filled: inventory
                .and_then(|inventory| inventory.slots.get(slot as usize))
                .map(|stack| {
                    let def = stack.item.get();
                    Filled {
                        icon: assets.load(def.icon.0),
                        name: def.display_name.to_owned(),
                        kind: stack.item.index() as u64,
                        count: stack.count,
                    }
                }),
        })
        .collect()
}

fn cell_key(cell: &CellData) -> u64 {
    let content = match &cell.filled {
        None => 0,
        Some(filled) => 1 | (filled.kind << 1) | ((filled.count as u64) << 24),
    };
    ((cell.slot as u64) << 48) | content
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
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
        {tooltip(false)}
        Cell { slot: {slot} }
        on(|click: On<Pointer<Click>>, cells: Query<&Cell>, keys: Res<ButtonInput<KeyCode>>, mut commands: Commands| {
            if let Ok(cell) = cells.get(click.entity) {
                let slot = cell.slot;
                let drop = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
                commands.queue(move |world: &mut World| act(world, slot, drop));
            }
        })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(filled.icon.clone()))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(filled.name.clone()))} ]
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

fn act(world: &mut World, slot: u32, drop: bool) {
    if drop {
        session::drop_item(world, slot);
    } else {
        session::use_item(world, slot);
    }
}
