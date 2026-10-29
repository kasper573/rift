use bevy::prelude::*;
use engine::core::geometry::Degrees;
use engine::core::render::world::sky::{Daylight, ambient_at, night, shining, sun_at};
use engine::core::time::{Seconds, UnixMillis};
use engine::systems::area::DayCycle;
use engine::systems::launch::ServerDaylight;

const NOON: (Degrees, Degrees) = (Degrees(52.0), Degrees(215.0));

fn luminance(color: Color) -> f32 {
    let linear = color.to_linear();
    0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue
}

#[test]
fn the_sun_the_moon_and_ambient_light_turn_with_the_hour_and_dim_under_cloud() {
    let white = Color::WHITE;
    let fixed = sun_at(Daylight::Fixed, NOON.0, NOON.1, white, 0.0, 0.0);
    assert_eq!(
        (fixed.elevation, fixed.azimuth, fixed.color),
        (NOON.0, NOON.1, white),
        "without a clock the sun stands as authored"
    );

    let noon = sun_at(Daylight::Hour(12.0), NOON.0, NOON.1, white, 0.0, 0.0);
    assert!(
        (noon.elevation.0 - NOON.0.0).abs() < 0.01 && (noon.azimuth.0 - NOON.1.0).abs() < 0.01,
        "at noon the sun stands where it was authored: {noon:?}"
    );
    assert_eq!(noon.illuminance, fixed.illuminance, "noon is full daylight");
    let morning = sun_at(Daylight::Hour(9.0), NOON.0, NOON.1, white, 0.0, 0.0);
    assert!(
        morning.elevation.0 > 0.0 && morning.elevation.0 < NOON.0.0 && morning.azimuth < NOON.1,
        "in the morning the sun is lower and further east: {morning:?}"
    );
    let dusk = sun_at(Daylight::Hour(18.3), NOON.0, NOON.1, white, 0.0, 0.0);
    assert!(
        dusk.elevation.0 < 2.0 && luminance(dusk.color) < luminance(white),
        "at dusk the sun sits on the horizon, reddened: {dusk:?}"
    );
    let midnight = sun_at(Daylight::Hour(0.0), NOON.0, NOON.1, white, 0.0, 0.0);
    assert!(
        midnight.elevation.0 < 0.0 && midnight.illuminance == 0.0,
        "at midnight the sun is below the horizon and gives no light: {midnight:?}"
    );
    assert!(
        ambient_at(Daylight::Hour(0.0), Some(midnight.elevation), 0.0, 0.0)
            < ambient_at(Daylight::Hour(12.0), Some(noon.elevation), 0.0, 0.0),
        "the ambient light dims by night"
    );

    assert_eq!(
        shining(noon, 0.0, 0.0),
        noon,
        "by day the sun lights the world"
    );
    let moon = shining(midnight, 0.0, 0.0);
    assert!(
        (moon.elevation.0 + midnight.elevation.0).abs() < 0.01
            && ((moon.azimuth.0 - midnight.azimuth.0).rem_euclid(360.0) - 180.0).abs() < 0.01,
        "by night the full moon stands opposite the sun: {moon:?} to {midnight:?}"
    );
    assert!(
        moon.illuminance > 0.0 && moon.illuminance < noon.illuminance / 2.0,
        "the moon lights the night, far dimmer than the sun: {moon:?}"
    );
    assert_eq!(
        (
            night(Daylight::Fixed, fixed),
            night(Daylight::Hour(12.0), noon),
            night(Daylight::Hour(0.0), midnight)
        ),
        (0.0, 0.0, 1.0),
        "the stars come out only at night, never under a fixed light"
    );

    let clouded = sun_at(Daylight::Hour(12.0), NOON.0, NOON.1, white, 1.0, 0.0);
    assert!(
        clouded.illuminance < noon.illuminance,
        "fair-weather cloud dims the sun"
    );
    let stormy = sun_at(Daylight::Hour(12.0), NOON.0, NOON.1, white, 1.0, 1.0);
    assert!(
        stormy.illuminance < clouded.illuminance && stormy.illuminance < noon.illuminance / 2.0,
        "storm cloud takes most of it"
    );
}

#[test]
fn a_day_turns_from_the_games_start_hour_or_holds_there_on_a_still_server() {
    let cycle = DayCycle {
        day_length: Seconds(1200.0),
        start: 10.0,
    };
    let started = UnixMillis(1_800_000_000_000);
    let later = |seconds: f32| started.after(Seconds(seconds));

    let turning = cycle.course(ServerDaylight::Turning, started);
    let hours = [0.0, 300.0, 1200.0 * 3.0 + 600.0].map(|at| cycle.hour(turning, later(at)));
    for (hour, expected) in hours.into_iter().zip([10.0, 16.0, 22.0]) {
        assert!(
            (hour - expected).abs() < 0.01,
            "a turning day starts at the game's start hour and turns once a day length: {hours:?}"
        );
    }

    let still = cycle.course(ServerDaylight::Still, started);
    assert_eq!(
        [0.0, 300.0, 1200.0 * 3.0 + 600.0].map(|at| cycle.hour(still, later(at))),
        [10.0; 3],
        "a still day holds at the game's start hour however long the world runs"
    );
}
