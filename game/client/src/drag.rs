use std::sync::Arc;

use bevy::ecs::hierarchy::ChildOf;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub pos: Vec2,
    pub size: Vec2,
}

#[derive(Component, Default, Clone)]
pub struct DragRoot;

#[derive(Component, Default, Clone)]
pub struct DragHandle;

#[derive(Component, Default, Clone)]
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

/// The snap grid in logical pixels (0 disables); the hud keeps it in step with the user's setting.
#[derive(Resource, Default)]
pub struct SnapGrid(pub f32);

/// Whether the press in progress has become a drag. A `Pointer<Click>` still fires when a drag ends,
/// so without this a drag would also trigger the tap (and open the widget's window).
#[derive(Resource, Default)]
struct Dragged(bool);

/// The viewport the panels were last laid out against, so a resize can rescale their positions.
#[derive(Resource, Default)]
struct LastViewport(Vec2);

/// The live drag. Its `raw` geometry accumulates the unsnapped pointer motion so snapping can render
/// every frame without the per-event rounding drifting; the node shows the snapped value.
#[derive(Resource, Default)]
struct DragState(Option<Active>);

struct Active {
    root: Entity,
    raw: Geom,
    resize: bool,
    min: Vec2,
}

pub struct DragPlugin;

impl Plugin for DragPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorLock>()
            .init_resource::<SnapGrid>()
            .init_resource::<Dragged>()
            .init_resource::<LastViewport>()
            .init_resource::<DragState>()
            .add_observer(on_press)
            .add_observer(on_drag_start)
            .add_observer(on_drag)
            .add_observer(on_drag_end)
            .add_observer(on_click)
            .add_systems(Update, scale_on_resize);
    }
}

fn on_press(_: On<Pointer<Press>>, mut dragged: ResMut<Dragged>) {
    dragged.0 = false;
}

#[allow(clippy::too_many_arguments)]
fn on_drag_start(
    drag: On<Pointer<DragStart>>,
    handles: Query<(), With<DragHandle>>,
    resizers: Query<&ResizeHandle>,
    is_root: Query<(), With<DragRoot>>,
    parents: Query<&ChildOf>,
    nodes: Query<&Node>,
    mut dragged: ResMut<Dragged>,
    mut state: ResMut<DragState>,
) {
    let entity = drag.entity;
    let resize = resizers.get(entity).ok();
    if resize.is_none() && !handles.contains(entity) {
        return;
    }
    dragged.0 = true;
    let Some(root) = nearest_root(entity, &parents, &is_root) else {
        return;
    };
    let raw = nodes.get(root).map(read_geom).unwrap_or(Geom {
        pos: Vec2::ZERO,
        size: Vec2::ZERO,
    });
    state.0 = Some(Active {
        root,
        raw,
        resize: resize.is_some(),
        min: resize.map_or(Vec2::ZERO, |handle| handle.min),
    });
}

fn on_drag(
    drag: On<Pointer<Drag>>,
    cursors: Query<&HoverCursor>,
    snap: Res<SnapGrid>,
    mut state: ResMut<DragState>,
    mut nodes: Query<&mut Node>,
    mut lock: ResMut<CursorLock>,
) {
    let Some(active) = state.0.as_mut() else {
        return;
    };
    if active.resize {
        active.raw.size = (active.raw.size + drag.delta).max(active.min);
        lock.0 = cursors.get(drag.entity).ok().map(|cursor| cursor.0.clone());
    } else {
        active.raw.pos += drag.delta;
    }
    let shown = snapped(active.raw, snap.0, active.resize, active.min);
    if let Ok(mut node) = nodes.get_mut(active.root) {
        apply_geom(&mut node, shown, active.resize);
    }
}

type AnyHandle = Or<(With<DragHandle>, With<ResizeHandle>)>;

fn on_drag_end(
    drag: On<Pointer<DragEnd>>,
    handles: Query<(), AnyHandle>,
    is_root: Query<(), With<DragRoot>>,
    parents: Query<&ChildOf>,
    mut lock: ResMut<CursorLock>,
    mut state: ResMut<DragState>,
    mut commands: Commands,
) {
    lock.0 = None;
    state.0 = None;
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
    dragged: Res<Dragged>,
    mut commands: Commands,
) {
    if dragged.0 {
        return;
    }
    let Some(root) = nearest_root(click.entity, &parents, &is_root) else {
        return;
    };
    let Ok(tap) = taps.get(root) else {
        return;
    };
    let tap = tap.0.clone();
    commands.queue(move |world: &mut World| tap(world));
}

/// Keeps panels at the same fractional spot when the viewport changes size, so they don't drift into
/// a corner or off-screen on a resize.
fn scale_on_resize(
    window: Single<&Window, With<PrimaryWindow>>,
    mut last: ResMut<LastViewport>,
    mut roots: Query<&mut Node, With<DragRoot>>,
) {
    let size = Vec2::new(window.resolution.width(), window.resolution.height());
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    if last.0 == Vec2::ZERO || last.0 == size {
        last.0 = size;
        return;
    }
    let ratio = size / last.0;
    last.0 = size;
    for mut node in &mut roots {
        node.left = Val::Px(px(node.left) * ratio.x);
        node.top = Val::Px(px(node.top) * ratio.y);
    }
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
    let geom = world.get::<Node>(root).map(read_geom).unwrap_or(Geom {
        pos: Vec2::ZERO,
        size: Vec2::ZERO,
    });
    let settled = (settle.0)(world, geom);
    write_geom(world, root, settled);
}

fn snapped(raw: Geom, grid: f32, resize: bool, min: Vec2) -> Geom {
    let snap = |value: f32| {
        if grid > 0.0 {
            (value / grid).round() * grid
        } else {
            value
        }
    };
    if resize {
        Geom {
            pos: raw.pos,
            size: Vec2::new(snap(raw.size.x).max(min.x), snap(raw.size.y).max(min.y)),
        }
    } else {
        Geom {
            pos: Vec2::new(snap(raw.pos.x), snap(raw.pos.y)),
            size: raw.size,
        }
    }
}

fn apply_geom(node: &mut Node, geom: Geom, resize: bool) {
    if resize {
        node.width = Val::Px(geom.size.x);
        node.height = Val::Px(geom.size.y);
    } else {
        node.left = Val::Px(geom.pos.x);
        node.top = Val::Px(geom.pos.y);
    }
}

fn read_geom(node: &Node) -> Geom {
    Geom {
        pos: Vec2::new(px(node.left), px(node.top)),
        size: Vec2::new(px(node.width), px(node.height)),
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
