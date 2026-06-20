use bevy_ui::{Node, Val};
use bevy_view::{View, node};

use crate::{
    Orientation,
    recipe::{Style, Styled},
    theme::color,
};

#[derive(Default)]
pub struct Separator {
    orientation: Orientation,
}

impl Separator {
    pub fn orientation(mut self, orientation: Orientation) -> Separator {
        self.orientation = orientation;
        self
    }
}

impl From<Separator> for View {
    fn from(separator: Separator) -> View {
        let (width, height) = match separator.orientation {
            Orientation::Horizontal => (Val::Percent(100.0), Val::Px(1.0)),
            Orientation::Vertical => (Val::Px(1.0), Val::Percent(100.0)),
        };
        let style = Style::new().background(color::surface_canvas_border_decorative);
        node()
            .attr(move |entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.width = width;
                    node.height = height;
                }
            })
            .style(style)
            .into()
    }
}
