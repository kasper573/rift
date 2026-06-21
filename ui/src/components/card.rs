use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_picking::hover::Hovered;
use bevy_ui::{BorderRadius, BoxShadow, FlexDirection, Node, Pressed, ShadowStyle, UiRect, Val};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::Pressable;
use crate::style::Style;
use crate::theme::{ColorVar, Family, color};
use crate::tokens::{radius, spacing};

#[derive(Default, Clone, Copy)]
pub struct CardOpts {
    pub intent: &'static str,
    pub floating: bool,
    pub interactive: bool,
    pub compact: bool,
}

pub fn card(opts: CardOpts) -> impl Bundle {
    let intent = card_intent(opts.intent);
    let (floating, interactive) = (opts.floating, opts.interactive);
    let style = card_style(&intent, floating, interactive, opts.compact).op(move |entity| {
        let hovered =
            entity.get::<Hovered>().is_some_and(Hovered::get) && entity.get::<Pressed>().is_none();
        match shadow(floating, interactive, hovered) {
            Some(shadow) => {
                entity.insert(shadow);
            }
            None => {
                entity.remove::<BoxShadow>();
            }
        }
    });
    (Node::default(), style, Pressable)
}

fn shadow(floating: bool, interactive: bool, hovered: bool) -> Option<BoxShadow> {
    let level = if floating {
        if interactive && hovered { 2 } else { 1 }
    } else if interactive && hovered {
        1
    } else {
        return None;
    };
    Some(elevation(level))
}

struct CardIntent {
    family: Family<ColorVar>,
    stroke: Option<ColorVar>,
}

fn card_intent(intent: &str) -> CardIntent {
    match intent {
        "success" => CardIntent {
            family: color::success_soft,
            stroke: None,
        },
        "error" => CardIntent {
            family: color::error_soft,
            stroke: None,
        },
        "info" => CardIntent {
            family: color::info_soft,
            stroke: None,
        },
        "muted" => CardIntent {
            family: color::neutral,
            stroke: None,
        },
        _ => CardIntent {
            family: color::surface_elevated,
            stroke: Some(color::surface_elevated.border),
        },
    }
}

fn card_style(palette: &CardIntent, floating: bool, interactive: bool, compact: bool) -> Style {
    let (corner, pad) = if compact {
        (radius::S, spacing::L)
    } else {
        (radius::M, spacing::XL)
    };
    let bordered = !floating && palette.stroke.is_some();
    let mut style = Style::new()
        .background(palette.family.base)
        .text_color(palette.family.on)
        .node(move |node| {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::M);
            node.padding = UiRect::all(Val::Px(pad));
            node.border_radius = BorderRadius::all(Val::Px(corner));
            if bordered {
                node.border = UiRect::all(Val::Px(1.0));
            }
        });
    if let Some(stroke) = palette.stroke.filter(|_| bordered) {
        style = style.border_color(stroke);
    }
    if interactive {
        style = style
            .hover(Style::new().background(palette.family.hover))
            .active(Style::new().background(palette.family.active))
            .transition(STANDARD_ENTER);
    }
    style
}

fn elevation(level: u8) -> BoxShadow {
    let scale = level as f32;
    BoxShadow(vec![
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(1.0 * scale),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(2.0 * scale),
        },
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(4.0 * scale),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(12.0 * scale),
        },
    ])
}
