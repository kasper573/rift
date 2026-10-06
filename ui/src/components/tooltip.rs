use std::time::Duration;

use bevy_color::Alpha;
use bevy_ecs::hierarchy::Children;
use bevy_picking::prelude::Pickable;
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::{
    BorderRadius, FlexDirection, GlobalZIndex, Node, OverrideClip, PositionType, UiRect, Val,
};

use crate::component;
use crate::components::rich_text::{RichPiece, RichText, rich_text};
use crate::components::text::styled_text;
use crate::overlay::{Open, OverlayContent, POPPER_ENTER, POPPER_EXIT, TooltipTimer};
use crate::place::Placement;
use crate::style::Style;
use crate::surface::elevation;
use crate::theme::theme;
use crate::tokens::layer;
use crate::tokens::{radius, typography};
use crate::{Align, Side};

const DELAY: Duration = Duration::from_millis(400);
const SKIP_DELAY: Duration = Duration::from_millis(300);
const TEXT_WIDTH: f32 = 260.0;

#[derive(Clone)]
pub struct TooltipText {
    pub title: String,
    pub lines: Vec<Vec<RichPiece>>,
    pub hint: Option<Vec<RichPiece>>,
}

pub fn tooltip(open: bool) -> impl Scene {
    bsn! {
        Open({open})
        component(TooltipTimer::new(DELAY, SKIP_DELAY))
    }
}

pub fn tooltip_content(side: Side, align: Align, offset: f32) -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute }
        GlobalZIndex({layer::ANCHORED})
        OverrideClip
        Placement { side: {side}, align: {align}, offset: {offset} }
        {OverlayContent::animated(POPPER_ENTER, POPPER_EXIT)}
        Pickable::IGNORE
    }
}

pub fn tooltip_text(text: TooltipText) -> impl Scene {
    let TooltipText { title, lines, hint } = text;
    let family = theme().surface_floating;
    let style = Style::new()
        .background(family.base)
        .border_color(family.on.with_alpha(0.15))
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(2.0);
            node.padding = UiRect::axes(Val::Px(8.0), Val::Px(5.0));
            node.max_width = Val::Px(TEXT_WIDTH);
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        });
    let caption = |pieces: Vec<RichPiece>, alpha: f32| -> Box<dyn Scene> {
        Box::new(rich_text(
            RichText {
                pieces,
                size: typography::CAPTION.font_size,
                color: family.on.with_alpha(alpha),
            },
            false,
        ))
    };
    let lines: Vec<Box<dyn Scene>> = lines
        .into_iter()
        .map(|line| caption(line, 0.85))
        .chain(hint.map(|hint| caption(hint, 0.55)))
        .collect();
    bsn! {
        template_value(style)
        template_value(elevation(1))
        Pickable::IGNORE
        Children [ {EntityScene(styled_text(title, family.on, typography::LABEL))}, {lines} ]
    }
}
