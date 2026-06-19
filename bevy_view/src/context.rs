//! React-style context, implemented over the entity hierarchy: [`provide`] attaches a typed value to
//! an element, and [`context`] resolves the nearest providing ancestor by walking `ChildOf`. This is
//! the ambient-scope mechanism overlays use for their `Provider` config, and it is available to games.

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;

use crate::view::Bind;

/// A context value of type `T` provided by an element to its descendants.
#[derive(Component, Clone)]
struct Context<T: Send + Sync + 'static>(T);

/// Provides a context value of type `T` to the subtree under this element. Apply with `use={…}`.
/// A descendant reads it with [`context`]. An inner provider shadows an outer one of the same type.
pub fn provide<T: Clone + Send + Sync + 'static>(value: T) -> Bind {
    Bind::new(move |element| element.insert(Context(value)))
}

/// Resolves the value of the nearest `T` provided at or above `entity`, or `None` if none is in
/// scope. Walks the entity's `ChildOf` ancestry.
pub fn context<T: Send + Sync + 'static>(world: &World, entity: Entity) -> Option<&T> {
    let mut current = entity;
    loop {
        if let Some(context) = world.get::<Context<T>>(current) {
            return Some(&context.0);
        }
        current = world.get::<ChildOf>(current)?.parent();
    }
}
