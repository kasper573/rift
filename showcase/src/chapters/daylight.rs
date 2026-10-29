use std::time::Duration;

use driver::{DeployEdit, Player, WaitBudget, lamps_near};
use engine::core::render::world::sky::Daylight;
use engine::core::time::Seconds;
use engine::systems::area::DayCycle;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "daylight",
    title: "Day and night",
    summary: "The sun crosses the sky and sets, the moon and the stars light the night, and dawn breaks",
    play,
};

// A day short enough to watch whole.
const DAY: DayCycle = DayCycle {
    day_length: Seconds(120.0),
    start: 13.0,
};

const A_DAY: WaitBudget = WaitBudget {
    time: Duration::from_secs(DAY.day_length.0 as u64),
    frames: 1200,
};

fn play(shoot: &mut Shoot) {
    shoot.deploy(&[DeployEdit::game(|game| game.daylight = Some(DAY))], || {});
    let mut player = shoot.join_fresh();
    wait_for_hours(&mut player, "the early afternoon", 12.5..14.5);
    let mut me = shoot.roll(player);
    me.come_to_rest();
    for _ in 0..5 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.caption("A game's day turns at the pace it sets; this one passes in two minutes, so watch the light move");
    me.caption("Clouds drift across the sky, and their shadows across the ground beneath them");
    me.hold();
    me.turn_camera(InputAction::TurnLeft);
    me.turn_camera(InputAction::TurnLeft);
    wait_for_hours(&mut me, "the evening", 17.0..19.0);
    me.caption(
        "Toward evening the sun sinks west and reddens, and the light turns golden and long",
    );
    wait_for_hours(&mut me, "nightfall", 19.5..24.0);
    me.caption("By night the full moon stands opposite the sun, lighting the world faintly blue, and the stars come out");
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    wait_for_hours(&mut me, "midnight", 23.0..24.0);
    let seen = me.snapshot();
    let here = Place {
        area: seen.area.expect("the player is in an area"),
        at: seen.me.as_ref().expect("the player").at,
    };
    if let Some(lamp) = lamps_near(here).first() {
        me.walk_near(*lamp);
        me.caption(
            "An area's lamps pool their light on the ground around them, and shine out best by night",
        );
        me.hold();
    }
    wait_for_hours(&mut me, "the small hours", 4.5..12.0);
    me.turn_camera(InputAction::TurnRight);
    me.turn_camera(InputAction::TurnRight);
    me.caption("Toward dawn the stars fade and the east brightens, until the sun climbs back up");
    wait_for_hours(&mut me, "the morning", 7.5..12.0);
    me.hold();
}

fn wait_for_hours(me: &mut Player, what: &str, hours: std::ops::Range<f32>) {
    me.wait_within(what, A_DAY, |snapshot| match snapshot.daylight {
        Daylight::Hour(hour) if hours.contains(&hour) => Some(()),
        _ => None,
    });
}
