use bevy::prelude::*;
use engine::core::jobs::Handoff;
use engine::core::render::world::clouds::{self, CLOUD_TILE, CloudDrift, CloudSheet, SHADOW_SPAN};
use engine::core::render::world::lights::sun_transform;
use engine::core::units::Degrees;

const CLOUDS: f32 = 0.5;
const WATER: f32 = 0.0;
const WIND: Vec2 = Vec2::new(1.0, 0.4);

#[test]
fn the_ground_lies_in_the_shadow_of_the_cloud_up_the_suns_rays_as_the_clouds_drift_and_the_sun_turns()
 {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    let mut sheet = CloudSheet::new(&mut world, testkit::look().clouds);
    let mut images = world.resource_mut::<Assets<Image>>();
    sheet.cover(CLOUDS, WATER);
    let near = Vec2::new(64.0, 64.0);
    let far = Vec2::new(3000.0, 900.0);
    let mut drifting = CloudDrift::default();
    let suns = [
        ("at noon", near, 52.0, 215.0, 0.0),
        ("a moment of drift later", near, 52.0, 215.0, 1.5),
        (
            "a little longer, laid shadows carried",
            near,
            52.0,
            215.0,
            6.0,
        ),
        ("long enough to be laid anew", near, 52.0, 215.0, 30.0),
        ("the sun turned a hair", near, 52.05, 215.1, 30.5),
        ("the sun low in the evening", near, 12.0, 290.0, 31.0),
        ("seen from far across the land", far, 12.0, 290.0, 31.5),
    ];
    for (when, over, elevation, azimuth, seconds) in suns {
        let light = sun_transform(Degrees(elevation), Degrees(azimuth), over);
        let drift = drifting.blown(WIND, seconds, sheet.look());
        sheet.lay(light, drift, &mut images, Handoff::NextFrame);
        // The shadows are painted off the main thread and shown from the next frame's lay on.
        let cast = sheet.lay(light, drift, &mut images, Handoff::NextFrame);
        let shadows = images.get(&sheet.over_sun().image).expect("the shadows");
        let toward = cast.back().as_vec3();
        let from_light = cast.compute_affine().inverse();
        let lit_at = |ground: Vec3| {
            let local = from_light.transform_point3(ground);
            texel(
                shadows,
                Vec2::new((1.0 - local.x) / 2.0, (local.y + 1.0) / 2.0),
            )
        };
        let mut off = 0.0;
        let mut mistold = 0.0;
        let mut points = 0.0;
        for x in (-64..64).step_by(8) {
            for z in (-64..64).step_by(8) {
                let ground = Vec3::new(over.x + x as f32, 0.0, over.y + z as f32);
                let up = ground + toward * sheet.look().height.0 / toward.y;
                let cloud = sheet
                    .reading()
                    .cloudiness((Vec2::new(up.x, up.z) - drift) / CLOUD_TILE.0);
                off += (lit_at(ground) - (1.0 - clouds::shade(WATER, sheet.look()) * cloud)).abs();
                mistold += (lit_at(ground) - (1.0 - sheet.shade_over(ground, drift))).abs();
                points += 1.0;
            }
        }
        assert!(
            off / points < 0.03,
            "{when} the shadows fall under the clouds: off by {} on average",
            off / points
        );
        assert!(
            mistold / points < 0.03,
            "{when} the shade told of a spot is the shadow it lies in: off by {} on average",
            mistold / points
        );
        let rim = SHADOW_SPAN.0 / 2.0 - 1.0;
        let across = cast.right().as_vec3() * rim;
        let joined = (-4..=4).all(|along| {
            let along = cast.up().as_vec3() * along as f32 * 50.0;
            let middle = Vec3::new(over.x, 0.0, over.y);
            (lit_at(middle + across + along) - lit_at(middle - across + along)).abs() < 0.01
        });
        assert!(joined, "{when} the shadows join where they repeat");
    }
    let told = drifting.drift();
    let mut joining = CloudDrift::told(told);
    assert_eq!(
        joining.blown(WIND, 500.0, sheet.look()),
        told,
        "a client joining finds the clouds where its server tells it they have drifted to"
    );
    let since = joining.blown(WIND, 501.0, sheet.look()) - told;
    let meanwhile = drifting.blown(WIND, 32.5, sheet.look()) - told;
    assert!(
        since.distance(meanwhile) < 1.0e-3,
        "and carries them on from there as the server does: {since} against {meanwhile}"
    );
}

#[test]
fn the_sky_never_shows_the_same_clouds_a_tile_over_whichever_way_one_looks() {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    let mut sheet = CloudSheet::new(&mut world, testkit::look().clouds);
    sheet.cover(CLOUDS, WATER);
    for (way, step) in [
        ("along a row", Vec2::X),
        ("along a column", Vec2::Y),
        ("across both", Vec2::ONE),
    ] {
        let mut differs = 0.0;
        let mut points = 0.0;
        for x in 0..32 {
            for y in 0..32 {
                let at = Vec2::new(x as f32, y as f32) / 32.0 * 8.0;
                differs +=
                    (sheet.reading().cloudiness(at) - sheet.reading().cloudiness(at + step)).abs();
                points += 1.0;
            }
        }
        assert!(
            differs / points > 0.1,
            "{way}, the clouds a tile over differ by only {} on average",
            differs / points
        );
    }
}

/// The red of a repeating picture at `uv`, the nearest texel's.
fn texel(image: &Image, uv: Vec2) -> f32 {
    let size = image.size();
    let at = (uv.rem_euclid(Vec2::ONE) * size.as_vec2())
        .as_uvec2()
        .min(size - 1);
    image
        .get_color_at(at.x, at.y)
        .expect("a texel")
        .to_linear()
        .red
}
