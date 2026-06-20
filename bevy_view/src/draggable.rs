use std::sync::Arc;

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityWorldMut;
use bevy_math::Vec2;
use bevy_ui::{Node, Val};

use crate::cursor::{CursorLock, HoverCursor};
use crate::view::{Bind, Element};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub pos: Vec2,
    pub size: Vec2,
}

type Tap = Arc<dyn Fn(&mut World) + Send + Sync>;
type Settle = Arc<dyn Fn(&mut World, Geom) -> Geom + Send + Sync>;

#[derive(Component, Clone)]
struct DragRoot;

/// `moved` lets a click distinguish itself from the tail of a drag; seeded once and never overwritten
/// by a re-render, so a live drag is never reset.
#[derive(Component, Default)]
struct DragState {
    moved: bool,
}

/// Marks a target whose initial geometry has been seeded, so seeding runs exactly once.
#[derive(Component)]
struct Seeded;

/// Latest `on_settle` wins, so handles read this instead of capturing it themselves.
#[derive(Component, Clone, Default)]
struct DragConfig {
    settle: Option<Settle>,
}

#[derive(Default)]
pub struct Draggable {
    initial: Option<Vec2>,
    initial_size: Option<Vec2>,
    tap: Option<Tap>,
    settle: Option<Settle>,
}

pub fn draggable() -> Draggable {
    Draggable::default()
}

impl Draggable {
    /// Applied once on first mount; afterwards the drag owns it.
    pub fn initial(mut self, pos: Vec2) -> Draggable {
        self.initial = Some(pos);
        self
    }

    /// Applied once on first mount — for a resizable panel whose size the resize grip then owns.
    pub fn initial_size(mut self, size: Vec2) -> Draggable {
        self.initial_size = Some(size);
        self
    }

    pub fn on_tap(mut self, handler: impl Fn(&mut World) + Send + Sync + 'static) -> Draggable {
        self.tap = Some(Arc::new(handler));
        self
    }

    pub fn on_settle(
        mut self,
        handler: impl Fn(&mut World, Geom) -> Geom + Send + Sync + 'static,
    ) -> Draggable {
        self.settle = Some(Arc::new(handler));
        self
    }

    pub fn root(&self) -> Bind {
        let initial = self.initial;
        let initial_size = self.initial_size;
        let settle = self.settle.clone();
        let tap = self.tap.clone();
        Bind::new(move |element: Element| {
            element
                .insert(DragRoot)
                .attr(move |entity| {
                    entity.insert(DragConfig {
                        settle: settle.clone(),
                    });
                    if !entity.contains::<Seeded>() {
                        seed(entity, initial, initial_size);
                        entity.insert((Seeded, DragState::default()));
                    }
                })
                .on_click_with(move |world, entity| {
                    if !take_moved(world, entity)
                        && let Some(tap) = &tap
                    {
                        tap(world);
                    }
                })
        })
    }

    pub fn handle(&self) -> Bind {
        Bind::new(|element: Element| {
            element
                .on_drag_with(|world, entity, delta| {
                    if let Some(root) = nearest_root(world, entity) {
                        move_by(world, root, delta);
                    }
                })
                .on_drag_end_with(|world, entity| {
                    if let Some(root) = nearest_root(world, entity) {
                        settle(world, root);
                    }
                })
        })
    }

    pub fn whole(&self) -> Bind {
        let root = self.root();
        let handle = self.handle();
        Bind::new(move |element: Element| element.bind(root).bind(handle))
    }
}

#[derive(Default)]
pub struct Resizable {
    min: Vec2,
}

pub fn resizable() -> Resizable {
    Resizable::default()
}

impl Resizable {
    pub fn min(mut self, min: Vec2) -> Resizable {
        self.min = min;
        self
    }

    pub fn handle(&self) -> Bind {
        let min = self.min;
        Bind::new(move |element: Element| {
            element
                .on_drag_with(move |world, entity, delta| {
                    if let Some(root) = nearest_root(world, entity) {
                        resize_by(world, root, delta, min);
                    }
                    lock_cursor_from(world, entity);
                })
                .on_drag_end_with(|world, entity| {
                    unlock_cursor(world);
                    if let Some(root) = nearest_root(world, entity) {
                        settle(world, root);
                    }
                })
        })
    }
}

fn seed(entity: &mut EntityWorldMut, pos: Option<Vec2>, size: Option<Vec2>) {
    if let Some(mut node) = entity.get_mut::<Node>() {
        if let Some(pos) = pos {
            node.left = Val::Px(pos.x);
            node.top = Val::Px(pos.y);
        }
        if let Some(size) = size {
            node.width = Val::Px(size.x);
            node.height = Val::Px(size.y);
        }
    }
}

fn nearest_root(world: &World, mut entity: Entity) -> Option<Entity> {
    loop {
        if world.get::<DragRoot>(entity).is_some() {
            return Some(entity);
        }
        entity = world.get::<ChildOf>(entity)?.parent();
    }
}

fn move_by(world: &mut World, root: Entity, delta: Vec2) {
    if let Some(mut node) = world.get_mut::<Node>(root) {
        node.left = Val::Px(px(node.left) + delta.x);
        node.top = Val::Px(px(node.top) + delta.y);
    }
    if let Some(mut state) = world.get_mut::<DragState>(root) {
        state.moved = true;
    }
}

fn resize_by(world: &mut World, root: Entity, delta: Vec2, min: Vec2) {
    if let Some(mut node) = world.get_mut::<Node>(root) {
        node.width = Val::Px((px(node.width) + delta.x).max(min.x));
        node.height = Val::Px((px(node.height) + delta.y).max(min.y));
    }
}

fn settle(world: &mut World, root: Entity) {
    let Some(settle) = world.get::<DragConfig>(root).and_then(|c| c.settle.clone()) else {
        return;
    };
    let geom = read_geom(world, root);
    let settled = settle(world, geom);
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
        // Auto-sized widgets keep intrinsic size; only resizable (explicit Px) panels commit size.
        if matches!(node.width, Val::Px(_)) {
            node.width = Val::Px(geom.size.x);
        }
        if matches!(node.height, Val::Px(_)) {
            node.height = Val::Px(geom.size.y);
        }
    }
}

fn take_moved(world: &mut World, entity: Entity) -> bool {
    if let Some(mut state) = world.get_mut::<DragState>(entity) {
        let was = state.moved;
        state.moved = false;
        was
    } else {
        false
    }
}

fn lock_cursor_from(world: &mut World, entity: Entity) {
    let icon = world
        .get::<HoverCursor>(entity)
        .map(|cursor| cursor.0.clone());
    if let Some(mut lock) = world.get_resource_mut::<CursorLock>() {
        lock.0 = icon;
    }
}

fn unlock_cursor(world: &mut World) {
    if let Some(mut lock) = world.get_resource_mut::<CursorLock>() {
        lock.0 = None;
    }
}

fn px(val: Val) -> f32 {
    match val {
        Val::Px(value) => value,
        _ => 0.0,
    }
}
