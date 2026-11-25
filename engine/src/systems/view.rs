use bevy::prelude::*;

use crate::core::audio::playback::Listener;
use crate::core::interpolate::{Jumps, RenderValueSystems};
use crate::core::math::Pos;
use crate::core::render::{CameraRig, CameraRigSystems, GroundHeights};
use crate::core::units::{Meters, Seconds};
use crate::systems::actor::Aloft;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

/// Time the follow camera takes to close half the gap to the player. Smaller is snappier, larger
/// floatier; this is what makes the camera glide after the actor rather than rigidly lock to it.
const CAMERA_HALF_LIFE: Seconds = Seconds(0.09);
/// How high above the player's feet the camera looks: about the chest of a standing character.
const FOCUS_HEIGHT: Meters = Meters(1.0);

pub struct CameraFollowPlugin;

impl Plugin for CameraFollowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FollowCenter>().add_systems(
            Update,
            track_player
                .after(RenderValueSystems)
                .in_set(CameraRigSystems)
                .run_if(in_state(ClientScreen::Area))
                .run_if(resource_exists::<CameraRig>),
        );
    }
}

/// Where the camera looks, eased toward the player each frame: the point on the ground and how
/// high the player's feet stand there, eased together, so a player put somewhere higher or lower,
/// as onto a boat, is followed there as smoothly as along the ground.
#[derive(Resource, Default)]
struct FollowCenter(Option<(Followed, Pos<Meters>, Meters)>);

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
    characters: Query<(&RenderPosition, &Position, &AreaTag, Option<&Aloft>)>,
    ground: Option<Res<GroundHeights>>,
    mut rig: ResMut<CameraRig>,
    mut listener: ResMut<Listener>,
    mut follow: ResMut<FollowCenter>,
) {
    let Some(seen) = viewpoint.0 else {
        return;
    };
    let Ok((render, server, tag, aloft)) = characters.get(seen) else {
        return;
    };
    let at = render.0;
    listener.0 = Some(at);
    let followed = Followed {
        entity: seen,
        area: tag.area,
        jumps: server.jumps,
    };
    let feet = aloft.map_or_else(
        || ground.map_or(Meters(0.0), |ground| ground.height_at(at)),
        |aloft| aloft.0,
    );
    let (center, underfoot) = match follow.0 {
        Some((before, prev, prev_feet)) if before == followed => {
            let caught = 1.0 - 0.5_f32.powf(time.delta_secs() / CAMERA_HALF_LIFE.0);
            (
                prev.lerp(at, caught),
                Meters(prev_feet.0 + (feet.0 - prev_feet.0) * caught),
            )
        }
        _ => (at, feet),
    };
    follow.0 = Some((followed, center, underfoot));
    rig.look_at(center, underfoot + FOCUS_HEIGHT);
}
