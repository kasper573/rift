//! `collapse`: a height-animated disclosure. Unlike a gate (which mounts and unmounts), the body stays
//! mounted and is clipped; a system eases the wrapper's height between zero and the body's natural
//! height each frame, so opening and closing tween smoothly. The accordion and collapsible build on it.

use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_ui::{ComputedNode, Node, Overflow, Val};
use bevy_view::{Element, View, node};

// `content_size` is the wrapper's natural content height regardless of the height we clip it to, so it's
// the target the open height eases toward (measuring the child directly reads the clipped height).

/// Tracks a collapse wrapper's requested open state and its current animated height.
#[derive(Component, Default)]
pub(crate) struct Collapse {
    pub(crate) open: bool,
    pub(crate) height: f32,
}

/// Wraps `body` in a height-collapsing container driven by `is_open`, read off the world every render so
/// it follows the controlled open/selected state.
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

/// Eases each collapse wrapper's height toward its body's natural height (open) or zero (closed). Runs
/// after layout so the body's measured height is current; the written height settles on the next frame.
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
        // Apply a whole-pixel height: a fractional clip height makes the 1px dividers inside flicker.
        node.height = Val::Px(collapse.height.round());
    }
}
