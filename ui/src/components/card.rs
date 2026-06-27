use bevy_picking::hover::Hovered;
use bevy_scene::{Scene, bsn, template_value};
use bevy_ui::{BorderRadius, BoxShadow, FlexDirection, Pressed, UiRect, Val};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::Pressable;
use crate::style::{StatefulPaint, Style};
use crate::theme::{Family, Theme, theme};
use crate::tokens::{radius, spacing};

#[derive(Clone, Copy)]
pub struct CardIntent {
    family: fn(&Theme) -> Family,
    bordered: bool,
}

impl Default for CardIntent {
    fn default() -> CardIntent {
        intent::DEFAULT
    }
}

pub mod intent {
    use super::CardIntent;
    use crate::theme::Theme;

    pub const DEFAULT: CardIntent = CardIntent {
        family: |t: &Theme| t.surface_elevated,
        bordered: true,
    };

    pub const SUCCESS: CardIntent = CardIntent {
        family: |t: &Theme| t.success_soft,
        bordered: false,
    };

    pub const ERROR: CardIntent = CardIntent {
        family: |t: &Theme| t.error_soft,
        bordered: false,
    };

    pub const INFO: CardIntent = CardIntent {
        family: |t: &Theme| t.info_soft,
        bordered: false,
    };

    pub const MUTED: CardIntent = CardIntent {
        family: |t: &Theme| t.neutral,
        bordered: false,
    };
}

#[derive(Default, Clone, Copy)]
pub struct CardOptions {
    pub intent: CardIntent,
    pub floating: bool,
    pub interactive: bool,
    pub compact: bool,
}

pub fn card(opts: CardOptions) -> impl Scene {
    let (floating, interactive) = (opts.floating, opts.interactive);
    let style = card_style(opts.intent, floating, interactive, opts.compact).op(move |entity| {
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
    bsn! {
        template_value(style)
        Pressable
    }
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

fn card_style(intent: CardIntent, floating: bool, interactive: bool, compact: bool) -> Style {
    let (corner, pad) = if compact {
        (radius::S, spacing::L)
    } else {
        (radius::M, spacing::XL)
    };
    let family = (intent.family)(&theme());
    let bordered = !floating && intent.bordered;
    let mut background = StatefulPaint::new(family.base);
    if interactive {
        background = background.hover(family.hover).active(family.active);
    }
    let mut style = Style::new()
        .background(background)
        .text_color(family.on)
        .node(move |node| {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::M);
            node.padding = UiRect::all(Val::Px(pad));
            node.border_radius = BorderRadius::all(Val::Px(corner));
            if bordered {
                node.border = UiRect::all(Val::Px(1.0));
            }
        });
    if bordered {
        style = style.border_color(family.border);
    }
    if interactive {
        style = style.transition(STANDARD_ENTER);
    }
    style
}
