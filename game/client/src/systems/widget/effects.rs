//! The effects widget: an always-on row of icons, one per active effect command that declares an
//! icon. The icon and tooltip come from the effect itself (via the command), recomputed from the
//! player's active effects each frame.

use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, DragHandle, DragRoot, OnSettle, Side, tooltip, tooltip_content};
use world::systems::effect::{EffectContext, active_effects};
use world::systems::player::session;

use super::{reconcile_children, slot_node, tooltip_label};
use ui::component;

#[derive(Component, Default, Clone)]
struct EffectsGrid;

inventory::submit! {
    super::WidgetDef {
        id: "effects",
        fallback: Vec2::new(8.0, 80.0),
        build,
        sync: sync_effects,
    }
}

fn build(pos: Vec2, id: &'static str) -> Box<dyn Scene> {
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        min_width: Val::Px(super::WIDGET.0),
        min_height: Val::Px(super::WIDGET.0 / 2.0),
        padding: UiRect::all(Val::Px(2.0)),
        ..default()
    };
    Box::new(bsn! {
        template_value(node)
        BackgroundColor({super::PANEL_BG})
        DragRoot
        DragHandle
        EffectsGrid
        component(OnSettle::new(move |world, geom| super::persist_widget(world, id, geom)))
    })
}

struct IconData {
    icon: Handle<Image>,
    label: String,
    key: u64,
}

fn sync_effects(world: &mut World) {
    let icons = effect_icons(world);
    let mut grids = world.query_filtered::<Entity, With<EffectsGrid>>();
    let Some(grid) = grids.iter(world).next() else {
        return;
    };
    let keys: Vec<u64> = icons.iter().map(|icon| icon.key).collect();
    reconcile_children(world, grid, &keys, |index| icon_scene(&icons[index]));
}

fn effect_icons(world: &World) -> Vec<IconData> {
    let Some(me) = session::me(world) else {
        return Vec::new();
    };
    let me = me.id();
    let commands = active_effects(world, me);
    let ctx = EffectContext {
        world,
        source: me,
        target: me,
    };
    let assets = world.resource::<AssetServer>();
    commands
        .into_iter()
        .filter_map(|command| {
            let icon = command.icon()?;
            let label = command.describe(&ctx);
            Some(IconData {
                key: key(icon, &label),
                icon: assets.load(icon.to_owned()),
                label,
            })
        })
        .collect()
}

fn icon_scene(data: &IconData) -> Box<dyn Scene> {
    let label = data.label.clone();
    Box::new(bsn! {
        template_value(slot_node())
        {tooltip(false)}
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(data.icon.clone()))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(label))} ]
            ),
        ]
    })
}

fn key(icon: &str, label: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    icon.hash(&mut hasher);
    label.hash(&mut hasher);
    hasher.finish()
}
