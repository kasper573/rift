use bevy_math::Vec2;
use ui::{Align, Side, place};

#[test]
fn anchors_below_on_the_preferred_side() {
    let pos = place(
        Vec2::new(100.0, 100.0),
        Vec2::new(50.0, 20.0),
        Vec2::new(80.0, 30.0),
        Vec2::new(1000.0, 1000.0),
        Side::Bottom,
        Align::Start,
        8.0,
    );
    assert_eq!(pos, Vec2::new(100.0, 128.0));
}

#[test]
fn centers_on_the_cross_axis() {
    let pos = place(
        Vec2::new(100.0, 100.0),
        Vec2::new(50.0, 20.0),
        Vec2::new(80.0, 30.0),
        Vec2::new(1000.0, 1000.0),
        Side::Bottom,
        Align::Center,
        0.0,
    );
    assert_eq!(pos, Vec2::new(85.0, 120.0));
}

#[test]
fn flips_to_the_opposite_side_on_overflow() {
    let pos = place(
        Vec2::new(100.0, 980.0),
        Vec2::new(50.0, 20.0),
        Vec2::new(80.0, 30.0),
        Vec2::new(1000.0, 1000.0),
        Side::Bottom,
        Align::Start,
        8.0,
    );
    assert_eq!(
        pos,
        Vec2::new(100.0, 942.0),
        "below would overflow, so it flips above"
    );
}

#[test]
fn clamps_into_the_viewport() {
    let pos = place(
        Vec2::new(-100.0, 100.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(40.0, 30.0),
        Vec2::new(1000.0, 1000.0),
        Side::Bottom,
        Align::Start,
        0.0,
    );
    assert_eq!(
        pos,
        Vec2::new(0.0, 110.0),
        "a negative cross position is clamped to the edge"
    );
}
