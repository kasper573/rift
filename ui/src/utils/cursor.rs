use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_input::ButtonInput;
use bevy_input::mouse::MouseButton;
use bevy_picking::hover::HoverMap;
use bevy_picking::pointer::PointerId;
use bevy_ui::{ComputedNode, InteractionDisabled};

use crate::carry::Ghost;

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

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ClickThrough;

pub fn clicks_through(world: &World, hit: Entity) -> bool {
    std::iter::successors(Some(hit), |&entity| {
        world.get::<ChildOf>(entity).map(ChildOf::parent)
    })
    .find_map(|entity| {
        passes_at(
            world.get::<ClickThrough>(entity).is_some(),
            world.get::<CursorStyle>(entity).is_some(),
        )
    })
    .unwrap_or(false)
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
    nodes: Query<Has<ClickThrough>, With<ComputedNode>>,
    parents: Query<&ChildOf>,
    styles: Query<(&CursorStyle, Has<InteractionDisabled>)>,
    ghosts: Query<(), With<Ghost>>,
    mut cursor: ResMut<InterfaceCursor>,
) {
    let ancestry = |hit: Entity| {
        std::iter::successors(Some(hit), |&entity| {
            parents.get(entity).ok().map(ChildOf::parent)
        })
    };
    let topmost = hover.get(&PointerId::Mouse).and_then(|hits| {
        hits.iter()
            .filter(|&(&hit, _)| {
                !ancestry(hit)
                    .find_map(|entity| {
                        passes_at(nodes.get(entity).unwrap_or(false), styles.contains(entity))
                    })
                    .unwrap_or(false)
            })
            .filter_map(|(&hit, data)| {
                Some((
                    ancestry(hit).find(|&entity| nodes.contains(entity))?,
                    data.depth,
                ))
            })
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

fn passes_at(click_through: bool, styled: bool) -> Option<bool> {
    match (styled, click_through) {
        (true, _) => Some(false),
        (false, true) => Some(true),
        (false, false) => None,
    }
}
