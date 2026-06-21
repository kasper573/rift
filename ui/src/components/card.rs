use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_picking::hover::Hovered;
use bevy_ui::{BorderRadius, BoxShadow, FlexDirection, Node, Pressed, ShadowStyle, UiRect, Val};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::Pressable;
use crate::style::Style;
use crate::theme::{ColorVar, color};
use crate::tokens::{radius, spacing};

#[derive(Default, Clone, Copy)]
pub struct CardOpts {
    pub intent: &'static str,
    pub floating: bool,
    pub interactive: bool,
    pub compact: bool,
}

pub fn card(opts: CardOpts) -> impl Bundle {
    let palette = palette(opts.intent);
    let (floating, interactive) = (opts.floating, opts.interactive);
    let style = card_style(&palette, floating, interactive, opts.compact).op(move |entity| {
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

struct Palette {
    base: ColorVar,
    hover: ColorVar,
    active: ColorVar,
    on: ColorVar,
    stroke: Option<ColorVar>,
}

fn palette(intent: &str) -> Palette {
    match intent {
        "success" => Palette {
            base: color::success_soft_base,
            hover: color::success_soft_hover,
            active: color::success_soft_active,
            on: color::success_soft_on,
            stroke: None,
        },
        "error" => Palette {
            base: color::error_soft_base,
            hover: color::error_soft_hover,
            active: color::error_soft_active,
            on: color::error_soft_on,
            stroke: None,
        },
        "info" => Palette {
            base: color::info_soft_base,
            hover: color::info_soft_hover,
            active: color::info_soft_active,
            on: color::info_soft_on,
            stroke: None,
        },
        "muted" => Palette {
            base: color::neutral_base,
            hover: color::neutral_hover,
            active: color::neutral_active,
            on: color::neutral_on,
            stroke: None,
        },
        _ => Palette {
            base: color::surface_elevated_base,
            hover: color::surface_elevated_hover,
            active: color::surface_elevated_active,
            on: color::surface_elevated_on,
            stroke: Some(color::surface_elevated_border_decorative),
        },
    }
}

fn card_style(palette: &Palette, floating: bool, interactive: bool, compact: bool) -> Style {
    let (corner, pad) = if compact {
        (radius::S, spacing::L)
    } else {
        (radius::M, spacing::XL)
    };
    let bordered = !floating && palette.stroke.is_some();
    let mut style = Style::new()
        .background(palette.base)
        .text_color(palette.on)
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
            .hover(Style::new().background(palette.hover))
            .active(Style::new().background(palette.active))
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
