use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_ui::{Checked, ComputedNode, Node, Val};

#[derive(Component, Default)]
#[require(Node)]
pub(crate) struct Collapse {
    pub(crate) height: f32,
}

pub(crate) fn advance_collapse(
    time: Res<Time>,
    mut collapses: Query<(&mut Collapse, &mut Node, &ComputedNode, Has<Checked>)>,
) {
    let step = (time.delta_secs() * 9.0).min(1.0);
    for (mut collapse, mut node, computed, open) in &mut collapses {
        let natural = computed.content_size.y * computed.inverse_scale_factor;
        let target = if open { natural } else { 0.0 };
        collapse.height += (target - collapse.height) * step;
        if (collapse.height - target).abs() < 0.5 {
            collapse.height = target;
        }
        // Fractional clip heights make 1px dividers flicker.
        node.height = Val::Px(collapse.height.round());
    }
}
