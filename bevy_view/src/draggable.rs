//! Composable drag behaviors built entirely on the public event substrate (`on_drag`/`on_click`/…)
//! plus a few private markers — the recipe a game would follow for its own behaviors.
//!
//! A panel is made movable with [`draggable`] and, optionally, resizable with [`resizable`]; both act
//! on the nearest [`DragRoot`] ancestor, so a title bar moves the window it lives in and a corner grip
//! resizes it. All persistent state lives on the entity (which the reconciler keeps identity-stable),
//! so the view never writes geometry and a dragged position survives re-renders.

use std::sync::Arc;

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityWorldMut;
use bevy_math::Vec2;
use bevy_ui::{Node, Val};

use crate::cursor::{CursorLock, HoverCursor};
use crate::view::{Bind, Element};

/// A panel's on-screen geometry in logical pixels: top-left position and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub pos: Vec2,
    pub size: Vec2,
}

type Tap = Arc<dyn Fn(&mut World) + Send + Sync>;
type Settle = Arc<dyn Fn(&mut World, Geom) -> Geom + Send + Sync>;

/// Marks the entity a handle moves/resizes: the nearest ancestor carrying this is the target.
#[derive(Component, Clone)]
struct DragRoot;

/// Per-target runtime state. `moved` lets a click tell itself apart from the tail of a drag; it is
/// seeded once and never overwritten by a re-render, so a live drag is never reset.
#[derive(Component, Default)]
struct DragState {
    moved: bool,
}

/// Marks a target whose initial geometry has been seeded, so seeding runs exactly once.
#[derive(Component)]
struct Seeded;

/// Per-target config, refreshed every render (latest `on_settle` wins). Read by the handles off the
/// target so they need not capture it themselves.
#[derive(Component, Clone, Default)]
struct DragConfig {
    settle: Option<Settle>,
}

/// A movable panel behavior. Wire [`root`](Draggable::root) onto the panel and a
/// [`handle`](Draggable::handle) onto whatever drives the move (or [`whole`](Draggable::whole) for a
/// self-dragging element).
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
    /// Top-left applied once, on first mount; afterwards the drag owns it.
    pub fn initial(mut self, pos: Vec2) -> Draggable {
        self.initial = Some(pos);
        self
    }

    /// Size applied once, on first mount — for a resizable panel whose size the resize grip then owns.
    pub fn initial_size(mut self, size: Vec2) -> Draggable {
        self.initial_size = Some(size);
        self
    }

    /// A click that is *not* the tail of a drag — wire the panel's real action here.
    pub fn on_tap(mut self, handler: impl Fn(&mut World) + Send + Sync + 'static) -> Draggable {
        self.tap = Some(Arc::new(handler));
        self
    }

    /// Runs when a drag (or resize) finishes, with the final geometry; return where to settle it
    /// (snap and persist here). Identity by default.
    pub fn on_settle(
        mut self,
        handler: impl Fn(&mut World, Geom) -> Geom + Send + Sync + 'static,
    ) -> Draggable {
        self.settle = Some(Arc::new(handler));
        self
    }

    /// Binds the movable target: seeds initial geometry once, holds the settle config, and runs the
    /// tap on a click that wasn't a drag.
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

    /// Binds a drag surface that moves the nearest [`root`](Draggable::root) ancestor.
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

    /// Sugar for a self-dragging element: [`root`](Draggable::root) and [`handle`](Draggable::handle)
    /// on the same node.
    pub fn whole(&self) -> Bind {
        let root = self.root();
        let handle = self.handle();
        Bind::new(move |element: Element| element.bind(root).bind(handle))
    }
}

/// A resize behavior. Its [`handle`](Resizable::handle) grows/shrinks the nearest movable
/// [`DragRoot`], clamped to a minimum, and persists through that root's `on_settle`.
#[derive(Default)]
pub struct Resizable {
    min: Vec2,
}

pub fn resizable() -> Resizable {
    Resizable::default()
}

impl Resizable {
    /// The smallest the target may be resized to.
    pub fn min(mut self, min: Vec2) -> Resizable {
        self.min = min;
        self
    }

    /// Binds a resize surface (e.g. a corner grip) that resizes the nearest root while dragged.
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
        // Only commit size onto an explicitly-sized (resizable) panel; an auto-sized widget keeps its
        // intrinsic size rather than being collapsed to its measured pixels.
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
