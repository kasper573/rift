pub mod actor;
pub mod combat;
pub mod debug;
pub mod fps;
pub mod input;
pub mod item;
pub mod scene;
pub mod session;
pub mod testing;
pub mod view;
pub mod widget;

use bevy::prelude::*;

pub(crate) fn despawn_all<M: Component>(entities: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}
