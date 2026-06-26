//! The effects widget: an always-on row of icons, one per active effect command that declares an
//! icon. The icon and tooltip come from the effect itself (via the command), recomputed from the
//! player's active effects each frame.

use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Align, Side, tooltip, tooltip_content};
use world::systems::effect::{EffectContext, active_effects};
use world::systems::player::session;

use super::{reconcile_children, slot_node, tooltip_label};
use ui::component;

#[derive(Component, Default, Clone)]
pub(super) struct EffectsGrid;

struct IconData {
    icon: Handle<Image>,
    label: String,
    key: u64,
}

pub(super) fn sync_effects(world: &mut World) {
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
