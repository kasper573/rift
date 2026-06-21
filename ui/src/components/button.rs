use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ecs::children;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::components::text::text_colored;
use crate::motion::transition::STANDARD_ENTER;
use crate::style::{StatefulPaint, Style};
use crate::theme::{Theme, theme};
use crate::tokens::{radius, spacing};

#[derive(Clone, Copy)]
pub struct ButtonIntent {
    base: fn(&Theme) -> Color,
    hover: fn(&Theme) -> Color,
    active: fn(&Theme) -> Color,
    on: fn(&Theme) -> Color,
}

pub mod intent {
    use bevy_color::Color;

    use super::ButtonIntent;
    use crate::theme::Theme;

    pub const PRIMARY: ButtonIntent = ButtonIntent {
        base: |t: &Theme| t.primary.base,
        hover: |t: &Theme| t.primary.hover,
        active: |t: &Theme| t.primary.active,
        on: |t: &Theme| t.primary.on,
    };

    pub const SECONDARY: ButtonIntent = ButtonIntent {
        base: |t: &Theme| t.secondary.base,
        hover: |t: &Theme| t.secondary.hover,
        active: |t: &Theme| t.secondary.active,
        on: |t: &Theme| t.secondary.on,
    };

    pub const DANGER: ButtonIntent = ButtonIntent {
        base: |t: &Theme| t.error_solid.base,
        hover: |t: &Theme| t.error_solid.hover,
        active: |t: &Theme| t.error_solid.active,
        on: |t: &Theme| t.error_solid.on,
    };

    // muted and plain deliberately blend slots from several families.
    pub const MUTED: ButtonIntent = ButtonIntent {
        base: |t: &Theme| t.surface_inset.base,
        hover: |t: &Theme| t.surface_elevated.hover,
        active: |t: &Theme| t.surface_elevated.active,
        on: |t: &Theme| t.surface_canvas.on,
    };

    pub const PLAIN: ButtonIntent = ButtonIntent {
        base: |_: &Theme| Color::NONE,
        hover: |t: &Theme| t.secondary.hover,
        active: |t: &Theme| t.secondary.active,
        on: |t: &Theme| t.surface_canvas.on,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
    Icon,
}

pub fn button(label: impl Into<String>) -> impl Bundle {
    button_styled(intent::PRIMARY, ButtonSize::Md, label)
}

pub fn button_styled(
    intent: ButtonIntent,
    size: ButtonSize,
    label: impl Into<String>,
) -> impl Bundle {
    let theme = theme();
    (
        Node::default(),
        Button,
        intent_style(intent, &theme).merge(sized(size)),
        children![text_colored(label.into(), (intent.on)(&theme))],
    )
}

fn intent_style(intent: ButtonIntent, theme: &Theme) -> Style {
    Style::new()
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
            node.column_gap = Val::Px(spacing::L);
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
        .background(
            StatefulPaint::new((intent.base)(theme))
                .hover((intent.hover)(theme))
                .active((intent.active)(theme)),
        )
        .transition(STANDARD_ENTER)
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
