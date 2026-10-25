use bevy::prelude::*;
use engine::core::geometry::Degrees;
use engine::core::math::Pos;
use engine::core::render::camera::ground_hit;
use engine::core::render::{CameraRig, capsule_hit, world_point};
use engine::core::terrain::{Heightfield, SAMPLES_PER_METER, TerrainSize};
use engine::core::units::{MeterPos, Meters};

const SIZE: TerrainSize = TerrainSize {
    width: 32,
    height: 32,
};
const WINDOW: Vec2 = Vec2::new(1280.0, 720.0);

/// A hill four metres high in the middle of the ground, sloping down a metre every two.
fn hill() -> Heightfield {
    Heightfield::from_fn(SIZE, |column, row| {
        let x = column as f32 / SAMPLES_PER_METER as f32;
        let y = row as f32 / SAMPLES_PER_METER as f32;
        let out = ((x - 16.0).powi(2) + (y - 16.0).powi(2)).sqrt();
        Meters((4.0 - out / 2.0).max(0.0))
    })
}

#[test]
fn the_pointer_ray_lands_where_the_ground_shows() {
    let heights = hill();
    let bounds = SIZE.bounds();
    let spots = [
        (Pos::new(16.0, 16.0), "the top of the hill"),
        (Pos::new(12.0, 19.0), "the slope"),
        (Pos::new(3.0, 3.0), "the flat"),
    ];
    for (turned, tilted) in [(0.0, -25.0), (45.0, 0.0), (135.0, 15.0), (270.0, -10.0)] {
        let mut rig = CameraRig::default();
        rig.look_at(Pos::new(14.0, 14.0), Meters(1.0));
        rig.turn_now(Degrees(turned));
        rig.tilt_now(Degrees(tilted));
        let view = rig.view(WINDOW);
        for (at, name) in spots {
            let shown = view
                .project(world_point(at, heights.height_at(at)))
                .unwrap_or_else(|| {
                    panic!("{name} is in view with the camera turned {turned}° tilted {tilted}°")
                });
            let landed = ground_hit(&heights, bounds, view.ray(shown)).unwrap_or_else(|| {
                panic!("the ray through {name} lands, turned {turned}° tilted {tilted}°")
            });
            assert!(
                landed.distance(at) < Meters(0.05),
                "pointing at {name} turned {turned}° landed {landed:?}, not {at:?}"
            );
        }
    }
}

#[test]
fn pointing_at_a_body_where_it_shows_picks_it() {
    const RADIUS: f32 = 0.4;
    const HEIGHT: f32 = 2.3;
    let heights = hill();
    let at = Pos::new(12.0, 19.0);
    let foot = world_point(at, heights.height_at(at));
    for zoomed in 0..3 {
        let mut rig = CameraRig::default();
        rig.look_at(Pos::new(14.0, 14.0), Meters(1.0));
        for _ in 0..zoomed {
            rig.zoom_in();
        }
        let view = rig.view(WINDOW);
        for (height, part) in [(0.0, "feet"), (HEIGHT / 2.0, "middle"), (HEIGHT, "head")] {
            let shown = view
                .project(foot + Vec3::Y * height)
                .unwrap_or_else(|| panic!("the body's {part} show at zoom {zoomed}"));
            assert!(
                capsule_hit(view.ray(shown), foot, RADIUS, HEIGHT).is_some(),
                "pointing at the body's {part} at zoom {zoomed} picks it"
            );
        }
        let beside = view
            .project(foot + Vec3::X * (RADIUS * 3.0))
            .expect("the ground beside the body shows");
        assert!(
            capsule_hit(view.ray(beside), foot, RADIUS, HEIGHT).is_none(),
            "pointing a metre beside the body at zoom {zoomed} misses it"
        );
    }
}
