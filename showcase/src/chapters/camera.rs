use driver::id;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
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

/// On the cottage path, a few steps north of the oak.
fn above_the_oak() -> Pos<Meters> {
    Pos::new(80.0, 39.0)
}

/// Where a walker stands with the oak between them and a camera looking north.
fn under_the_oak() -> Pos<Meters> {
    Pos::new(80.0, 42.0)
}

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as(
        "swordsman",
        Some(Place {
            area: id::<AreaDef>("island"),
            at: above_the_oak(),
        }),
    );
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
    me.walk_near(under_the_oak());
    me.hold();
    me.caption("Turn the camera and the tree is no longer in the way, so it comes back");
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    me.hold();
    me.close_in();
    me.hold();
}
