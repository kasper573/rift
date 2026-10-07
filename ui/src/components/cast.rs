use bevy::image::Image;
use bevy_asset::{Assets, Handle};
use bevy_color::Color;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn};

use crate::component;
use bevy_ui::widget::ImageNode;
use bevy_ui::{Node, PositionType, Val, ZIndex};

use crate::Side;
use crate::motion::Transform2d;
use crate::motion::transition::{EMPHASIZED_ENTER, STANDARD_EXIT};
use crate::presence::{Leaving, Presence, PresenceMove};
use crate::style::Style;

const FRONT_HEIGHT: f32 = 64.0;
const BACK_HEIGHT: f32 = 56.0;
const BACK_LIFT: f32 = 8.0;
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
    face: Handle<Image>,
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
    casts: Query<(Entity, &Cast, Option<&Children>), Changed<Cast>>,
    mut busts: Query<(
        &mut Bust,
        &mut ImageNode,
        &mut Style,
        &mut Presence,
        Has<Leaving>,
    )>,
    mut commands: Commands,
) {
    for (stage, cast, kids) in &casts {
        let mut present = Vec::new();
        for &kid in kids.into_iter().flatten() {
            let Ok((mut bust, mut image, mut style, mut presence, leaving)) = busts.get_mut(kid)
            else {
                continue;
            };
            match cast.members.iter().find(|member| member.key == bust.key) {
                Some(member) => {
                    present.push(member.key);
                    if leaving {
                        commands.entity(kid).remove::<Leaving>();
                    }
                    if bust.face != member.image {
                        bust.face = member.image.clone();
                    }
                    image.flip_x = member.flip;
                    *style = shown(member);
                    let wanted = presence_of(member.side);
                    presence.enter = wanted.enter;
                    presence.exit = wanted.exit;
                    commands.entity(kid).insert(depth_order(member.depth));
                }
                None if !leaving => {
                    commands.entity(kid).insert(Leaving);
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
                    face: member.image.clone(),
                },
                ImageNode {
                    flip_x: member.flip,
                    ..ImageNode::new(member.image.clone())
                },
                presence_of(member.side).hidden(),
                shown(member),
                depth_order(member.depth),
                Pickable::IGNORE,
                ChildOf(stage),
            ));
        }
    }
}

pub(crate) fn show_loaded_faces(
    images: Res<Assets<Image>>,
    mut busts: Query<(&Bust, &mut ImageNode, &mut Presence), Without<Leaving>>,
) {
    for (bust, mut image, mut presence) in &mut busts {
        if !images.contains(&bust.face) {
            continue;
        }
        if image.image != bust.face {
            image.image = bust.face.clone();
        }
        if !presence.shown {
            presence.shown = true;
        }
    }
}

fn shown(member: &CastMember) -> Style {
    let (height, inset, lift) = match member.depth {
        CastDepth::Front => (FRONT_HEIGHT, FRONT_INSET, 0.0),
        CastDepth::Back => (BACK_HEIGHT, BACK_INSET, BACK_LIFT),
    };
    let side = member.side;
    Style::new()
        .node(move |node| {
            node.position_type = PositionType::Absolute;
            node.bottom = Val::Vh(lift);
            node.height = Val::Vh(height);
            match side {
                Side::Left => node.left = Val::Vw(inset),
                _ => node.right = Val::Vw(inset),
            }
        })
        .image_color(if member.lit { Color::WHITE } else { UNLIT })
        .transition(EMPHASIZED_ENTER)
}

fn presence_of(side: Side) -> Presence {
    Presence::new(
        PresenceMove::new(offstage(side), EMPHASIZED_ENTER),
        PresenceMove::new(offstage(side), STANDARD_EXIT),
    )
}

fn offstage(side: Side) -> Transform2d {
    let direction = match side {
        Side::Left => -1.0,
        _ => 1.0,
    };
    Transform2d::new(Vec2::new(direction * SLIDE, 0.0), 1.0)
}

fn depth_order(depth: CastDepth) -> ZIndex {
    match depth {
        CastDepth::Back => ZIndex(0),
        CastDepth::Front => ZIndex(1),
    }
}
