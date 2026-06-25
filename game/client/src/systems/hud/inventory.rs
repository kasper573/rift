//! The inventory pane: a fixed grid of `max` slots reconciled against the player's replicated
//! inventory. Every slot draws a subtle cell so the grid shows through behind the icons; a filled
//! slot adds the item icon, its stack count, a name tooltip, and uses (or Ctrl-drops) the item on
//! click.

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, Side, text_colored, tooltip, tooltip_content};
use world::systems::items::{INVENTORY_MAX, Inventory};
use world::systems::player::session;

use crate::systems::hud::settings::ScreenPx;
use ui::component;

const SLOT: ScreenPx = ScreenPx(36.0);
const SLOT_BG: Color = Color::srgb(0.14, 0.14, 0.14);
const SLOT_BORDER: Color = Color::srgb(0.24, 0.24, 0.24);
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
                        icon: assets.load(def.icon.0.clone()),
                        name: def.display_name.clone(),
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
        template_value(cell_node())
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
        template_value(cell_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
        {tooltip(false)}
        Cell { kind: {filled.kind}, slot: {slot} }
        on(|click: On<Pointer<Click>>, cells: Query<&Cell>, keys: Res<ButtonInput<KeyCode>>, mut commands: Commands| {
            if let Ok(cell) = cells.get(click.entity) {
                let slot = cell.slot;
                let drop = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
                commands.queue(move |world: &mut World| {
                    if drop {
                        session::drop_item(world, slot);
                    } else {
                        session::use_item(world, slot);
                    }
                });
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

fn cell_node() -> Node {
    Node {
        width: Val::Px(SLOT.0),
        height: Val::Px(SLOT.0),
        margin: UiRect::all(Val::Px(1.0)),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
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
