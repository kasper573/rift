use bevy_ui::{AlignItems, BorderRadius, JustifyContent, UiRect, Val};
use bevy_view::{Bind, View, button};

use crate::Text;
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Paint, Style, Styled};
use crate::theme::{ColorVar, color};
use crate::tokens::{radius, spacing};

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

#[derive(Default)]
pub struct Button {
    label: String,
    variants: Vec<(&'static str, &'static str)>,
    modify: Option<Bind>,
    children: Vec<View>,
}

impl Button {
    pub fn label(mut self, label: impl Into<String>) -> Button {
        self.label = label.into();
        self
    }

    pub fn modify(mut self, decorate: Bind) -> Button {
        self.modify = Some(decorate);
        self
    }
}

variant_props!(Button { variant, size });
children_builder!(Button);

impl From<Button> for View {
    fn from(button_component: Button) -> View {
        let intent = selected_intent(&button_component.variants);
        let mut element = button().style(intent_style(intent, sized(&button_component.variants)));
        if !button_component.label.is_empty() {
            element = element.child(caption(button_component.label, intent.on));
        }
        element = element.children(button_component.children);
        if let Some(decorate) = button_component.modify {
            element = element.bind(decorate);
        }
        element.into()
    }
}

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

fn sized(variants: &[(&'static str, &'static str)]) -> Style {
    let (height, padding) = match chosen(variants, "size").unwrap_or("md") {
        "sm" => (32.0, spacing::L),
        "lg" => (48.0, spacing::XXL),
        _ => (40.0, spacing::XL),
    };
    Style::new().node(move |node| {
        node.height = Val::Px(height);
        node.padding = UiRect::horizontal(Val::Px(padding));
    })
}

fn selected_intent(variants: &[(&'static str, &'static str)]) -> &'static Intent {
    let name = chosen(variants, "variant").unwrap_or("primary");
    INTENTS
        .iter()
        .find(|intent| intent.name == name)
        .unwrap_or(&INTENTS[0])
}

fn chosen<'a>(variants: &[(&'a str, &'a str)], dimension: &str) -> Option<&'a str> {
    variants
        .iter()
        .find(|(name, _)| *name == dimension)
        .map(|(_, option)| *option)
}

/// Button caption in intent's on-color. [`Text`] ignores picking so clicks reach the button.
fn caption(label: String, on: ColorVar) -> View {
    Text::new(label).intent("label").color(on).into()
}
