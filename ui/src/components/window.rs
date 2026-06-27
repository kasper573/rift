use bevy::prelude::*;
use bevy_scene::{EntityScene, Scene, bsn, on, template_value};

use crate::components::button::{ButtonSize, button_styled, intent};
use crate::components::{scroll_area, scroll_bar, scroll_thumb, scroll_viewport, text_colored};
use crate::drag::{DragHandle, DragRoot, OnSettle, OnTap, ResizeHandle};
use crate::style::Style;
use crate::theme::theme;
use crate::{Activate, component};

const TITLE_H: f32 = 22.0;
const MIN_WINDOW: Vec2 = Vec2::new(100.0, 100.0);

/// A draggable, resizable window: a title bar (title + close), a scrolling content area, and a resize
/// grip. Closing runs `on_close`; settling after a drag or resize runs `on_settle`.
pub struct WindowOptions {
    pub pos: Vec2,
    pub size: Vec2,
    pub title: String,
    pub on_close: OnTap,
    pub on_settle: OnSettle,
    pub content: Box<dyn Scene>,
}

pub fn window(opts: WindowOptions) -> impl Scene {
    let family = theme().surface_floating;
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(opts.pos.x),
        top: Val::Px(opts.pos.y),
        width: Val::Px(opts.size.x),
        height: Val::Px(opts.size.y),
        border: UiRect::all(Val::Px(1.0)),
        flex_direction: FlexDirection::Column,
        overflow: Overflow::clip(),
        ..default()
    };
    bsn! {
        template_value(node)
        BackgroundColor({family.base})
        component(BorderColor::all(family.border))
        DragRoot
        component(opts.on_settle)
        Children [
            {EntityScene(title_bar(opts.title, opts.on_close))},
            {EntityScene(body(opts.content))},
            {EntityScene(resize_grip())},
        ]
    }
}

fn title_bar(title: String, on_close: OnTap) -> impl Scene {
    let family = theme().surface_inset;
    bsn! {
        template_value(Style::new().background(family.base).node(|node| {
            node.width = Val::Percent(100.0);
            node.height = Val::Px(TITLE_H);
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::SpaceBetween;
            node.padding = UiRect::horizontal(Val::Px(6.0));
        }))
        DragHandle
        Children [
            {EntityScene(text_colored(title, family.on))},
            {EntityScene(close_button(on_close))},
        ]
    }
}

fn close_button(on_close: OnTap) -> impl Scene {
    let close = on_close.0;
    bsn! {
        {button_styled(intent::PRIMARY, ButtonSize::Icon, "×")}
        on(move |_: On<Activate>, mut commands: Commands| {
            let close = close.clone();
            commands.queue(move |world: &mut World| close(world));
        })
    }
}

fn body(content: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { flex_grow: 1.0, min_height: Val::Px(0.0) }
        Children [
            ( {scroll_area()}
              Children [
                ( {scroll_viewport()}
                  Children [
                    (
                        Node { width: Val::Percent(100.0), padding: {UiRect::all(Val::Px(4.0))} }
                        Children [ {EntityScene(content)} ]
                    )
                  ]
                ),
                ( {scroll_bar()} Children [ {EntityScene(scroll_thumb())} ] )
              ]
            )
        ]
    }
}

fn resize_grip() -> impl Scene {
    let family = theme().surface_floating;
    bsn! {
        template_value(Style::new().background(family.border).node(|node| {
            node.position_type = PositionType::Absolute;
            node.right = Val::Px(0.0);
            node.bottom = Val::Px(0.0);
            node.width = Val::Px(16.0);
            node.height = Val::Px(16.0);
        }))
        ResizeHandle { min: {MIN_WINDOW} }
    }
}
