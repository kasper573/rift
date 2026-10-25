use bevy::prelude::*;
use engine::core::geometry::Degrees;
use engine::core::render::world::sky::{Daylight, SkyColors, ambient_at, sky_at, sun_at};

const NOON: (Degrees, Degrees) = (Degrees(52.0), Degrees(215.0));
const DAY: SkyColors = SkyColors {
    zenith: Color::srgb(0.3, 0.55, 0.9),
    horizon: Color::srgb(0.8, 0.9, 0.95),
};

fn same(a: SkyColors, b: SkyColors) -> bool {
    let close = |a: Color, b: Color| {
        let (a, b) = (a.to_linear(), b.to_linear());
        (a.red - b.red).abs() < 1e-4
            && (a.green - b.green).abs() < 1e-4
            && (a.blue - b.blue).abs() < 1e-4
    };
    close(a.zenith, b.zenith) && close(a.horizon, b.horizon)
}

fn luminance(color: Color) -> f32 {
    let linear = color.to_linear();
    0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue
}

#[test]
fn the_sun_sky_and_ambient_light_turn_with_the_hour_and_dim_under_cloud() {
    let white = Color::WHITE;
    let fixed = sun_at(Daylight::Fixed, NOON.0, NOON.1, white, 0.0);
    assert_eq!(
        (fixed.elevation, fixed.azimuth, fixed.color),
        (NOON.0, NOON.1, white),
        "without a clock the sun stands as authored"
    );
    assert!(
        same(sky_at(DAY, Daylight::Fixed, Some(NOON.0), 0.0), DAY),
        "without a clock the sky shows as authored"
    );

    let noon = sun_at(Daylight::Hour(12.0), NOON.0, NOON.1, white, 0.0);
    assert!(
        (noon.elevation.0 - NOON.0.0).abs() < 0.01 && (noon.azimuth.0 - NOON.1.0).abs() < 0.01,
        "at noon the sun stands where it was authored: {noon:?}"
    );
    assert_eq!(noon.illuminance, fixed.illuminance, "noon is full daylight");
    let morning = sun_at(Daylight::Hour(9.0), NOON.0, NOON.1, white, 0.0);
    assert!(
        morning.elevation.0 > 0.0 && morning.elevation.0 < NOON.0.0 && morning.azimuth < NOON.1,
        "in the morning the sun is lower and further east: {morning:?}"
    );
    let dusk = sun_at(Daylight::Hour(18.3), NOON.0, NOON.1, white, 0.0);
    assert!(
        dusk.elevation.0 < 2.0 && luminance(dusk.color) < luminance(white),
        "at dusk the sun sits on the horizon, reddened: {dusk:?}"
    );
    let midnight = sun_at(Daylight::Hour(0.0), NOON.0, NOON.1, white, 0.0);
    assert!(
        midnight.elevation.0 < 0.0 && midnight.illuminance == 0.0,
        "at midnight the sun is below the horizon and gives no light: {midnight:?}"
    );

    let night_sky = sky_at(DAY, Daylight::Hour(0.0), Some(midnight.elevation), 0.0);
    let day_sky = sky_at(DAY, Daylight::Hour(12.0), Some(noon.elevation), 0.0);
    assert!(
        luminance(night_sky.zenith) < luminance(day_sky.zenith) / 4.0,
        "the night sky is dark: {night_sky:?} against {day_sky:?}"
    );
    assert!(
        ambient_at(Daylight::Hour(0.0), Some(midnight.elevation), 0.0)
            < ambient_at(Daylight::Hour(12.0), Some(noon.elevation), 0.0),
        "the ambient light dims by night"
    );
    let indoors = sky_at(DAY, Daylight::Hour(0.0), None, 0.0);
    assert!(
        same(indoors, DAY),
        "an area without a sun keeps its sky day and night: {indoors:?}"
    );

    let clouded = sun_at(Daylight::Hour(12.0), NOON.0, NOON.1, white, 1.0);
    assert!(
        clouded.illuminance < noon.illuminance / 2.0,
        "cloud takes most of the sun"
    );
    let grey = sky_at(DAY, Daylight::Hour(12.0), Some(noon.elevation), 1.0)
        .zenith
        .to_linear();
    assert!(
        (grey.red - grey.blue).abs() < 0.1 && (grey.green - grey.blue).abs() < 0.1,
        "an overcast sky is grey, not {grey:?}"
    );
}
