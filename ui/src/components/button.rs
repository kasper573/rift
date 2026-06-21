use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ecs::children;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::components::text::text_colored;
use crate::motion::transition::STANDARD_ENTER;
use crate::style::{StatefulPaint, Style};
use crate::theme::{Family, theme};
use crate::tokens::{radius, spacing};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonIntent {
    Primary,
    Secondary,
    Danger,
    Muted,
    Plain,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
    Icon,
}

pub fn button(label: impl Into<String>) -> impl Bundle {
    button_styled(ButtonIntent::Primary, ButtonSize::Md, label)
}

pub fn button_styled(
    intent: ButtonIntent,
    size: ButtonSize,
    label: impl Into<String>,
) -> impl Bundle {
    let surfaces = intent.surfaces();
    (
        Node::default(),
        Button,
        intent_style(&surfaces, sized(size)),
        children![text_colored(label.into(), surfaces.on)],
    )
}

struct Surfaces {
    base: Color,
    hover: Color,
    active: Color,
    on: Color,
    border: Option<Color>,
}

impl Surfaces {
    // The common case: base/hover/active/on come straight from one color family.
    fn family(family: Family) -> Surfaces {
        Surfaces {
            base: family.base,
            hover: family.hover,
            active: family.active,
            on: family.on,
            border: None,
        }
    }
}

impl ButtonIntent {
    fn surfaces(self) -> Surfaces {
        let theme = theme();
        match self {
            ButtonIntent::Primary => Surfaces::family(theme.primary),
            ButtonIntent::Secondary => Surfaces::family(theme.secondary),
            ButtonIntent::Danger => Surfaces::family(theme.error_solid),
            // muted and plain deliberately blend slots from several families.
            ButtonIntent::Muted => Surfaces {
                base: theme.surface_inset.base,
                hover: theme.surface_elevated.hover,
                active: theme.surface_elevated.active,
                on: theme.surface_canvas.on,
                border: None,
            },
            ButtonIntent::Plain => Surfaces {
                base: Color::NONE,
                hover: theme.secondary.hover,
                active: theme.secondary.active,
                on: theme.surface_canvas.on,
                border: None,
            },
        }
    }
}

fn intent_style(surfaces: &Surfaces, size: Style) -> Style {
    let mut style = Style::new()
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
            node.column_gap = Val::Px(spacing::L);
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
        .background(
            StatefulPaint::new(surfaces.base)
                .hover(surfaces.hover)
                .active(surfaces.active),
        )
        .transition(STANDARD_ENTER)
        .merge(size);
    if let Some(border) = surfaces.border {
        style = style
            .node(|node| node.border = UiRect::all(Val::Px(1.0)))
            .border_color(border);
    }
    style
}

fn sized(size: ButtonSize) -> Style {
    if size == ButtonSize::Icon {
        return Style::new().node(|node| {
            node.width = Val::Px(16.0);
            node.height = Val::Px(16.0);
            node.padding = UiRect::ZERO;
        });
    }
    let (height, padding) = match size {
        ButtonSize::Sm => (32.0, spacing::L),
        ButtonSize::Lg => (48.0, spacing::XXL),
        ButtonSize::Md | ButtonSize::Icon => (40.0, spacing::XL),
    };
    Style::new().node(move |node| {
        node.height = Val::Px(height);
        node.padding = UiRect::horizontal(Val::Px(padding));
    })
}
