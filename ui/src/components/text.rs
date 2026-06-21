use bevy_ecs::bundle::Bundle;
use bevy_picking::prelude::Pickable;
use bevy_text::{FontSource, FontWeight, TextFont};
use bevy_ui::widget::Text;

use bevy_color::Color;

use crate::style::Style;
use crate::theme::theme;
use crate::tokens::typography::{self, Typography};

pub fn text(content: impl Into<String>) -> impl Bundle {
    styled(content, theme().surface_canvas.on)
}

pub fn text_colored(content: impl Into<String>, color: Color) -> impl Bundle {
    styled(content, color)
}

fn styled(content: impl Into<String>, color: Color) -> impl Bundle {
    (
        Text::new(content.into()),
        font(typography::BODY),
        Pickable::IGNORE,
        Style::new().text_color(color),
    )
}

fn font(typography: Typography) -> TextFont {
    TextFont {
        font: FontSource::Family(typography.family.into()),
        font_size: typography.font_size.into(),
        weight: FontWeight(typography.weight),
        ..Default::default()
    }
}
