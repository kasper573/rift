use bevy::prelude::*;
use engine::core::render::world::haze::HazeSight;

const FOV: f32 = std::f32::consts::FRAC_PI_3;
const ASPECT: f32 = 16.0 / 9.0;
const EYE: Vec3 = Vec3::new(0.0, 10.0, 0.0);

/// A camera looks through the haze its rays cross and no other: a bank lying where it looks, as
/// far as its rays run before they meet the lowest ground, and none behind it, beside it or past
/// where they have all met the ground. Looking level its rays never meet the ground, and looking
/// straight down they run every way round it.
#[test]
fn a_camera_looks_through_the_haze_its_rays_cross_and_no_other() {
    let ahead = Vec3::NEG_Z;
    let down_ahead = Vec3::new(0.0, -1.0, -1.0);
    let cases = [
        ("ahead", down_ahead, (-5.0, -30.0), (5.0, -20.0), true),
        ("round the eye", down_ahead, (-2.0, -2.0), (2.0, 2.0), true),
        ("behind", down_ahead, (-5.0, 20.0), (5.0, 30.0), false),
        ("beside", down_ahead, (20.0, 0.0), (30.0, 10.0), false),
        (
            "past the rays' reach",
            down_ahead,
            (-5.0, -80.0),
            (5.0, -70.0),
            false,
        ),
        (
            "far ahead, looking level",
            ahead,
            (-5.0, -5000.0),
            (5.0, -4990.0),
            true,
        ),
        (
            "behind, looking level",
            ahead,
            (-5.0, 20.0),
            (5.0, 30.0),
            false,
        ),
        (
            "behind, looking down",
            Vec3::NEG_Y,
            (-3.0, 3.0),
            (3.0, 8.0),
            true,
        ),
        (
            "far off, looking down",
            Vec3::NEG_Y,
            (-5.0, 30.0),
            (5.0, 40.0),
            false,
        ),
    ];
    for (what, looking, low, high, crossed) in cases {
        let up = if looking == Vec3::NEG_Y {
            Vec3::NEG_Z
        } else {
            Vec3::Y
        };
        let eye = GlobalTransform::from(Transform::from_translation(EYE).looking_to(looking, up));
        let lens = bevy::math::proj::perspective_infinite_reverse(FOV, ASPECT, 0.1);
        let sight = HazeSight::of(lens, &eye, 0.0);
        assert_eq!(
            sight.crosses(Vec2::from(low), Vec2::from(high)),
            crossed,
            "a bank {what}"
        );
    }
}
