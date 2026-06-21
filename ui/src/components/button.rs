use bevy_ecs::bundle::Bundle;
use bevy_ecs::children;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::components::text::text_colored;
use crate::motion::transition::STANDARD_ENTER;
use crate::style::{Paint, StatefulPaint, Style};
use crate::theme::{ColorVar, Family, color};
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

struct Surfaces {
    base: Surface,
    hover: Surface,
    active: Surface,
    on: ColorVar,
    border: Option<ColorVar>,
}

impl Surfaces {
    // The common case: base/hover/active/on come straight from one color family.
    const fn family(family: Family<ColorVar>) -> Surfaces {
        Surfaces {
            base: Surface::Themed(family.base),
            hover: Surface::Themed(family.hover),
            active: Surface::Themed(family.active),
            on: family.on,
            border: None,
        }
    }
}

impl ButtonIntent {
    fn surfaces(self) -> Surfaces {
        match self {
            ButtonIntent::Primary => Surfaces::family(color::primary),
            ButtonIntent::Secondary => Surfaces::family(color::secondary),
            ButtonIntent::Danger => Surfaces::family(color::error_solid),
            // muted and plain deliberately blend slots from several families.
            ButtonIntent::Muted => Surfaces {
                base: Surface::Themed(color::surface_inset.base),
                hover: Surface::Themed(color::surface_elevated.hover),
                active: Surface::Themed(color::surface_elevated.active),
                on: color::surface_canvas.on,
                border: None,
            },
            ButtonIntent::Plain => Surfaces {
                base: Surface::Transparent,
                hover: Surface::Themed(color::secondary.hover),
                active: Surface::Themed(color::secondary.active),
                on: color::surface_canvas.on,
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
