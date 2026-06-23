//! The inventory pane: a grid of item slots reconciled against the player's replicated inventory,
//! each slot showing the item icon, a name tooltip, and using the item on click.

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, Side, text_colored, tooltip, tooltip_content};
use world::systems::items::Inventory;
use world::systems::player::session;

use crate::component;
use crate::hud::settings::ScreenPx;

const SLOT: ScreenPx = ScreenPx(36.0);
const TITLE_BG: Color = Color::srgb(0.18, 0.18, 0.18);
const TOOLTIP_BG: Color = Color::BLACK;

#[derive(Component, Default, Clone)]
pub(super) struct InventoryGrid;

#[derive(Component, Default, Clone)]
struct Cell {
    kind: u64,
    slot: u32,
}

/// A reconciled child's identity within its list (see `reconcile_children`).
#[derive(Component)]
struct Keyed(u64);

struct CellData {
    icon: Handle<Image>,
    name: String,
    kind: u64,
    slot: u32,
}

pub(super) fn sync_inventory(world: &mut World) {
    let cells = inventory_cells(world);
    let mut grids = world.query_filtered::<Entity, With<InventoryGrid>>();
    let Some(grid) = grids.iter(world).next() else {
        return;
    };
    let keys: Vec<u64> = cells
        .iter()
        .map(|cell| (cell.slot as u64) << 32 | cell.kind)
        .collect();
    reconcile_children(world, grid, &keys, |index| {
        Box::new(slot(&cells[index])) as Box<dyn Scene>
    });
}

/// Keeps `container`'s keyed children equal to `keys`: when the live keys differ (in value or order)
/// the keyed children are despawned and rebuilt from `build`, in order. The rendered list is re-derived
/// from `keys` whenever they change, so it can't go stale, duplicate, or fall out of order — dynamic
/// lists stay correct here instead of via a hand-written diff at each call site.
fn reconcile_children(
    world: &mut World,
    container: Entity,
    keys: &[u64],
    build: impl Fn(usize) -> Box<dyn Scene>,
) {
    let current: Vec<(Entity, u64)> = world
        .get::<Children>(container)
        .map(|children| {
            children
                .iter()
                .filter_map(|child| world.get::<Keyed>(child).map(|keyed| (child, keyed.0)))
                .collect()
        })
        .unwrap_or_default();
    if current.iter().map(|(_, key)| *key).eq(keys.iter().copied()) {
        return;
    }
    for (entity, _) in current {
        world.entity_mut(entity).despawn();
    }
    for (index, &key) in keys.iter().enumerate() {
        if let Ok(mut spawned) = world.spawn_scene(build(index)) {
            spawned.insert(Keyed(key));
            let child = spawned.id();
            world.entity_mut(container).add_child(child);
        }
    }
}

fn inventory_cells(world: &World) -> Vec<CellData> {
    let items = session::me(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or_else(Vec::new, |inventory| inventory.items.clone());
    let assets = world.resource::<AssetServer>();
    items
        .iter()
        .enumerate()
        .map(|(slot, item)| {
            let def = item.get();
            CellData {
                icon: assets.load(def.icon.0.clone()),
                name: def.display_name.clone(),
                kind: item.index() as u64,
                slot: slot as u32,
            }
        })
        .collect()
}

fn slot(cell: &CellData) -> impl Scene {
    bsn! {
        Node {
            width: Val::Px({SLOT.0}),
            height: Val::Px({SLOT.0}),
            margin: {UiRect::all(Val::Px(1.0))},
        }
        BackgroundColor({TITLE_BG})
        {tooltip(false)}
        Cell { kind: {cell.kind}, slot: {cell.slot} }
        on(|click: On<Pointer<Click>>, cells: Query<&Cell>, mut commands: Commands| {
            if let Ok(cell) = cells.get(click.entity) {
                let slot = cell.slot;
                commands.queue(move |world: &mut World| session::use_item(world, slot));
            }
        })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(cell.icon.clone()))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(cell.name.clone()))} ]
            ),
        ]
    }
}

fn tooltip_label(text: impl Into<String>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::axes(Val::Px(6.0), Val::Px(3.0))} }
        BackgroundColor({TOOLTIP_BG})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text.into(), Color::WHITE))} ]
    }
}
