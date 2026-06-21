use bevy_scene::{Scene, bsn, template_value};
use bevy_ui::Val;

use crate::Orientation;
use crate::style::Style;
use crate::theme::theme;

pub fn separator(orientation: Orientation) -> impl Scene {
    let (width, height) = match orientation {
        Orientation::Horizontal => (Val::Percent(100.0), Val::Px(1.0)),
        Orientation::Vertical => (Val::Px(1.0), Val::Percent(100.0)),
    };
    bsn! {
        template_value(Style::new()
            .background(theme().surface_canvas.border)
            .node(move |node| {
                node.width = width;
                node.height = height;
            }))
    }
}
