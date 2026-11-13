pub mod camera;
pub mod claims;
mod escape;
pub mod map;
pub mod marks;
pub mod scope;

use bevy::prelude::*;

use crate::systems::player::RespawnRequest;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

pub struct PlayerInputPlugin;

impl Plugin for PlayerInputPlugin {
    fn build(&self, app: &mut App) {
        map::plugin(app);
        scope::plugin(app);
        claims::plugin(app);
        marks::plugin(app);
        camera::plugin(app);
        escape::plugin(app);
        app.add_systems(
            Update,
            respawn_when_dead.run_if(in_state(ClientScreen::Area)),
        );
    }
}

fn respawn_when_dead(world: &mut World) {
    if session::is_dead(world) && map::just_pressed(world, map::InputAction::Respawn) {
        world.write_message(RespawnRequest);
    }
}
