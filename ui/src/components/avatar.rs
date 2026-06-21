use bevy_scene::{Scene, bsn, template_value};
use bevy_ui::widget::ImageNode;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Overflow, Val};

use crate::component;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{radius, size};

pub fn avatar() -> impl Scene {
    bsn! {
        template_value(Style::new()
            .background(theme().secondary.base)
            .node(|node| {
                node.width = Val::Px(size::STEP_1000);
                node.height = Val::Px(size::STEP_1000);
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
                node.align_items = AlignItems::Center;
                node.justify_content = JustifyContent::Center;
                node.overflow = Overflow::hidden();
            }))
    }
}

pub fn avatar_image(source: impl Into<ImageNode>) -> impl Scene {
    bsn! {
        component(source.into())
    }
}

pub fn avatar_fallback() -> impl Scene {
    bsn! {
        template_value(Style::new().text_color(theme().secondary.on).node(|node| {
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
            node.width = Val::Percent(100.0);
            node.height = Val::Percent(100.0);
        }))
    }
}
