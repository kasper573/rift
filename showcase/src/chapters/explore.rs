use engine::core::math::Offset;
use engine::systems::input::map::InputAction;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "explore",
    title: "Explore",
    summary: "Point and click to roam the island",
    play,
};

const ROUTE: [((f32, f32), &str); 4] = [
    (
        (4.0, 1.0),
        "Click the ground to walk there — the camera follows, and the grass parts underfoot",
    ),
    (
        (5.0, -2.0),
        "Paths route around water, rocks and anything in the way",
    ),
    (
        (-3.0, -4.0),
        "Beaches, docks and palm groves — the island is open to explore",
    ),
    (
        (0.0, 0.0),
        "Town is safe ground: monsters roam the wilds beyond it, and fight back",
    ),
];

fn play(shoot: &mut Shoot) {
    let player = shoot.join_fresh();
    let mut me = shoot.roll(player);
    let spawn = me.come_to_rest();
    for ((x, y), says) in ROUTE {
        me.caption(says);
        me.walk_near(spawn + Offset::new(x, y));
    }
    me.caption("Turn the camera a step at a time with the arrow keys");
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    me.caption("Zoom in close with the wheel, or out wide");
    me.zoom_camera(1);
    me.zoom_camera(-2);
    me.caption("Page Up tilts the camera toward the horizon, to see the land as a walker would");
    me.zoom_camera(2);
    for _ in 0..4 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.caption("Home brings the camera back to looking north and down");
    me.camera_home();
}
