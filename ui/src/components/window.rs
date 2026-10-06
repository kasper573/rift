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
    pub frame: WindowFrame,
    pub content: Vec<WindowContent>,
}

pub enum WindowFrame {
    Floating {
        pos: Vec2,
        size: Vec2,
        on_close: OnTap,
        on_settle: OnSettle,
    },
    Anchored {
        width: Val,
        height: Val,
    },
}

pub fn window(opts: WindowOptions) -> impl Scene {
    let family = theme().surface_floating;
    let (titles, scenes): (Vec<String>, Vec<Box<dyn Scene>>) = opts
        .content
        .into_iter()
        .map(|content| (content.title, content.scene))
        .unzip();
    let initial: Vec<String> = titles.iter().take(1).map(|_| tab_value(0)).collect();
    let Chrome {
        node,
        on_close,
        settle,
        grip,
    } = chrome(opts.frame);
    bsn! {
        {settle}
        template_value(node)
        BackgroundColor({family.base})
        component(BorderColor::all(family.border))
        component(SelectGroup { exclusive: true, toggleable: false, initial })
        DragRoot
        Children [
            {EntityScene(header(titles, on_close))},
            {EntityScene(body(scenes))},
            {grip},
        ]
    }
}

struct Chrome {
    node: Node,
    on_close: Option<OnTap>,
    settle: Box<dyn Scene>,
    grip: Vec<Box<dyn Scene>>,
}

fn chrome(frame: WindowFrame) -> Chrome {
    let base = Node {
        border: UiRect::all(Val::Px(1.0)),
        flex_direction: FlexDirection::Column,
        overflow: Overflow::clip(),
        ..default()
    };
    match frame {
        WindowFrame::Floating {
            pos,
            size,
            on_close,
            on_settle,
        } => Chrome {
            node: Node {
                position_type: PositionType::Absolute,
                left: Val::Px(pos.x),
                top: Val::Px(pos.y),
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..base
            },
            on_close: Some(on_close),
            settle: Box::new(component(on_settle)),
            grip: vec![Box::new(resize_grip())],
        },
        WindowFrame::Anchored { width, height } => Chrome {
            node: Node {
                position_type: PositionType::Relative,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width,
                height,
                ..base
            },
            on_close: None,
            settle: Box::new(bsn! {}),
            grip: Vec::new(),
        },
    }
}

fn tab_value(index: usize) -> String {
    index.to_string()
}

fn header(titles: Vec<String>, on_close: Option<OnTap>) -> impl Scene {
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
    let close: Vec<Box<dyn Scene>> = on_close
        .map(|on_close| -> Box<dyn Scene> {
            Box::new(bsn! {
                Node { padding: {UiRect::horizontal(Val::Px(6.0))} }
                Children [ {EntityScene(close_button(on_close))} ]
            })
        })
        .into_iter()
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
            {close},
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
