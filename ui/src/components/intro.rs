use bevy::image::Image;
use bevy_asset::Handle;
use bevy_color::{Alpha, Color};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn, template_value};
use bevy_time::Time;
use bevy_ui::widget::{ImageNode, TextShadow};
use bevy_ui::{AlignItems, BorderRadius, FlexDirection, Node, UiRect, UiTransform, Val, Val2};

use crate::component;
use crate::components::rich_text::{MotionPreference, RichText, TextMotion, rich_text};
use crate::components::text::styled_text;
use crate::cursor::ClickThrough;
use crate::motion::{Easing, Timing, Transform2d};
use crate::presence::{Presence, PresenceMove};
use crate::style::Style;
use crate::tokens::{font, palette, radius, spacing, typography};

const TITLE: typography::Typography = typography::Typography {
    font_size: 44.0,
    line_height: 56.0,
    weight: font::WEIGHT_BOLD,
    family: font::FAMILY_DISPLAY,
};
const EMBLEM: f32 = 48.0;
const RULE: f32 = 64.0;
const WIDEST: f32 = 560.0;
const INTRO_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 14.0), 0.94),
        Timing::new(800, Easing::Standard),
    ),
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, -8.0), 1.04),
        Timing::new(600, Easing::StandardAccelerate),
    ),
);

#[derive(Component, Clone, PartialEq)]
#[require(Node)]
pub struct Intro {
    pub title: String,
    pub text: RichText,
    pub tint: Option<Color>,
    pub motion: Option<TextMotion>,
    pub emblem: Option<Handle<Image>>,
}

pub fn intro(intro: Intro) -> impl Scene {
    let shadow = TextShadow {
        offset: Vec2::new(0.0, 2.0),
        color: Color::BLACK.with_alpha(0.8),
    };
    let tint = intro.tint.unwrap_or(palette::SLATE_100);
    let text = RichText {
        size: typography::LINE.font_size,
        color: palette::SLATE_100,
        ..intro.text.clone()
    };
    let emblem: Vec<Box<dyn Scene>> = intro
        .emblem
        .clone()
        .map(|image| -> Box<dyn Scene> {
            Box::new(bsn! {
                Node { width: Val::Px(EMBLEM), height: Val::Px(EMBLEM) }
                ImageNode { image: {image} }
                Pickable::IGNORE
            })
        })
        .into_iter()
        .collect();
    let rule = move || {
        bsn! {
            Node { width: Val::Px(RULE), height: Val::Px(1.0) }
            template_value(Style::new().background(tint.with_alpha(0.6)))
            Pickable::IGNORE
        }
    };
    let backdrop = Style::new()
        .background(Color::BLACK.with_alpha(0.35))
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::XXXL), Val::Px(spacing::XL));
            node.border_radius = BorderRadius::all(Val::Px(radius::L));
        });
    bsn! {
        component(intro.clone())
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px({spacing::L}),
            max_width: Val::Px(WIDEST),
        }
        template_value(backdrop)
        component(INTRO_PRESENCE)
        component(Hovered::default())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            {emblem},
            (
                Node { align_items: AlignItems::Center, column_gap: Val::Px({spacing::XL}) }
                Pickable::IGNORE
                Children [
                    {rule()},
                    ( {styled_text(intro.title.clone(), tint, TITLE)} IntroTitle component(shadow) UiTransform ),
                    {rule()},
                ]
            ),
            ( {rich_text(text, false)} ),
        ]
    }
}

#[derive(Component, Default, Clone)]
pub(crate) struct IntroTitle;

pub(crate) fn move_intro_titles(
    time: Res<Time>,
    preference: Res<MotionPreference>,
    intros: Query<(&Intro, &Children)>,
    rows: Query<&Children>,
    mut titles: Query<&mut UiTransform, With<IntroTitle>>,
) {
    let t = time.elapsed_secs();
    for (intro, kids) in &intros {
        let (offset, scale) = match intro.motion.filter(|_| !preference.reduced) {
            None => (Vec2::ZERO, 1.0),
            Some(TextMotion::Pulse) => (Vec2::ZERO, 1.0 + (t * 3.0).sin() * 0.04),
            Some(TextMotion::Wave) => (Vec2::new(0.0, (t * 4.0).sin() * 3.0), 1.0),
            Some(TextMotion::Shake) => (
                Vec2::new((t * 53.0).sin() * 1.5, (t * 47.0).cos() * 1.5),
                1.0,
            ),
        };
        let title_rows = kids.iter().filter_map(|kid| rows.get(kid).ok());
        for row in title_rows {
            let mut found = titles.iter_many_mut(row.iter());
            while let Some(mut transform) = found.fetch_next() {
                let next = UiTransform {
                    translation: Val2::px(offset.x, offset.y),
                    scale: Vec2::splat(scale),
                    ..*transform
                };
                if *transform != next {
                    *transform = next;
                }
            }
        }
    }
}
