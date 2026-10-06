use bevy_color::{Alpha, Color};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn};
use bevy_ui::widget::TextShadow;
use bevy_ui::{AlignItems, FlexDirection, Node, Val};

use crate::component;
use crate::components::captions::{CountHost, LabelledLine, LineHost, PlaceParts, label_text};
use crate::components::rich_text::RichText;
use crate::components::text::styled_text;
use crate::cursor::ClickThrough;
use crate::motion::{Easing, Timing, Transform2d};
use crate::presence::{Presence, PresenceMove};
use crate::tokens::{palette, spacing, typography};

const LINE: typography::Typography = typography::Typography {
    font_size: 34.0,
    line_height: 40.0,
    ..typography::NAME
};
const MILESTONE_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::ZERO, 1.25),
        Timing::new(520, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -14.0), 1.04),
        Timing::new(350, Easing::StandardAccelerate),
    ),
)
.collapsing();

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct Milestones {
    pub rows: Vec<LabelledLine>,
    pub more: Option<RichText>,
}

pub fn milestones() -> impl Scene {
    bsn! {
        Milestones
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px({spacing::L}),
        }
        component(Hovered::default())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            (
                LineHost
                Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: Val::Px({spacing::XL}) }
                Pickable { should_block_lower: false, is_hoverable: true }
            ),
            ( CountHost Node Pickable { should_block_lower: false, is_hoverable: true } ),
        ]
    }
}

pub(crate) fn sync_milestones(
    milestones: Query<(Entity, &Milestones), Changed<Milestones>>,
    mut places: PlaceParts,
    mut commands: Commands,
) {
    for (root, milestones) in &milestones {
        places.show_lines(
            &mut commands,
            root,
            &milestones.rows,
            MILESTONE_PRESENCE,
            |line| Box::new(milestone_row(line.clone())),
        );
        places.show_count(&mut commands, root, milestones.more.clone());
    }
}

fn milestone_row(line: LabelledLine) -> impl Scene {
    let shadow = TextShadow {
        offset: bevy_math::Vec2::new(0.0, 2.0),
        color: Color::BLACK.with_alpha(0.8),
    };
    let words = line.text.plain(&Default::default());
    bsn! {
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: Val::Px({spacing::S}) }
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            ( {label_text(&line.label, line.replaced, palette::SLATE_100.with_alpha(0.85))} component(shadow) ),
            ( {styled_text(words, palette::AMBER_80, LINE)} component(shadow) ),
        ]
    }
}
