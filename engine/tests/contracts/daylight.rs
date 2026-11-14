use bevy::prelude::*;
use engine::core::content::ImageRef;
use engine::core::math::Rgb;
use engine::core::render::world::sky::{Daylight, ambient_at, night, shining, sun_at};
use engine::core::time::UnixMillis;
use engine::core::units::{DayHours, Degrees, HourOfDay, Seconds};
use engine::systems::area::{DayCycle, TimeOfDay, TimeOfDayName};
use engine::systems::hud::day_clock;
use engine::systems::launch::{ServerDay, ServerDaylight};
use ui::egui;

const NOON: (Degrees, Degrees) = (Degrees(52.0), Degrees(215.0));

fn luminance(color: Color) -> f32 {
    let linear = color.to_linear();
    0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue
}

#[test]
fn the_sun_the_moon_and_ambient_light_turn_with_the_hour_and_dim_under_cloud() {
    let white = Color::WHITE;
    let look = testkit::look();
    let fixed = sun_at(Daylight::Fixed, NOON.0, NOON.1, white, 0.0, 0.0, &look);
    assert_eq!(
        (fixed.elevation, fixed.azimuth, fixed.color),
        (NOON.0, NOON.1, white),
        "without a clock the sun stands as authored"
    );

    let noon = sun_at(
        Daylight::Hour(HourOfDay(12.0)),
        NOON.0,
        NOON.1,
        white,
        0.0,
        0.0,
        &look,
    );
    assert!(
        (noon.elevation.0 - NOON.0.0).abs() < 0.01 && (noon.azimuth.0 - NOON.1.0).abs() < 0.01,
        "at noon the sun stands where it was authored: {noon:?}"
    );
    assert_eq!(noon.illuminance, fixed.illuminance, "noon is full daylight");
    let morning = sun_at(
        Daylight::Hour(HourOfDay(9.0)),
        NOON.0,
        NOON.1,
        white,
        0.0,
        0.0,
        &look,
    );
    assert!(
        morning.elevation.0 > 0.0 && morning.elevation.0 < NOON.0.0 && morning.azimuth < NOON.1,
        "in the morning the sun is lower and further east: {morning:?}"
    );
    let dusk = sun_at(
        Daylight::Hour(HourOfDay(18.3)),
        NOON.0,
        NOON.1,
        white,
        0.0,
        0.0,
        &look,
    );
    assert!(
        dusk.elevation.0 < 2.0 && luminance(dusk.color) < luminance(white),
        "at dusk the sun sits on the horizon, reddened: {dusk:?}"
    );
    let midnight = sun_at(
        Daylight::Hour(HourOfDay(0.0)),
        NOON.0,
        NOON.1,
        white,
        0.0,
        0.0,
        &look,
    );
    assert!(
        midnight.elevation.0 < 0.0 && midnight.illuminance == 0.0,
        "at midnight the sun is below the horizon and gives no light: {midnight:?}"
    );
    assert!(
        ambient_at(
            Daylight::Hour(HourOfDay(0.0)),
            Some(midnight.elevation),
            0.0,
            0.0,
            &look
        ) < ambient_at(
            Daylight::Hour(HourOfDay(12.0)),
            Some(noon.elevation),
            0.0,
            0.0,
            &look
        ),
        "the ambient light dims by night"
    );

    assert_eq!(
        shining(noon, 0.0, 0.0, &look),
        noon,
        "by day the sun lights the world"
    );
    let moon = shining(midnight, 0.0, 0.0, &look);
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
            night(Daylight::Fixed, fixed, &look.sky),
            night(Daylight::Hour(HourOfDay(12.0)), noon, &look.sky),
            night(Daylight::Hour(HourOfDay(0.0)), midnight, &look.sky)
        ),
        (0.0, 0.0, 1.0),
        "the stars come out only at night, never under a fixed light"
    );

    let clouded = sun_at(
        Daylight::Hour(HourOfDay(12.0)),
        NOON.0,
        NOON.1,
        white,
        1.0,
        0.0,
        &look,
    );
    assert!(
        clouded.illuminance < noon.illuminance,
        "fair-weather cloud dims the sun"
    );
    let stormy = sun_at(
        Daylight::Hour(HourOfDay(12.0)),
        NOON.0,
        NOON.1,
        white,
        1.0,
        1.0,
        &look,
    );
    assert!(
        stormy.illuminance < clouded.illuminance && stormy.illuminance < noon.illuminance / 2.0,
        "storm cloud takes most of it"
    );
}

#[test]
fn a_day_turns_from_the_games_start_hour_or_holds_there_on_a_still_server() {
    let cycle = day(&[]);
    let started = UnixMillis(1_800_000_000_000);
    let later = |seconds: f32| started.after(Seconds(seconds));

    let turning = cycle.course(ServerDay::TURNING, started);
    let hours = [0.0, 300.0, 1200.0 * 3.0 + 600.0].map(|at| cycle.hour(turning, later(at)).0);
    for (hour, expected) in hours.into_iter().zip([10.0, 16.0, 22.0]) {
        assert!(
            (hour - expected).abs() < 0.01,
            "a turning day starts at the game's start hour and turns once a day length: {hours:?}"
        );
    }

    let still = cycle.course(
        ServerDay {
            daylight: ServerDaylight::Still,
            hour: None,
        },
        started,
    );
    assert_eq!(
        [0.0, 300.0, 1200.0 * 3.0 + 600.0].map(|at| cycle.hour(still, later(at))),
        [HourOfDay(10.0); 3],
        "a still day holds at the game's start hour however long the world runs"
    );

    let fixed = cycle.course(
        ServerDay {
            daylight: ServerDaylight::Fixed,
            hour: Some(HourOfDay(18.5)),
        },
        started,
    );
    assert_eq!(
        [0.0, 300.0, 1200.0 * 3.0 + 600.0].map(|at| cycle.hour(fixed, later(at))),
        [HourOfDay(18.5); 3],
        "a fixed day holds at the hour the server is given however long the world runs"
    );
}

