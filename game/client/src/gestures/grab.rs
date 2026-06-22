use bevy::picking::hover::HoverMap;
use bevy::prelude::*;

use crate::gestures::InputIntent;

/// Claims a press that lands on the HUD so the world ignores it; bevy's picking drives the actual
/// widget interaction, so there is nothing to do once it is claimed.
pub(super) struct Grab;

impl InputIntent for Grab {
    fn claims(&self, world: &mut World) -> bool {
        world
            .resource::<HoverMap>()
            .values()
            .flat_map(|hits| hits.keys())
            .any(|&entity| world.get::<Node>(entity).is_some())
    }

    fn drive(&self, _world: &mut World, _start: bool) {}
}
