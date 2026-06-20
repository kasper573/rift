//! `Text` ignores picking so captions don't swallow clicks.

use std::sync::Arc;

use bevy_ecs::prelude::World;
use bevy_picking::prelude::Pickable;
use bevy_text::{FontSource, FontWeight, TextFont};
use bevy_view::{View, dyn_text, text};

use crate::recipe::{Paint, Style, Styled};
use crate::theme::color;
use crate::tokens::typography::{self, Typography};

enum Content {
    Fixed(String),
    Live(Arc<dyn Fn(&World) -> String + Send + Sync>),
}

pub struct Text {
    content: Content,
    typography: Typography,
    color: Paint,
}

impl Text {
    pub fn new(content: impl Into<String>) -> Text {
        Text::with(Content::Fixed(content.into()))
    }

    pub fn dynamic<F>(content: F) -> Text
    where
        F: Fn(&World) -> String + Send + Sync + 'static,
    {
        Text::with(Content::Live(Arc::new(content)))
    }

    fn with(content: Content) -> Text {
        Text {
            content,
            typography: typography::BODY,
            color: color::surface_canvas_on.into(),
        }
    }

    pub fn intent(mut self, intent: &str) -> Text {
        self.typography = typography::by_name(intent);
        self
    }

    pub fn color(mut self, color: impl Into<Paint>) -> Text {
        self.color = color.into();
        self
    }
}

impl From<Text> for View {
    fn from(value: Text) -> View {
        let typography = value.typography;
        let element = match value.content {
            Content::Fixed(content) => text(content),
            Content::Live(content) => dyn_text(move |world| content(world)),
        };
        element
            .insert(TextFont {
                font: FontSource::Family(typography.family.into()),
                font_size: typography.font_size.into(),
                weight: FontWeight(typography.weight),
                ..Default::default()
            })
            .insert(Pickable::IGNORE)
            .style(Style::new().text_color(value.color))
            .into()
    }
}
