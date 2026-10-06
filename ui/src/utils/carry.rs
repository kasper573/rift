use bevy::image::Image;
use bevy_asset::Handle;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_picking::prelude::{Drag, DragDrop, DragEnd, DragStart, Pickable, Pointer};
use bevy_ui::widget::ImageNode;
use bevy_ui::{GlobalZIndex, Node, PositionType, Val};

use crate::components::input::{CatalogInput, InputRef};
use crate::cursor::CursorStyle;
use crate::opacity::Opacity;
use crate::state::ancestor_with;
use crate::tokens::layer;

const GHOST: f32 = 40.0;

#[derive(Component, Clone)]
#[require(Node, CursorStyle::Pointer)]
pub struct Carriable {
    pub image: Handle<Image>,
    pub payload: u64,
    pub input: InputRef,
}

#[derive(Component, Clone, Default)]
#[require(Node)]
pub struct CarryTarget;

#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Carried {
    #[event_target]
    pub target: Entity,
    pub source: Entity,
    pub payload: u64,
}

#[derive(Component)]
pub(crate) struct Ghost;

pub(crate) fn lift(
    start: On<Pointer<DragStart>>,
    carriables: Query<&Carriable>,
    parents: Query<&ChildOf>,
    is_carriable: Query<(), With<Carriable>>,
    gestures: CatalogInput,
    mut commands: Commands,
) {
    if start.entity != start.original_event_target() {
        return;
    }
    let Some(source) = ancestor_with::<Carriable>(start.entity, &parents, &is_carriable) else {
        return;
    };
    let Ok(carriable) = carriables.get(source) else {
        return;
    };
    if !gestures.dragged(carriable.input, start.button) {
        return;
    }
    let at = start.pointer_location.position;
    commands.spawn((
        Ghost,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(at.x - GHOST / 2.0),
            top: Val::Px(at.y - GHOST / 2.0),
            width: Val::Px(GHOST),
            height: Val::Px(GHOST),
            ..Node::default()
        },
        ImageNode::new(carriable.image.clone()),
        Opacity(0.8),
        GlobalZIndex(layer::CARRIED),
        Pickable::IGNORE,
    ));
}

pub(crate) fn follow(drag: On<Pointer<Drag>>, mut ghosts: Query<&mut Node, With<Ghost>>) {
    let at = drag.pointer_location.position;
    for mut node in &mut ghosts {
        node.left = Val::Px(at.x - GHOST / 2.0);
        node.top = Val::Px(at.y - GHOST / 2.0);
    }
}

pub(crate) fn set_down(
    _: On<Pointer<DragEnd>>,
    ghosts: Query<Entity, With<Ghost>>,
    mut commands: Commands,
) {
    for ghost in &ghosts {
        commands.entity(ghost).despawn();
    }
}

pub(crate) fn deliver(
    drop: On<Pointer<DragDrop>>,
    carriables: Query<&Carriable>,
    parents: Query<&ChildOf>,
    is_carriable: Query<(), With<Carriable>>,
    is_target: Query<(), With<CarryTarget>>,
    gestures: CatalogInput,
    mut commands: Commands,
) {
    if drop.entity != drop.original_event_target() {
        return;
    }
    let Some(source) = ancestor_with::<Carriable>(drop.dropped, &parents, &is_carriable) else {
        return;
    };
    let Some(target) = ancestor_with::<CarryTarget>(drop.entity, &parents, &is_target) else {
        return;
    };
    let Ok(carriable) = carriables.get(source) else {
        return;
    };
    if !gestures.dragged(carriable.input, drop.button) {
        return;
    }
    commands.trigger(Carried {
        target,
        source,
        payload: carriable.payload,
    });
}