#[test]
fn the_times_of_a_day_divide_it_between_them_and_ease_into_one_another() {
    let cycle = day(&[
        ("Dawn", 5.0, 1.0),
        ("Day", 8.0, 1.0),
        ("Dusk", 18.0, 1.0),
        ("Night", 21.0, 1.0),
    ]);
    let holds = |cycle: &DayCycle, hour: f32| {
        cycle
            .time_at(HourOfDay(hour))
            .map(|time| time.name.0.clone())
    };
    let share = |cycle: &DayCycle, name: &str, hour: f32| {
        cycle.share(&TimeOfDayName(name.to_owned()), HourOfDay(hour))
    };

    for (hour, name) in [
        (0.0, "Night"),
        (4.99, "Night"),
        (5.0, "Dawn"),
        (12.0, "Day"),
        (20.99, "Dusk"),
        (21.0, "Night"),
        (24.0, "Night"),
    ] {
        assert_eq!(
            holds(&cycle, hour).as_deref(),
            Some(name),
            "each time holds from its hour until the next begins, the last round midnight: {hour}"
        );
    }

    for (name, hour, expected) in [
        ("Night", 20.5, 0.0),
        ("Night", 20.75, 0.25),
        ("Night", 21.0, 0.5),
        ("Night", 21.5, 1.0),
        ("Night", 0.0, 1.0),
        ("Dusk", 21.0, 0.5),
        ("Dusk", 21.5, 0.0),
        ("Day", 21.0, 0.0),
        ("Noon", 12.0, 0.0),
    ] {
        assert!(
            (share(&cycle, name, hour) - expected).abs() < 0.001,
            "a time comes over its eases, half before its hour and half after, as the one before \
             it goes: {name} at {hour} is {}",
            share(&cycle, name, hour)
        );
    }
    for quarter in 0..96 {
        let hour = quarter as f32 / 4.0;
        let all: f32 = cycle
            .times
            .iter()
            .map(|time| share(&cycle, &time.name.0, hour))
            .sum();
        assert!(
            (all - 1.0).abs() < 0.001,
            "the shares of a day's times add up to one at every hour: {all} at {hour}"
        );
    }

    let sharp = day(&[("Day", 6.0, 0.0), ("Night", 18.0, 0.0)]);
    assert_eq!(
        [17.99, 18.0].map(|hour| share(&sharp, "Night", hour)),
        [0.0, 1.0],
        "a time that eases over nothing begins at a stroke"
    );

    let always = day(&[("Always", 9.0, 0.0)]);
    assert_eq!(
        [0.0, 9.0, 23.0].map(|hour| (holds(&always, hour), share(&always, "Always", hour))),
        [(); 3].map(|()| (Some("Always".to_owned()), 1.0)),
        "a day of one time holds it at every hour"
    );

    let unnamed = day(&[]);
    assert_eq!(
        (holds(&unnamed, 12.0), share(&unnamed, "Day", 12.0)),
        (None, 0.0),
        "a day that names no times has none to hold"
    );
}

#[test]
fn the_clock_round_the_minimap_marks_the_hour_where_an_hour_hand_would_point() {
    let toward = |hour: f32| day_clock::hand(HourOfDay(hour));
    let (up, right, down, left) = (
        egui::vec2(0.0, -1.0),
        egui::vec2(1.0, 0.0),
        egui::vec2(0.0, 1.0),
        egui::vec2(-1.0, 0.0),
    );
    for (hour, points) in [
        (0.0, up),
        (3.0, right),
        (6.0, down),
        (9.0, left),
        (12.0, up),
        (15.0, right),
        (18.0, down),
        (21.0, left),
    ] {
        assert!(
            (toward(hour) - points).length() < 0.001,
            "at {hour} the clock marks {:?}, not {points:?}",
            toward(hour)
        );
    }
    let (to_nine, to_ten) = (
        (toward(9.5) - toward(9.0)).length(),
        (toward(9.5) - toward(10.0)).length(),
    );
    assert!(
        (to_nine - to_ten).abs() < 0.001 && to_nine < (toward(9.0) - toward(10.0)).length(),
        "half past nine is marked midway between nine and ten"
    );
    assert!(
        (toward(22.0) - toward(10.0)).length() < 0.001,
        "ten at night is marked where ten in the morning is"
    );
}

fn day(times: &[(&str, f32, f32)]) -> DayCycle {
    DayCycle {
        day_length: Seconds(1200.0),
        start: HourOfDay(10.0),
        times: times
            .iter()
            .map(|&(name, from, eases)| TimeOfDay {
                name: TimeOfDayName(name.to_owned()),
                from: HourOfDay(from),
                eases: DayHours(eases),
                color: Rgb::WHITE,
                icon: ImageRef::new("icons/time.png".to_owned()),
            })
            .collect(),
    }
}
