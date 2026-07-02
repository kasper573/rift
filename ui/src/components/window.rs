use bevy::prelude::*;
use bevy_scene::{EntityScene, Scene, bsn, on, template_value};

use crate::components::button::{ButtonSize, button_styled, intent};
use crate::components::{tabs_content, tabs_trigger, text};
use crate::drag::{DragHandle, DragRoot, OnSettle, OnTap, ResizeHandle};
use crate::state::SelectGroup;
use crate::style::Style;
use crate::theme::theme;
use crate::{Activate, component};

const MIN_WINDOW: Vec2 = Vec2::new(100.0, 100.0);

pub struct WindowContent {
    pub title: String,
    pub scene: Box<dyn Scene>,
}

pub struct WindowOptions {
    pub pos: Vec2,
    pub size: Vec2,
    pub on_close: OnTap,
    pub on_settle: OnSettle,
    pub content: Vec<WindowContent>,
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
    let (titles, scenes): (Vec<String>, Vec<Box<dyn Scene>>) = opts
        .content
        .into_iter()
        .map(|content| (content.title, content.scene))
        .unzip();
    let initial: Vec<String> = titles.iter().take(1).map(|_| tab_value(0)).collect();
    bsn! {
        template_value(node)
        BackgroundColor({family.base})
        component(BorderColor::all(family.border))
        component(SelectGroup { exclusive: true, toggleable: false, initial })
        DragRoot
        component(opts.on_settle)
        Children [
            {EntityScene(header(titles, opts.on_close))},
            {EntityScene(body(scenes))},
            {EntityScene(resize_grip())},
        ]
    }
}

fn tab_value(index: usize) -> String {
    index.to_string()
}

fn header(titles: Vec<String>, on_close: OnTap) -> impl Scene {
    let family = theme().surface_inset;
    let triggers: Vec<Box<dyn Scene>> = titles
        .into_iter()
        .enumerate()
        .map(|(index, title)| -> Box<dyn Scene> {
            Box::new(bsn! {
                {tabs_trigger(tab_value(index))}
                Children [ {EntityScene(text(title))} ]
            })
        })
        .collect();
    bsn! {
        template_value(Style::new().background(family.base).node(|node| {
            node.width = Val::Percent(100.0);
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::SpaceBetween;
        }))
        DragHandle
        Children [
            (
                Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Stretch }
                Children [ {triggers} ]
            ),
            (
                Node { padding: {UiRect::horizontal(Val::Px(6.0))} }
                Children [ {EntityScene(close_button(on_close))} ]
            ),
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

fn body(scenes: Vec<Box<dyn Scene>>) -> impl Scene {
    let panes: Vec<Box<dyn Scene>> = scenes
        .into_iter()
        .enumerate()
        .map(|(index, scene)| -> Box<dyn Scene> {
            Box::new(bsn! {
                {tabs_content(tab_value(index))}
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
                Children [ {EntityScene(scene)} ]
            })
        })
        .collect();
    bsn! {
        Node { flex_grow: 1.0, min_height: Val::Px(0.0) }
        Children [ {panes} ]
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
