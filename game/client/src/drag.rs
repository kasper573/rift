use std::sync::Arc;

use bevy::ecs::hierarchy::ChildOf;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::window::CursorIcon;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub pos: Vec2,
    pub size: Vec2,
}

#[derive(Component)]
pub struct DragRoot;

#[derive(Component)]
pub struct DragHandle;

#[derive(Component)]
pub struct ResizeHandle {
    pub min: Vec2,
}

type Settle = Arc<dyn Fn(&mut World, Geom) -> Geom + Send + Sync>;
type Tap = Arc<dyn Fn(&mut World) + Send + Sync>;

#[derive(Component, Clone)]
pub struct OnSettle(pub Settle);

impl OnSettle {
    pub fn new(handler: impl Fn(&mut World, Geom) -> Geom + Send + Sync + 'static) -> OnSettle {
        OnSettle(Arc::new(handler))
    }
}

#[derive(Component, Clone)]
pub struct OnTap(pub Tap);

impl OnTap {
    pub fn new(handler: impl Fn(&mut World) + Send + Sync + 'static) -> OnTap {
        OnTap(Arc::new(handler))
    }
}

#[derive(Component, Clone)]
pub struct HoverCursor(pub CursorIcon);

#[derive(Resource, Default)]
pub struct CursorLock(pub Option<CursorIcon>);

pub struct DragPlugin;

impl Plugin for DragPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorLock>()
            .add_observer(on_drag)
            .add_observer(on_drag_end)
            .add_observer(on_click);
    }
}

#[allow(clippy::too_many_arguments)]
fn on_drag(
    drag: On<Pointer<Drag>>,
    handles: Query<(), With<DragHandle>>,
    resizers: Query<&ResizeHandle>,
    cursors: Query<&HoverCursor>,
    is_root: Query<(), With<DragRoot>>,
    parents: Query<&ChildOf>,
    mut nodes: Query<&mut Node>,
    mut lock: ResMut<CursorLock>,
) {
    let entity = drag.entity;
    let Some(root) = nearest_root(entity, &parents, &is_root) else {
        return;
    };
    if let Ok(resize) = resizers.get(entity) {
        if let Ok(mut node) = nodes.get_mut(root) {
            node.width = Val::Px((px(node.width) + drag.delta.x).max(resize.min.x));
            node.height = Val::Px((px(node.height) + drag.delta.y).max(resize.min.y));
        }
        lock.0 = cursors.get(entity).ok().map(|cursor| cursor.0.clone());
    } else if handles.contains(entity)
        && let Ok(mut node) = nodes.get_mut(root)
    {
        node.left = Val::Px(px(node.left) + drag.delta.x);
        node.top = Val::Px(px(node.top) + drag.delta.y);
    }
}

type AnyHandle = Or<(With<DragHandle>, With<ResizeHandle>)>;

fn on_drag_end(
    drag: On<Pointer<DragEnd>>,
    handles: Query<(), AnyHandle>,
    is_root: Query<(), With<DragRoot>>,
    parents: Query<&ChildOf>,
    mut lock: ResMut<CursorLock>,
    mut commands: Commands,
) {
    lock.0 = None;
    if !handles.contains(drag.entity) {
        return;
    }
    let Some(root) = nearest_root(drag.entity, &parents, &is_root) else {
        return;
    };
    commands.queue(move |world: &mut World| settle(world, root));
}

fn on_click(
    click: On<Pointer<Click>>,
    taps: Query<&OnTap>,
    is_root: Query<(), With<DragRoot>>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let Some(root) = nearest_root(click.entity, &parents, &is_root) else {
        return;
    };
    let Ok(tap) = taps.get(root) else {
        return;
    };
    let tap = tap.0.clone();
    commands.queue(move |world: &mut World| tap(world));
}

pub fn hovered_cursor(world: &World) -> Option<CursorIcon> {
    if let Some(locked) = world
        .get_resource::<CursorLock>()
        .and_then(|lock| lock.0.clone())
    {
        return Some(locked);
    }
    let hover_map = world.get_resource::<HoverMap>()?;
    let mut topmost: Option<(f32, CursorIcon)> = None;
    for hits in hover_map.values() {
        for (&entity, hit) in hits.iter() {
            if let Some(cursor) = world.get::<HoverCursor>(entity)
                && topmost.as_ref().is_none_or(|(depth, _)| hit.depth > *depth)
            {
                topmost = Some((hit.depth, cursor.0.clone()));
            }
        }
    }
    topmost.map(|(_, cursor)| cursor)
}

fn nearest_root(
    mut entity: Entity,
    parents: &Query<&ChildOf>,
    is_root: &Query<(), With<DragRoot>>,
) -> Option<Entity> {
    loop {
        if is_root.get(entity).is_ok() {
            return Some(entity);
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

fn settle(world: &mut World, root: Entity) {
    let Some(settle) = world.get::<OnSettle>(root).cloned() else {
        return;
    };
    let geom = read_geom(world, root);
    let settled = (settle.0)(world, geom);
    write_geom(world, root, settled);
}

fn read_geom(world: &World, root: Entity) -> Geom {
    let node = world.get::<Node>(root);
    let read = |get: fn(&Node) -> Val| node.map_or(0.0, |node| px(get(node)));
    Geom {
        pos: Vec2::new(read(|n| n.left), read(|n| n.top)),
        size: Vec2::new(read(|n| n.width), read(|n| n.height)),
    }
}

fn write_geom(world: &mut World, root: Entity, geom: Geom) {
    if let Some(mut node) = world.get_mut::<Node>(root) {
        node.left = Val::Px(geom.pos.x);
        node.top = Val::Px(geom.pos.y);
        if matches!(node.width, Val::Px(_)) {
            node.width = Val::Px(geom.size.x);
        }
        if matches!(node.height, Val::Px(_)) {
            node.height = Val::Px(geom.size.y);
        }
    }
}

fn px(val: Val) -> f32 {
    match val {
        Val::Px(value) => value,
        _ => 0.0,
    }
}
