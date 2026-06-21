use bevy_ecs::bundle::Bundle;
use bevy_ui::{Node, Val};

use crate::Orientation;
use crate::style::Style;
use crate::theme::theme;

pub fn separator(orientation: Orientation) -> impl Bundle {
    let (width, height) = match orientation {
        Orientation::Horizontal => (Val::Percent(100.0), Val::Px(1.0)),
        Orientation::Vertical => (Val::Px(1.0), Val::Percent(100.0)),
    };
    (
        Node::default(),
        Style::new()
            .background(theme().surface_canvas.border)
            .node(move |node| {
                node.width = width;
                node.height = height;
            }),
    )
}
