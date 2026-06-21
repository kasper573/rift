use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn, template_value};
use bevy_text::{FontSource, FontWeight, TextFont};
use bevy_ui::widget::Text;

use bevy_color::Color;

use crate::component;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::typography::{self, Typography};

pub fn text(content: impl Into<String>) -> impl Scene {
    styled(content, theme().surface_canvas.on)
}

pub fn text_colored(content: impl Into<String>, color: Color) -> impl Scene {
    styled(content, color)
}

fn styled(content: impl Into<String>, color: Color) -> impl Scene {
    let style = Style::new().text_color(color);
    bsn! {
        Text({content.into()})
        component(font(typography::BODY))
        Pickable { should_block_lower: false, is_hoverable: false }
        template_value(style)
    }
}

fn font(typography: Typography) -> TextFont {
    TextFont {
        font: FontSource::Family(typography.family.into()),
        font_size: typography.font_size.into(),
        weight: FontWeight(typography.weight),
        ..Default::default()
    }
}
