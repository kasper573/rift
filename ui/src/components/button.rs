use bevy_ecs::bundle::Bundle;
use bevy_ecs::children;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::components::text::text_colored;
use crate::motion::transition::STANDARD_ENTER;
use crate::style::{Paint, Style};
use crate::theme::{ColorVar, color};
use crate::tokens::{radius, spacing};

pub fn button(label: impl Into<String>) -> impl Bundle {
    button_styled("primary", "md", label)
}

pub fn button_styled(variant: &str, size: &str, label: impl Into<String>) -> impl Bundle {
    let intent = selected_intent(variant);
    (
        Node::default(),
        Button,
        intent_style(intent, sized(size)),
        children![text_colored(label.into(), intent.on)],
    )
}

#[derive(Clone, Copy)]
enum Surface {
    Themed(ColorVar),
    Transparent,
}

impl From<Surface> for Paint {
    fn from(surface: Surface) -> Paint {
        match surface {
            Surface::Themed(var) => Paint::Var(var),
            Surface::Transparent => Paint::Literal(bevy_color::Color::NONE),
        }
    }
}

struct Intent {
    name: &'static str,
    base: Surface,
    hover: Surface,
    active: Surface,
    on: ColorVar,
    border: Option<ColorVar>,
}

use Surface::{Themed, Transparent};

const INTENTS: &[Intent] = &[
    Intent {
        name: "primary",
        base: Themed(color::primary_base),
        hover: Themed(color::primary_hover),
        active: Themed(color::primary_active),
        on: color::primary_on,
        border: None,
    },
    Intent {
        name: "secondary",
        base: Themed(color::secondary_base),
        hover: Themed(color::secondary_hover),
        active: Themed(color::secondary_active),
        on: color::secondary_on,
        border: None,
    },
    Intent {
        name: "muted",
        base: Themed(color::surface_inset_base),
        hover: Themed(color::surface_elevated_hover),
        active: Themed(color::surface_elevated_active),
        on: color::surface_canvas_on,
        border: None,
    },
    Intent {
        name: "danger",
        base: Themed(color::error_solid_base),
        hover: Themed(color::error_solid_hover),
        active: Themed(color::error_solid_active),
        on: color::error_solid_on,
        border: None,
    },
    Intent {
        name: "plain",
        base: Transparent,
        hover: Themed(color::secondary_hover),
        active: Themed(color::secondary_active),
        on: color::surface_canvas_on,
        border: None,
    },
];

fn intent_style(intent: &Intent, size: Style) -> Style {
    let mut style = Style::new()
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
            node.column_gap = Val::Px(spacing::L);
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
        .background(intent.base)
        .hover(Style::new().background(intent.hover))
        .active(Style::new().background(intent.active))
        .transition(STANDARD_ENTER)
        .merge(size);
    if let Some(border) = intent.border {
        style = style
            .node(|node| node.border = UiRect::all(Val::Px(1.0)))
            .border_color(border);
    }
    style
}

fn sized(size: &str) -> Style {
    if size == "icon" {
        return Style::new().node(|node| {
            node.width = Val::Px(16.0);
            node.height = Val::Px(16.0);
            node.padding = UiRect::ZERO;
        });
    }
    let (height, padding) = match size {
        "sm" => (32.0, spacing::L),
        "lg" => (48.0, spacing::XXL),
        _ => (40.0, spacing::XL),
    };
    Style::new().node(move |node| {
        node.height = Val::Px(height);
        node.padding = UiRect::horizontal(Val::Px(padding));
    })
}

fn selected_intent(name: &str) -> &'static Intent {
    INTENTS
        .iter()
        .find(|intent| intent.name == name)
        .unwrap_or(&INTENTS[0])
}
