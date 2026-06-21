use bevy_scene::{Scene, bsn, template_value};
use bevy_ui::{AlignItems, FlexDirection, JustifyContent, Node, Overflow, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::collapse::Collapse;
use crate::motion::transition::STANDARD_ENTER;
use crate::state::{SelectGroup, SelectItem, SelectTrigger};
use crate::style::{StatefulPaint, Style};
use crate::theme::theme;
use crate::tokens::spacing;

const OPEN: &str = "open";

pub fn collapsible(open: bool) -> impl Scene {
    bsn! {
        SelectGroup {
            exclusive: false,
            toggleable: true,
            initial: {open.then(|| OPEN.to_owned()).into_iter().collect::<Vec<_>>()},
        }
        template_value(Style::new().node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.row_gap = Val::Px(spacing::L);
        }))
    }
}

pub fn collapsible_trigger() -> impl Scene {
    bsn! {
        Button
        SelectItem { value: {OPEN.to_owned()} }
        SelectTrigger
        template_value(Style::new()
            .text_color(theme().surface_canvas.on)
            .background(
                StatefulPaint::new(bevy_color::Color::NONE).hover(theme().surface_canvas.hover),
            )
            .transition(STANDARD_ENTER)
            .node(|node| {
                node.width = Val::Percent(100.0);
                node.flex_direction = FlexDirection::Row;
                node.justify_content = JustifyContent::SpaceBetween;
                node.align_items = AlignItems::Center;
                node.padding = UiRect::axes(Val::Px(spacing::M), Val::Px(spacing::L));
            }))
    }
}

pub fn collapsible_content() -> impl Scene {
    bsn! {
        Node { overflow: {Overflow::clip()}, width: Val::Percent(100.0) }
        SelectItem { value: {OPEN.to_owned()} }
        Collapse
    }
}

pub fn collapsible_body() -> impl Scene {
    bsn! {
        template_value(Style::new()
            .text_color(theme().surface_canvas.on)
            .node(|node| {
                node.padding = UiRect::new(
                    Val::Px(spacing::M),
                    Val::Px(spacing::M),
                    Val::Px(spacing::S),
                    Val::Px(spacing::L),
                );
            }))
    }
}
