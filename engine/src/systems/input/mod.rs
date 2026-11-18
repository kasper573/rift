pub mod camera;
pub mod claims;
pub(crate) mod escape;
pub mod map;
pub mod marks;
pub mod scope;

use bevy::prelude::*;

use crate::systems::messages::messages;
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

messages! {
    /// What the game says of keys, buttons and the actions bound to them.
    pub struct InputMessages {
        /// The name of the Control key, as a key cap and a screen reader say it.
        ctrl,
        /// The name of the Alt key, as a key cap and a screen reader say it.
        alt,
        /// The name of the Shift key, as a key cap and a screen reader say it.
        shift,
        /// The name of the Super key, as a key cap and a screen reader say it.
        super_key,
        /// The name of the Escape key on its key cap.
        escape,
        /// The name of the space bar on its key cap.
        space,
        /// The name of the Enter key on its key cap.
        enter,
        /// The name of the Tab key on its key cap.
        tab,
        /// The name of the Backspace key on its key cap.
        backspace,
        /// The name of the Delete key on its key cap.
        delete,
        /// The name of the Insert key on its key cap.
        insert,
        /// The name of the Home key on its key cap.
        home,
        /// The name of the End key on its key cap.
        end,
        /// The name of the Page Up key on its key cap.
        page_up,
        /// The name of the Page Down key on its key cap.
        page_down,
        /// The name of the Caps Lock key on its key cap.
        caps_lock,
        /// What an action that any key does is bound to, on its cap and as a screen reader says it.
        any_key,
        /// The name of a turn of the mouse wheel away from the player's hand.
        wheel_up,
        /// The name of a turn of the mouse wheel toward the player's hand.
        wheel_down,
        /// How the left mouse button is named in each gesture.
        left(gesture: Text),
        /// How the right mouse button is named in each gesture.
        right(gesture: Text),
        /// How the middle mouse button is named in each gesture.
        middle(gesture: Text),
        /// How the mouse's back button is named in each gesture.
        back(gesture: Text),
        /// How the mouse's forward button is named in each gesture.
        forward(gesture: Text),
        /// How a mouse button with no name of its own, number `{number}`, is named in each gesture.
        other(gesture: Text, number: Number),
    }
}
