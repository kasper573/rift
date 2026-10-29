use driver::{content, initial_spawn, props_near};
use engine::core::math::Offset;
use engine::systems::area;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "camera",
    title: "Camera",
    summary: "Turn, zoom and tilt the view — and it never loses sight of you",
    play,
};

fn play(shoot: &mut Shoot) {
    let (above, under) = by_something_in_the_way();
    let player = shoot.join_as("swordsman", Some(above));
    let mut me = shoot.roll(player);
    me.come_to_rest();
    me.caption("The camera looks down on you from the south, pitched like a tabletop; the compass in the corner points north");
    me.hold();
    me.caption("Turn it a step at a time with the arrow keys — the compass turns with it, and the controls stay as they are");
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    me.hold();
    me.turn_camera(InputAction::TurnLeft);
    me.hold();
    me.caption("Zoom out wide with the wheel, or back in close");
    me.zoom_camera(-2);
    me.hold();
    me.caption("Page Up tilts the camera toward the horizon, to see the land as a walker would");
    me.zoom_camera(2);
    for _ in 0..4 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.hold();
    me.caption("Page Down tilts it back toward the map");
    me.tilt_camera(InputAction::TiltDown);
    me.tilt_camera(InputAction::TiltDown);
    me.hold();
    me.caption("Home brings the camera back to looking north and down from the middle distance");
    me.camera_home();
    me.hold();
    me.close_in();
    me.caption("Whatever stands between you and the camera fades, so you are never out of sight");
    me.walk_near(under.at);
    me.hold();
    me.caption("Turn the camera and it is no longer in the way, so it comes back");
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    me.hold();
    me.close_in();
    me.hold();
}

/// By the prop nearest where players start that fades out of the way: a few steps north of it,
/// and right behind it, where it stands between a walker and a camera looking north.
fn by_something_in_the_way() -> (Place, Place) {
    let start = initial_spawn();
    let mesh = &area::load(content(), start.area).mesh;
    let place = |at| Place {
        area: start.area,
        at,
    };
    props_near(start, |prop| prop.fade)
        .into_iter()
        .map(|at| (at + Offset::new(0.0, -6.0), at + Offset::new(0.0, -3.0)))
        .find(|(above, under)| {
            mesh.walkable(*above) && mesh.walkable(*under) && mesh.connects(*above, *under)
        })
        .map(|(above, under)| (place(above), place(under)))
        .expect("something that fades out of the way, with open ground north of it")
}
