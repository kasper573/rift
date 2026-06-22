use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::widget::ImageNode;

use crate::components::{text_colored, tooltip, tooltip_content};
use crate::drag::{DragHandle, DragRoot, HoverCursor, OnSettle, OnTap};
use crate::style::Style;
use crate::theme::theme;
use crate::{Align, Side, component};

const WIDGET: f32 = 48.0;
const POINTER: CursorIcon = CursorIcon::System(SystemCursorIcon::Pointer);

/// A draggable widget: an icon with a corner badge and a hover tooltip. A tap (press without a drag)
/// runs `on_tap`; settling after a drag runs `on_settle`.
pub struct Widget {
    pub pos: Vec2,
    pub icon: Handle<Image>,
    pub badge: String,
    pub tooltip: String,
    pub on_tap: OnTap,
    pub on_settle: OnSettle,
}

pub fn widget(opts: Widget) -> impl Scene {
    let family = theme().surface_floating;
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(opts.pos.x),
        top: Val::Px(opts.pos.y),
        width: Val::Px(WIDGET),
        height: Val::Px(WIDGET),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    };
    bsn! {
        template_value(node)
        BackgroundColor({family.base})
        component(BorderColor::all(family.border))
        {tooltip(false)}
        DragRoot
        DragHandle
        component(opts.on_tap)
        component(opts.on_settle)
        component(HoverCursor(POINTER))
        Children [
            (
                Node {
                    width: Val::Px(32.0),
                    height: Val::Px(32.0),
                    margin: {UiRect::all(Val::Px(8.0))},
                }
                component(ImageNode::new(opts.icon))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            {EntityScene(badge(opts.badge))},
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(label(opts.tooltip))} ]
            ),
        ]
    }
}

fn badge(text: String) -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute, right: Val::Px(2.0), bottom: Val::Px(2.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text, theme().surface_floating.on))} ]
    }
}

fn label(text: String) -> impl Scene {
    let family = theme().surface_inset;
    bsn! {
        template_value(Style::new().background(family.base).node(|node| {
            node.padding = UiRect::axes(Val::Px(6.0), Val::Px(3.0));
        }))
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text, family.on))} ]
    }
}
