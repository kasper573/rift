use bevy_ecs::bundle::Bundle;
use bevy_picking::prelude::Pickable;
use bevy_text::{FontSource, FontWeight, TextFont};
use bevy_ui::widget::Text;

use crate::style::{Paint, Style};
use crate::theme::color;
use crate::tokens::typography::{self, Typography};

pub fn text(content: impl Into<String>) -> impl Bundle {
    styled(content, color::surface_canvas.on.into())
}

pub fn text_colored(content: impl Into<String>, color: impl Into<Paint>) -> impl Bundle {
    styled(content, color.into())
}

fn styled(content: impl Into<String>, color: Paint) -> impl Bundle {
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
