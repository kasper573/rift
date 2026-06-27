use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_ui::{ComputedNode, Node, UiGlobalTransform, Val};
use bevy_window::Window;

use crate::{Align, Side};

#[derive(Component, Clone, Copy, Default)]
#[require(Node)]
pub struct Placement {
    pub side: Side,
    pub align: Align,
    pub offset: f32,
}

#[derive(Component)]
pub(crate) struct Placed;

pub(crate) fn position_overlays(
    contents: Query<(Entity, &Placement, &ChildOf)>,
    placed: Query<(), With<Placed>>,
    measured: Query<(&ComputedNode, &UiGlobalTransform)>,
    windows: Query<&Window>,
    mut nodes: Query<&mut Node>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let viewport = window.size();
    for (entity, placement, child_of) in &contents {
        let anchor = child_of.parent();
        let (Ok((anchor_node, anchor_transform)), Ok((content_node, _))) =
            (measured.get(anchor), measured.get(entity))
        else {
            continue;
        };
        let anchor_size = anchor_node.size * anchor_node.inverse_scale_factor;
        let anchor_center = anchor_transform.translation * anchor_node.inverse_scale_factor;
        let anchor_pos = anchor_center - anchor_size / 2.0;
        let content_size = content_node.size * content_node.inverse_scale_factor;
        if content_size.x < 1.0 || content_size.y < 1.0 {
            if placed.contains(entity) {
                commands.entity(entity).remove::<Placed>();
            }
            continue;
        }
        let pos = place(
            anchor_pos,
            anchor_size,
            content_size,
            viewport,
            placement.side,
            placement.align,
            placement.offset,
        );
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.left = Val::Px(pos.x - anchor_pos.x);
            node.top = Val::Px(pos.y - anchor_pos.y);
        }
        if !placed.contains(entity) {
            commands.entity(entity).insert(Placed);
        }
    }
}

pub fn place(
    anchor_pos: Vec2,
    anchor_size: Vec2,
    content: Vec2,
    viewport: Vec2,
    side: Side,
    align: Align,
    offset: f32,
) -> Vec2 {
    let preferred = anchored(side, anchor_pos, anchor_size, content, align, offset);
    let pos = if overflows(side, preferred, content, viewport) {
        let opposite = opposite(side);
        let alternate = anchored(opposite, anchor_pos, anchor_size, content, align, offset);
        if overflows(opposite, alternate, content, viewport) {
            preferred
        } else {
            alternate
        }
    } else {
        preferred
    };
    Vec2::new(
        pos.x.clamp(0.0, (viewport.x - content.x).max(0.0)),
        pos.y.clamp(0.0, (viewport.y - content.y).max(0.0)),
    )
}

fn anchored(side: Side, pos: Vec2, size: Vec2, content: Vec2, align: Align, offset: f32) -> Vec2 {
    let cross = |start: f32, extent: f32, content: f32| match align {
        Align::Start => start,
        Align::Center => start + (extent - content) / 2.0,
        Align::End => start + extent - content,
    };
    match side {
        Side::Bottom => Vec2::new(cross(pos.x, size.x, content.x), pos.y + size.y + offset),
        Side::Top => Vec2::new(cross(pos.x, size.x, content.x), pos.y - content.y - offset),
        Side::Right => Vec2::new(pos.x + size.x + offset, cross(pos.y, size.y, content.y)),
        Side::Left => Vec2::new(pos.x - content.x - offset, cross(pos.y, size.y, content.y)),
    }
}

fn overflows(side: Side, pos: Vec2, content: Vec2, viewport: Vec2) -> bool {
    match side {
        Side::Bottom => pos.y + content.y > viewport.y,
        Side::Top => pos.y < 0.0,
        Side::Right => pos.x + content.x > viewport.x,
        Side::Left => pos.x < 0.0,
    }
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}
