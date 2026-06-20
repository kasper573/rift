use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_ui::{ComputedNode, Node, Overflow, Val};
use bevy_view::{Element, View, node};

#[derive(Component, Default)]
pub(crate) struct Collapse {
    pub(crate) open: bool,
    pub(crate) height: f32,
}

pub(crate) fn collapse<F>(is_open: F, body: Vec<View>) -> Element
where
    F: Fn(&World, Entity) -> bool + Send + Sync + 'static,
{
    node()
        .attr(move |entity| {
            let id = entity.id();
            let open = entity.world_scope(|world| is_open(world, id));
            if let Some(mut collapse) = entity.get_mut::<Collapse>() {
                collapse.open = open;
            } else {
                entity.insert(Collapse { open, height: 0.0 });
            }
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.overflow = Overflow::clip();
                node.width = Val::Percent(100.0);
            }
        })
        .children(body)
}

/// Ease collapse heights toward target (natural when open, zero when closed). Runs after layout.
pub(crate) fn advance_collapse(
    time: Res<Time>,
    mut collapses: Query<(&mut Collapse, &mut Node, &ComputedNode)>,
) {
    let step = (time.delta_secs() * 9.0).min(1.0);
    for (mut collapse, mut node, computed) in &mut collapses {
        let natural = computed.content_size.y * computed.inverse_scale_factor;
        let target = if collapse.open { natural } else { 0.0 };
        collapse.height += (target - collapse.height) * step;
        if (collapse.height - target).abs() < 0.5 {
            collapse.height = target;
        }
        // Whole-pixel height: fractional clip makes 1px dividers flicker.
        node.height = Val::Px(collapse.height.round());
    }
}
