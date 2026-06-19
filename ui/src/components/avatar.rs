//! `Avatar`: a 40px round image placeholder or fallback. The root centers content; the image is
//! a source-driven element; the fallback displays when no image is set.

use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Val, prelude::ImageNode};
use bevy_view::{View, image, node};

use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Default)]
pub struct Avatar {
    children: Vec<View>,
}

children_builder!(Avatar);

/// The avatar image; renders nothing until a source is set.
#[derive(Default)]
pub struct AvatarImage {
    source: Option<ImageNode>,
}

impl AvatarImage {
    pub fn src(mut self, source: impl Into<ImageNode>) -> AvatarImage {
        self.source = Some(source.into());
        self
    }
}

/// Shown in place of the image (e.g. initials) when no source loads.
#[derive(Default)]
pub struct AvatarFallback {
    children: Vec<View>,
}

children_builder!(AvatarFallback);

impl From<Avatar> for View {
    fn from(avatar: Avatar) -> View {
        let style = avatar_style();
        node().style(style).children(avatar.children).into()
    }
}

impl From<AvatarImage> for View {
    fn from(avatar: AvatarImage) -> View {
        match avatar.source {
            Some(source) => image(source).into(),
            None => View::empty(),
        }
    }
}

impl From<AvatarFallback> for View {
    fn from(fallback: AvatarFallback) -> View {
        let style = fallback_style();
        node().style(style).children(fallback.children).into()
    }
}

fn avatar_style() -> Style {
    Style::new().background(color::secondary_base).node(|node| {
        node.width = Val::Px(size::STEP_1000);
        node.height = Val::Px(size::STEP_1000);
        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::Center;
        node.overflow = bevy_ui::Overflow::hidden();
    })
}

fn fallback_style() -> Style {
    Style::new().text_color(color::secondary_on).node(|node| {
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::Center;
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
    })
}
