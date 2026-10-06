use bevy_color::Alpha;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_picking::prelude::{Click, Pickable, Pointer};
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::{AlignItems, BorderRadius, FlexDirection, JustifyContent, Node, UiRect, Val};

use crate::component;
use crate::components::captions::{CountHost, LabelledLine, LineHost, PlaceParts, label_text};
use crate::components::rich_text::{RichText, rich_text};
use crate::components::text::styled_text;
use crate::cursor::{ClickThrough, CursorStyle};
use crate::motion::{Easing, Timing, Transform2d};
use crate::presence::{Presence, PresenceMove};
use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{palette, radius, spacing, typography};

const WIDEST: f32 = 580.0;
const ALERT_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -16.0), 0.98),
        Timing::new(360, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -10.0), 0.98),
        Timing::new(220, Easing::EmphasizedAccelerate),
    ),
)
.collapsing();

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct AlertBar {
    pub alerts: Vec<LabelledLine>,
    pub more: Option<RichText>,
}

#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct AlertClosed {
    #[event_target]
    pub bar: Entity,
    pub key: u64,
}

pub fn alert_bar() -> impl Scene {
    bsn! {
        AlertBar
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px({spacing::M}),
            width: Val::Px(WIDEST),
            max_width: Val::Vw(94.0),
        }
        component(Hovered::default())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            (
                LineHost
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::M}), width: Val::Percent(100.0) }
                Pickable { should_block_lower: false, is_hoverable: true }
            ),
            ( CountHost Node Pickable { should_block_lower: false, is_hoverable: true } ),
        ]
    }
}

pub(crate) fn sync_alert_bars(
    bars: Query<(Entity, &AlertBar), Changed<AlertBar>>,
    mut places: PlaceParts,
    mut commands: Commands,
) {
    for (root, bar) in &bars {
        places.show_lines(&mut commands, root, &bar.alerts, ALERT_PRESENCE, |line| {
            Box::new(alert_row(line.clone()))
        });
        places.show_count(&mut commands, root, bar.more.clone());
    }
}

#[derive(Component, Clone, Copy)]
pub(crate) struct AlertCloseButton(u64);

pub(crate) fn close_alert(
    click: On<Pointer<Click>>,
    buttons: Query<&AlertCloseButton>,
    parents: Query<&ChildOf>,
    is_bar: Query<(), With<AlertBar>>,
    mut commands: Commands,
) {
    let Ok(button) = buttons.get(click.entity) else {
        return;
    };
    if let Some(bar) = ancestor_with::<AlertBar>(click.entity, &parents, &is_bar) {
        commands.trigger(AlertClosed { bar, key: button.0 });
    }
}

fn alert_row(line: LabelledLine) -> impl Scene {
    let surface = theme().surface_trough;
    let ink = surface.on;
    let frame = Style::new()
        .background(surface.base.with_alpha(0.94))
        .border_color(palette::AZURE_70)
        .node(|node| {
            node.width = Val::Percent(100.0);
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::SpaceBetween;
            node.column_gap = Val::Px(spacing::L);
            node.padding = UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        });
    let text = RichText {
        size: typography::BODY.font_size,
        color: ink,
        ..line.text
    };
    let close = bsn! {
        Node { padding: {UiRect::axes(Val::Px(spacing::M), Val::Px(spacing::S))} }
        component(AlertCloseButton(line.key))
        component(CursorStyle::Pointer)
        Pickable { should_block_lower: true, is_hoverable: true }
        Children [ {EntityScene(styled_text("\u{d7}", ink.with_alpha(0.7), typography::NAME))} ]
    };
    bsn! {
        template_value(frame)
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            (
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::S}), flex_shrink: 1.0, min_width: Val::Px(0.0) }
                Pickable::IGNORE
                Children [
                    {EntityScene(label_text(&line.label, line.replaced, palette::AZURE_80))},
                    {EntityScene(rich_text(text, false))},
                ]
            ),
            {close},
        ]
    }
}
