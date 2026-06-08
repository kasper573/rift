use client::render::{proximity_pan, proximity_volume};
use world::core::math::{Pos, Tiles};

fn p(x: f32, y: f32) -> Pos<Tiles> {
    Pos::new(Tiles(x), Tiles(y))
}

#[test]
fn volume_is_full_at_the_listener() {
    let at = p(10.0, 10.0);
    assert_eq!(proximity_volume(at, at), 1.0);
}

#[test]
fn volume_falls_linearly_to_the_view_edge() {
    let at = p(0.0, 0.0);
    // Half the view spans 12 tiles wide and 9 tall: halfway out is half volume, the edge is silent.
    assert_eq!(proximity_volume(at, p(6.0, 0.0)), 0.5);
    assert_eq!(proximity_volume(at, p(12.0, 0.0)), 0.0);
    assert_eq!(proximity_volume(at, p(0.0, 4.5)), 0.5);
}

#[test]
fn volume_is_zero_beyond_the_view() {
    let at = p(0.0, 0.0);
    assert_eq!(proximity_volume(at, p(50.0, 0.0)), 0.0);
    assert_eq!(proximity_volume(at, p(0.0, -100.0)), 0.0);
}

#[test]
fn pan_is_centered_at_the_listener() {
    let at = p(10.0, 10.0);
    assert_eq!(proximity_pan(at, at), 0.0);
}

#[test]
fn pan_tracks_horizontal_offset() {
    let at = p(0.0, 0.0);
    // Half the view is 12 tiles wide: halfway is ±0.5, the edge is hard ±1.
    assert_eq!(proximity_pan(at, p(-6.0, 0.0)), -0.5);
    assert_eq!(proximity_pan(at, p(6.0, 0.0)), 0.5);
    assert_eq!(proximity_pan(at, p(12.0, 0.0)), 1.0);
}

#[test]
fn pan_clamps_beyond_the_view_and_ignores_vertical() {
    let at = p(0.0, 0.0);
    assert_eq!(proximity_pan(at, p(50.0, 0.0)), 1.0);
    assert_eq!(proximity_pan(at, p(-100.0, 0.0)), -1.0);
    assert_eq!(proximity_pan(at, p(6.0, 99.0)), 0.5);
}
