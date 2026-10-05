use std::time::Duration;

use bevy::image::Image;
use bevy_asset::Handle;
use bevy_color::Color;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn};

use crate::component;
use bevy_time::Time;
use bevy_ui::widget::ImageNode;
use bevy_ui::{Node, PositionType, UiTransform, Val, ZIndex};

use crate::Side;
use crate::motion::transition::{EMPHASIZED_ENTER, STANDARD_EXIT};
use crate::motion::{Motion, Transform2d};
use crate::opacity::Opacity;
use crate::style::Style;

const FRONT_HEIGHT: f32 = 64.0;
const BACK_HEIGHT: f32 = 56.0;
const FRONT_INSET: f32 = 1.0;
const BACK_INSET: f32 = 13.0;
const SLIDE: f32 = 70.0;
const UNLIT: Color = Color::srgb(0.5, 0.5, 0.56);

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct Cast {
    pub members: Vec<CastMember>,
}

#[derive(Clone, PartialEq)]
pub struct CastMember {
    pub key: u64,
    pub image: Handle<Image>,
    pub side: Side,
    pub depth: CastDepth,
    pub lit: bool,
    pub flip: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CastDepth {
    Front,
    Back,
}

#[derive(Component)]
pub(crate) struct Bust {
    key: u64,
    side: Side,
    leaving: Option<Duration>,
}

pub fn cast() -> impl Scene {
    bsn! {
        component(Cast::default())
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
        }
        Pickable::IGNORE
    }
}

pub(crate) fn sync_cast(
    time: Res<Time>,
    casts: Query<(Entity, &Cast, Option<&Children>), Changed<Cast>>,
    mut busts: Query<(&mut Bust, &mut ImageNode, &mut Style)>,
    mut commands: Commands,
) {
    let now = time.elapsed();
    for (stage, cast, kids) in &casts {
        let mut present = Vec::new();
        for &kid in kids.into_iter().flatten() {
            let Ok((mut bust, mut image, mut style)) = busts.get_mut(kid) else {
                continue;
            };
            match cast.members.iter().find(|member| member.key == bust.key) {
                Some(member) => {
                    present.push(member.key);
                    bust.leaving = None;
                    bust.side = member.side;
                    if image.image != member.image {
                        image.image = member.image.clone();
                    }
                    image.flip_x = member.flip;
                    *style = shown(member);
                    commands.entity(kid).insert(depth_order(member.depth));
                }
                None if bust.leaving.is_none() => {
                    bust.leaving = Some(now);
                    *style = gone(style.clone(), bust.side);
                }
                None => {}
            }
        }
        for member in cast
            .members
            .iter()
            .filter(|member| !present.contains(&member.key))
        {
            commands.spawn((
                Bust {
                    key: member.key,
                    side: member.side,
                    leaving: None,
                },
                ImageNode {
                    flip_x: member.flip,
                    ..ImageNode::new(member.image.clone())
                },
                Motion::default(),
                Opacity(0.0),
                UiTransform::default(),
                shown(member),
                depth_order(member.depth),
                Pickable::IGNORE,
                ChildOf(stage),
            ));
        }
    }
}

pub(crate) fn reap_busts(time: Res<Time>, busts: Query<(Entity, &Bust)>, mut commands: Commands) {
    let now = time.elapsed();
    for (entity, bust) in &busts {
        if bust
            .leaving
            .is_some_and(|since| now.saturating_sub(since) >= STANDARD_EXIT.duration)
        {
            commands.entity(entity).despawn();
        }
    }
}

fn shown(member: &CastMember) -> Style {
    let (height, inset) = match member.depth {
        CastDepth::Front => (FRONT_HEIGHT, FRONT_INSET),
        CastDepth::Back => (BACK_HEIGHT, BACK_INSET),
    };
    let side = member.side;
    Style::new()
        .node(move |node| {
            node.position_type = PositionType::Absolute;
            node.bottom = Val::Px(0.0);
            node.height = Val::Vh(height);
            match side {
                Side::Left => node.left = Val::Vw(inset),
                _ => node.right = Val::Vw(inset),
            }
        })
        .image_color(if member.lit { Color::WHITE } else { UNLIT })
        .opacity(1.0)
        .enter_opacity(0.0)
        .translate(Vec2::ZERO)
        .enter(offstage(side))
        .transition(EMPHASIZED_ENTER)
}

fn gone(style: Style, side: Side) -> Style {
    style
        .opacity(0.0)
        .translate(offstage(side).translation)
        .transition(STANDARD_EXIT)
}

fn offstage(side: Side) -> Transform2d {
    let direction = match side {
        Side::Left => -1.0,
        _ => 1.0,
    };
    Transform2d {
        translation: Vec2::new(direction * SLIDE, 0.0),
        ..Transform2d::IDENTITY
    }
}

fn depth_order(depth: CastDepth) -> ZIndex {
    match depth {
        CastDepth::Back => ZIndex(0),
        CastDepth::Front => ZIndex(1),
    }
}
