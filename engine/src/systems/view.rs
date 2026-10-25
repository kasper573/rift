use bevy::prelude::*;

use crate::core::audio::playback::Listener;
use crate::core::interpolate::{Jumps, RenderValueSystems};
use crate::core::math::Pos;
use crate::core::render::CameraRig;
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

/// Time the follow camera takes to close half the gap to the player. Smaller is snappier, larger
/// floatier; this is what makes the camera glide after the actor rather than rigidly lock to it.
const CAMERA_HALF_LIFE: Seconds = Seconds(0.09);
/// How high above the ground under the player the camera looks: about the chest of a standing
/// character.
const FOCUS_HEIGHT: Meters = Meters(1.0);

pub struct CameraFollowPlugin;

impl Plugin for CameraFollowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FollowCenter>().add_systems(
            Update,
            track_player
                .after(RenderValueSystems)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

/// The camera's smoothed ground point, eased toward the player each frame.
#[derive(Resource, Default)]
struct FollowCenter(Option<(Followed, Pos<Meters>)>);

/// Who the camera follows, where: the camera glides after them only while this stays the same, and
/// cuts to them when it changes, as they jump, cross into another area, or someone else is followed.
#[derive(Clone, Copy, PartialEq)]
struct Followed {
    entity: Entity,
    area: AreaId,
    jumps: Jumps,
}

fn track_player(
    time: Res<Time>,
    viewpoint: Res<MyCharacter>,
    characters: Query<(&RenderPosition, &Position, &AreaTag)>,
    mut rig: ResMut<CameraRig>,
    mut listener: ResMut<Listener>,
    mut follow: ResMut<FollowCenter>,
) {
    let Some(seen) = viewpoint.0 else {
        return;
    };
    let Ok((render, server, tag)) = characters.get(seen) else {
        return;
    };
    let at = render.0;
    listener.0 = Some(at);
    let followed = Followed {
        entity: seen,
        area: tag.area,
        jumps: server.jumps,
    };
    let center = match follow.0 {
        Some((before, prev)) if before == followed => {
            let caught = 1.0 - 0.5_f32.powf(time.delta_secs() / CAMERA_HALF_LIFE.0);
            prev.lerp(at, caught)
        }
        _ => at,
    };
    follow.0 = Some((followed, center));
    rig.look_at(center, FOCUS_HEIGHT);
}
