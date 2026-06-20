use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;

use crate::view::Bind;

#[derive(Component, Clone)]
struct Context<T: Send + Sync + 'static>(T);

pub fn provide<T: Clone + Send + Sync + 'static>(value: T) -> Bind {
    Bind::new(move |element| element.insert(Context(value)))
}

pub fn context<T: Send + Sync + 'static>(world: &World, entity: Entity) -> Option<&T> {
    let mut current = entity;
    loop {
        if let Some(context) = world.get::<Context<T>>(current) {
            return Some(&context.0);
        }
        current = world.get::<ChildOf>(current)?.parent();
    }
}
