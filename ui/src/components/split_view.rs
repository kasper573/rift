use bevy_color::Color;
use bevy_ecs::hierarchy::Children;
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::{BorderColor, FlexDirection, Node, UiRect, Val};

use crate::component;
use crate::components::scroll_area::scrolled;
use crate::components::text::styled_text;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{spacing, typography};

const LIST_SHARE: f32 = 42.0;

pub fn split_view(list: Box<dyn Scene>, detail: Box<dyn Scene>) -> impl Scene {
    let divider = BorderColor::all(theme().surface_floating.border);
    bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), flex_direction: FlexDirection::Row }
        Children [
            (
                Node {
                    width: Val::Percent({LIST_SHARE}),
                    height: Val::Percent(100.0),
                    border: {UiRect::right(Val::Px(1.0))},
                }
                component(divider)
                Children [ {EntityScene(scrolled(list))} ]
            ),
            (
                Node { flex_grow: 1.0, height: Val::Percent(100.0) }
                Children [ {EntityScene(scrolled(detail))} ]
            ),
        ]
    }
}

pub fn list_header(title: impl Into<String>, aside: impl Into<String>) -> impl Scene {
    let family = theme().surface_inset;
    let style = Style::new().background(Color::NONE).node(|node| {
        node.width = Val::Percent(100.0);
        node.justify_content = bevy_ui::JustifyContent::SpaceBetween;
        node.padding = UiRect::new(
            Val::Px(spacing::L),
            Val::Px(spacing::L),
            Val::Px(spacing::L),
            Val::Px(spacing::S),
        );
    });
    bsn! {
        template_value(style)
        Children [
            {EntityScene(styled_text(title.into(), family.on, typography::LABEL))},
            {EntityScene(styled_text(aside.into(), family.on, typography::CAPTION))},
        ]
    }
}
