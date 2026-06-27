use crate::Family;
use bevy_ecs::hierarchy::Children;
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::components::text::text_colored;
use crate::motion::transition::STANDARD_ENTER;
use crate::style::{StatefulPaint, Style};
use crate::theme::{Theme, theme};
use crate::tokens::{radius, spacing};

#[derive(Clone, Copy)]
pub struct ButtonIntent {
    family: fn(&Theme) -> Family,
}

pub mod intent {
    use super::ButtonIntent;
    use crate::Family;
    use crate::theme::Theme;
    use bevy_color::Color;

    pub const PRIMARY: ButtonIntent = ButtonIntent {
        family: |t: &Theme| t.primary,
    };

    pub const SECONDARY: ButtonIntent = ButtonIntent {
        family: |t: &Theme| t.secondary,
    };

    pub const DANGER: ButtonIntent = ButtonIntent {
        family: |t: &Theme| t.error_solid,
    };

    pub const MUTED: ButtonIntent = ButtonIntent {
        family: |t: &Theme| t.neutral,
    };

    pub const PLAIN: ButtonIntent = ButtonIntent {
        family: |t: &Theme| Family {
            base: Color::NONE,
            on: t.surface_canvas.on,
            ..t.secondary
        },
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
    Icon,
}

pub fn button(label: impl Into<String>) -> impl Scene {
    button_styled(intent::PRIMARY, ButtonSize::Md, label)
}

pub fn button_styled(
    intent: ButtonIntent,
    size: ButtonSize,
    label: impl Into<String>,
) -> impl Scene {
    let theme = theme();
    let style = intent_style(intent, &theme).merge(sized(size));
    let label = text_colored(label.into(), (intent.family)(&theme).on);
    bsn! {
        Node
        Button
        template_value(style)
        Children [ {EntityScene(label)} ]
    }
}

fn intent_style(intent: ButtonIntent, theme: &Theme) -> Style {
    let family = (intent.family)(theme);
    Style::new()
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
            node.column_gap = Val::Px(spacing::L);
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
        .background(
            StatefulPaint::new(family.base)
                .hover(family.hover)
                .active(family.active),
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
