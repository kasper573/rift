use bevy_ecs::bundle::Bundle;
use bevy_picking::hover::Hovered;
use bevy_ui::{BorderRadius, BoxShadow, FlexDirection, Node, Pressed, UiRect, Val};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::Pressable;
use crate::style::{StatefulPaint, Style};
use crate::theme::{ColorVar, Family, color};
use crate::tokens::{radius, spacing};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum CardIntent {
    #[default]
    Default,
    Success,
    Error,
    Info,
    Muted,
}

#[derive(Default, Clone, Copy)]
pub struct CardOpts {
    pub intent: CardIntent,
    pub floating: bool,
    pub interactive: bool,
    pub compact: bool,
}

pub fn card(opts: CardOpts) -> impl Bundle {
    let palette = opts.intent.palette();
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
    Some(crate::surface::elevation(level))
}

struct CardPalette {
    family: Family<ColorVar>,
    stroke: Option<ColorVar>,
}

impl CardIntent {
    fn palette(self) -> CardPalette {
        match self {
            CardIntent::Success => CardPalette {
                family: color::success_soft,
                stroke: None,
            },
            CardIntent::Error => CardPalette {
                family: color::error_soft,
                stroke: None,
            },
            CardIntent::Info => CardPalette {
                family: color::info_soft,
                stroke: None,
            },
            CardIntent::Muted => CardPalette {
                family: color::neutral,
                stroke: None,
            },
            CardIntent::Default => CardPalette {
                family: color::surface_elevated,
                stroke: Some(color::surface_elevated.border),
            },
        }
    }
}

fn card_style(palette: &CardPalette, floating: bool, interactive: bool, compact: bool) -> Style {
    let (corner, pad) = if compact {
        (radius::S, spacing::L)
    } else {
        (radius::M, spacing::XL)
    };
    let bordered = !floating && palette.stroke.is_some();
    let mut background = StatefulPaint::new(palette.family.base);
    if interactive {
        background = background
            .hover(palette.family.hover)
            .active(palette.family.active);
    }
    let mut style = Style::new()
        .background(background)
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
        style = style.transition(STANDARD_ENTER);
    }
    style
}
