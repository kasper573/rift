//! `Text`: the library's typographic primitive and the one way text is rendered. Pick a role with
//! `intent` (the type scale — `body`, `label`, `title`, `headline_*`, …) and
//! the font size, line height and weight come from the [typography tokens](crate::tokens::typography);
//! `color` sets the foreground from a [theme variable](crate::theme) or a literal color. Content is
//! either fixed ([`new`](Text::new)) or read off the world every render ([`dynamic`](Text::dynamic)).
//! It wraps the bare [`bevy_view::text`]/[`bevy_view::dyn_text`] primitives and ignores picking, so a
//! label never swallows a click meant for the control it captions. By convention every piece of text in
//! the library — and apps built on it — is a `Text`, leaving the lowercase primitives used only here.

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

    /// Content read off the world on every render — for values that change, like a live counter.
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

    /// Selects the type scale by name (`body`, `label`, `title`, …), falling back to
    /// `body` for an unknown name.
    pub fn intent(mut self, intent: &str) -> Text {
        self.typography = typography::by_name(intent);
        self
    }

    /// Overrides the foreground color — a [`ColorVar`](crate::theme::ColorVar) or a literal `Color`
    /// (defaults to on-surface text).
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
