use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_input::ButtonInput;
use bevy_input::mouse::MouseButton;
use bevy_picking::hover::HoverMap;
use bevy_picking::pointer::PointerId;
use bevy_ui::{ComputedNode, InteractionDisabled};

use crate::carry::Ghost;
use crate::state::ancestor_with;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Default,
    Pointer,
    Text,
    Resize,
    Grab,
    Grabbing,
}

#[derive(Resource, Default)]
pub struct InterfaceCursor {
    hovered: Option<CursorStyle>,
    held: Option<CursorStyle>,
}

impl InterfaceCursor {
    pub fn style(&self) -> Option<CursorStyle> {
        self.held.or(self.hovered)
    }

    pub fn captured(&self) -> bool {
        self.style().is_some()
    }
}

pub(crate) fn track_interface_cursor(
    hover: Res<HoverMap>,
    mouse: Res<ButtonInput<MouseButton>>,
    nodes: Query<(), With<ComputedNode>>,
    parents: Query<&ChildOf>,
    styles: Query<(&CursorStyle, Has<InteractionDisabled>)>,
    ghosts: Query<(), With<Ghost>>,
    mut cursor: ResMut<InterfaceCursor>,
) {
    let topmost = hover.get(&PointerId::Mouse).and_then(|hits| {
        hits.iter()
            .filter_map(|(&hit, data)| Some((ancestor_with(hit, &parents, &nodes)?, data.depth)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(node, _)| node)
    });
    cursor.hovered = topmost.map(|node| {
        std::iter::successors(Some(node), |&entity| {
            parents.get(entity).ok().map(ChildOf::parent)
        })
        .find_map(|entity| styles.get(entity).ok())
        .filter(|(_, disabled)| !disabled)
        .map_or(CursorStyle::Default, |(style, _)| *style)
    });
    if mouse.get_pressed().next().is_none() {
        cursor.held = None;
    } else if mouse.get_just_pressed().next().is_some() && cursor.held.is_none() {
        cursor.held = cursor.hovered.map(|style| match style {
            CursorStyle::Grab => CursorStyle::Grabbing,
            style => style,
        });
    }
    if cursor.held.is_some() && !ghosts.is_empty() {
        cursor.held = Some(CursorStyle::Grabbing);
    }
}
