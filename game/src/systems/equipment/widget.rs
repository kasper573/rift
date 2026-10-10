use crate::core::content::Content;
use crate::systems::equipment::{self, Equipment, EquipmentSlot};
use crate::systems::item::card;
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, RichPiece, Side, TooltipText, tooltip, tooltip_content};

use crate::systems::hud::{
    HudAudience, SLOT_BG, SLOT_BORDER, Window, reconcile_children, slot_node, tooltip_label,
};
use crate::systems::input::map::{ActionInput, InputAction, input};
use ui::component;

#[derive(Component, Default, Clone)]
pub(super) struct EquipmentGrid;

pub struct EquipmentWindow;

impl Window for EquipmentWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Players
    }
    fn title(&self) -> &'static str {
        "Equipment"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleEquipment
    }
    fn icon(&self) -> &'static str {
        "icons/equipment/helm.png"
    }
    fn order(&self) -> u32 {
        1
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        crate::systems::hud::single_tab(self.title(), ui::scrolled(content()))
    }
    fn sync(&self, world: &mut World) {
        sync_equipment(world)
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

struct CellData {
    slot: EquipmentSlot,
    item: Option<crate::data::item::Id>,
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
    reconcile_children(world, grid, &keys, |world, index| {
        cell_scene(world.resource::<Content>(), &cells[index])
    });
}

fn equipment_cells(world: &World) -> Vec<CellData> {
    let content = world.resource::<Content>();
    let equipment = session::my_character(world).and_then(|me| me.get::<Equipment>());
    let assets = world.resource::<AssetServer>();
    equipment::EquipmentSlot::all()
        .iter()
        .copied()
        .map(|slot| {
            let item = equipment.and_then(|equipment| equipment.slots.get(&slot).copied());
            CellData {
                slot,
                item,
                icon: item.map(|item| assets.load(item.get(content).icon.0)),
            }
        })
        .collect()
}

fn cell_key(index: usize, cell: &CellData) -> u64 {
    let content = cell.item.map_or(0, |item| item.index() as u64 + 1);
    ((index as u64) << 48) | content
}

fn cell_scene(content: &Content, cell: &CellData) -> Box<dyn Scene> {
    match &cell.icon {
        Some(icon) => Box::new(worn_slot(
            content,
            cell.slot,
            cell.item.expect("a worn slot holds an item"),
            icon.clone(),
        )),
        None => Box::new(empty_slot(cell.slot)),
    }
}

fn empty_slot(slot: EquipmentSlot) -> impl Scene {
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

fn worn_slot(
    content: &Content,
    slot: EquipmentSlot,
    item: crate::data::item::Id,
    icon: Handle<Image>,
) -> impl Scene {
    let def = item.get(content);
    let tip = TooltipText {
        title: def.display_name.to_owned(),
        lines: vec![vec![RichPiece::text(card::overview(def))]],
        hint: Some(
            card::inspect_hint()
                .into_iter()
                .chain([
                    RichPiece::text(" · "),
                    input(InputAction::UseItem),
                    RichPiece::text(" to take it off"),
                ])
                .collect(),
        ),
    };
    bsn! {
        template_value(slot_node())
        BackgroundColor({SLOT_BG})
        component(BorderColor::all(SLOT_BORDER))
        {tooltip(false)}
        ui::component(ui::CursorStyle::Pointer)
        on(move |click: On<Pointer<Click>>, input: ActionInput, mut commands: Commands| {
            if input.clicked(InputAction::UseItem, &click) {
                commands.queue(move |world: &mut World| session::unequip(world, slot));
            } else if input.clicked(InputAction::InspectItem, &click) {
                commands.queue(move |world: &mut World| card::open(world, item));
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
                Children [ {EntityScene(ui::tooltip_text(tip))} ]
            ),
        ]
    }
}
