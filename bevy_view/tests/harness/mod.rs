#![allow(dead_code)]

use bevy_app::App;
use bevy_ecs::hierarchy::Children;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_ui::Node;
use bevy_ui::prelude::Text;
use bevy_view::{View, ViewPlugin, render};

#[derive(Resource, Default)]
pub struct Log(pub Vec<String>);

pub fn log(world: &mut World, message: impl Into<String>) {
    world.resource_mut::<Log>().0.push(message.into());
}

pub struct Ui {
    app: App,
    host: Entity,
}

impl Ui {
    pub fn new() -> Ui {
        let mut app = App::new();
        app.add_plugins(ViewPlugin);
        app.init_resource::<Log>();
        let host = app.world_mut().spawn(Node::default()).id();
        Ui { app, host }
    }

    pub fn host(&self) -> Entity {
        self.host
    }

    pub fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

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

    pub fn child_count(&self) -> usize {
        self.children().len()
    }

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

    pub fn texts_under(&self, entity: Entity) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_text(entity, &mut out);
        out
    }

    pub fn text_of(&self, entity: Entity) -> Option<String> {
        self.app
            .world()
            .get::<Text>(entity)
            .map(|text| text.0.clone())
    }

    pub fn log(&self) -> Vec<String> {
        self.app.world().resource::<Log>().0.clone()
    }

    pub fn clear_log(&mut self) {
        self.app.world_mut().resource_mut::<Log>().0.clear();
    }

    pub fn activate_click(&mut self, entity: Entity) {
        bevy_view::activate_click(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn activate_drag(&mut self, entity: Entity, delta: Vec2) {
        bevy_view::activate_drag(self.app.world_mut(), entity, delta);
        self.app.world_mut().flush();
    }

    pub fn activate_drag_end(&mut self, entity: Entity) {
        bevy_view::activate_drag_end(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn activate_over(&mut self, entity: Entity) {
        bevy_view::activate_over(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn activate_out(&mut self, entity: Entity) {
        bevy_view::activate_out(self.app.world_mut(), entity);
        self.app.world_mut().flush();
    }

    pub fn has<C: Component>(&self, entity: Entity) -> bool {
        self.app.world().get::<C>(entity).is_some()
    }

    pub fn get<C: Component + Clone>(&self, entity: Entity) -> Option<C> {
        self.app.world().get::<C>(entity).cloned()
    }

    pub fn find<C: Component>(&self, root: Entity) -> Option<Entity> {
        if self.app.world().get::<C>(root).is_some() && root != self.host {
            return Some(root);
        }
        for child in self.children_of(root) {
            if let Some(found) = self.find::<C>(child) {
                return Some(found);
            }
        }
        None
    }
}
