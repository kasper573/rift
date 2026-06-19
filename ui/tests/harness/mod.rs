//! Shared scaffolding for the `ui` component-library spec. Drives the reconciler from an external
//! author's perspective: build a [`View`], render it against a host, then assert on the resulting
//! `bevy_ui` tree (`Children`, `Text`, component values) — never on library internals. The app wires
//! `bevy_view`'s [`ViewPlugin`] and the library's [`UiPlugin`] so overlay state (the `Overlays`
//! resource, the dismissal observer) is registered exactly as in a real client.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use bevy_app::App;
use bevy_ecs::hierarchy::Children;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_ui::Node;
use bevy_ui::prelude::Text;
use bevy_view::{View, ViewPlugin, render};
use ui::UiPlugin;

/// App-owned state for a controlled component: the test holds the value, passes a snapshot in as a
/// prop, and the component's change callback writes the next value back here. Cloneable and thread-safe
/// so a `view!` builder can read it and a handler closure can capture it.
#[derive(Clone)]
pub struct State<T>(Arc<Mutex<T>>);

impl<T: Clone> State<T> {
    pub fn new(value: T) -> State<T> {
        State(Arc::new(Mutex::new(value)))
    }

    pub fn get(&self) -> T {
        self.0.lock().unwrap().clone()
    }

    pub fn set(&self, value: T) {
        *self.0.lock().unwrap() = value;
    }
}

pub struct Ui {
    app: App,
    host: Entity,
}

impl Ui {
    pub fn new() -> Ui {
        let mut app = App::new();
        app.add_plugins((ViewPlugin, UiPlugin));
        let host = app.world_mut().spawn(Node::default()).id();
        Ui { app, host }
    }

    pub fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

    /// Renders `view` against the host, then flushes so mount/cleanup commands take effect.
    pub fn render(&mut self, view: impl Into<View>) {
        let host = self.host;
        render(self.app.world_mut(), host, view.into());
        self.app.world_mut().flush();
    }

    pub fn children_of(&self, parent: Entity) -> Vec<Entity> {
        self.app
            .world()
            .get::<Children>(parent)
            .map(|children| children.iter().collect())
            .unwrap_or_default()
    }

    pub fn children(&self) -> Vec<Entity> {
        self.children_of(self.host)
    }

    /// Every `Text` value in pre-order traversal of the host subtree.
    pub fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_text(self.host, &mut out);
        out
    }

    fn collect_text(&self, entity: Entity, out: &mut Vec<String>) {
        if let Some(text) = self.app.world().get::<Text>(entity) {
            out.push(text.0.clone());
        }
        for child in self.children_of(entity) {
            self.collect_text(child, out);
        }
    }

    pub fn activate_click(&mut self, entity: Entity) {
        bevy_view::activate_click(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    /// Advances time past an overlay's exit window and runs the close pass, so a requested close (which
    /// is deferred while the content eases out) actually completes.
    pub fn settle(&mut self) {
        let world = self.app.world_mut();
        if world.get_resource::<Time>().is_none() {
            world.insert_resource(Time::<()>::default());
        }
        world
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(400));
        ui::advance_overlay_close(world);
        world.flush();
    }

    pub fn activate_over(&mut self, entity: Entity) {
        bevy_view::activate_over(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn activate_drag(&mut self, entity: Entity, delta: bevy_math::Vec2) {
        bevy_view::activate_drag(self.app.world_mut(), entity, delta);
        self.app.world_mut().flush();
    }

    pub fn activate_out(&mut self, entity: Entity) {
        bevy_view::activate_out(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn get<C: Component + Clone>(&self, entity: Entity) -> Option<C> {
        self.app.world().get::<C>(entity).cloned()
    }

    /// Advances the virtual clock, then opens any tooltip whose delay has now elapsed (the timing the
    /// `UiPlugin` runs each frame in a real app).
    pub fn advance(&mut self, by: std::time::Duration) {
        self.app
            .world_mut()
            .resource_mut::<bevy_time::Time>()
            .advance_by(by);
        ui::open_due_tooltips(self.app.world_mut());
        self.app.world_mut().flush();
    }

    /// Installs a virtual clock so tooltip timing can be driven deterministically.
    pub fn with_clock(mut self) -> Ui {
        self.app
            .world_mut()
            .insert_resource(bevy_time::Time::<()>::default());
        self
    }

    /// Advances the virtual clock by `by` and runs the app's schedules once, so time-driven systems —
    /// notably the motion engine's tween integrator — step. Needs [`with_clock`](Ui::with_clock).
    pub fn tick(&mut self, by: std::time::Duration) {
        self.app
            .world_mut()
            .resource_mut::<bevy_time::Time>()
            .advance_by(by);
        self.app.update();
    }
}
