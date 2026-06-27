use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, Side, tooltip, tooltip_content};
use world::core::table::Id;
use world::systems::equipment::{self, Equipment, SlotId};
use world::systems::item::ItemDef;
use world::systems::player::session;

use super::{SLOT_BG, SLOT_BORDER, Window, reconcile_children, slot_node, tooltip_label};
use ui::component;

#[derive(Component, Default, Clone)]
pub(super) struct EquipmentGrid;

inventory::submit! {
    Window {
        id: "Equipment",
        title: "Equipment",
        toggle: KeyCode::KeyE,
        keybind: "E",
        icon: "icons/equipment/helm.png",
        order: 1,
        content,
        sync: sync_equipment,
    }
}

fn content() -> Box<dyn Scene> {
    Box::new(bsn! {
        Node {
            width: Val::Percent(100.0),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::FlexStart,
        }
        EquipmentGrid
    })
}

#[derive(Component, Default, Clone)]
struct Cell {
    slot: SlotId,
}

struct CellData {
    slot: SlotId,
    item: Option<Id<ItemDef>>,
    icon: Option<Handle<Image>>,
}

pub(super) fn sync_equipment(world: &mut World) {
    let cells = equipment_cells(world);
    let mut grids = world.query_filtered::<Entity, With<EquipmentGrid>>();
    let Some(grid) = grids.iter(world).next() else {
        return;
    };
    let keys: Vec<u64> = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| cell_key(index, cell))
        .collect();
    reconcile_children(world, grid, &keys, |index| cell_scene(&cells[index]));
}

fn equipment_cells(world: &World) -> Vec<CellData> {
    let equipment = session::me(world).and_then(|me| me.get::<Equipment>());
    let assets = world.resource::<AssetServer>();
    equipment::slot::all()
        .map(|slot| {
            let item = equipment.and_then(|equipment| equipment.slots.get(&slot).copied());
            CellData {
                slot,
                item,
                icon: item.map(|item| assets.load(item.get().icon.0.clone())),
            }
        })
        .collect()
}

fn cell_key(index: usize, cell: &CellData) -> u64 {
    let content = cell.item.map_or(0, |item| item.index() as u64 + 1);
    ((index as u64) << 48) | content
}

fn cell_scene(cell: &CellData) -> Box<dyn Scene> {
    match &cell.icon {
        Some(icon) => Box::new(worn_slot(
            cell.slot,
            cell.item.expect("a worn slot holds an item"),
            icon.clone(),
        )),
        None => Box::new(empty_slot(cell.slot)),
    }
}

fn empty_slot(slot: SlotId) -> impl Scene {
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
        {tooltip(false)}
        Children [
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(slot.label().to_owned()))} ]
            ),
        ]
    }
}

fn worn_slot(slot: SlotId, item: Id<ItemDef>, icon: Handle<Image>) -> impl Scene {
    let name = item.get().display_name.clone();
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
        {tooltip(false)}
        Cell { slot: {slot} }
        on(|click: On<Pointer<Click>>, cells: Query<&Cell>, mut commands: Commands| {
            if let Ok(cell) = cells.get(click.entity) {
                let slot = cell.slot;
                commands.queue(move |world: &mut World| session::unequip(world, slot));
            }
        })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(icon))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(name))} ]
            ),
        ]
    }
}
