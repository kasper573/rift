use bevy::prelude::*;
use engine::core::geometry::Degrees;
use engine::core::render::world::clouds::{self, CLOUD_BASE, CLOUD_TILE, CloudSheet};
use engine::core::render::world::lights::sun_transform;

const CLOUDS: f32 = 0.5;
const WATER: f32 = 0.0;
const WIND: Vec2 = Vec2::new(1.0, 0.4);

#[test]
fn the_ground_lies_in_the_shadow_of_the_cloud_up_the_suns_rays_as_the_clouds_drift_and_the_sun_turns()
 {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    let mut sheet = CloudSheet::from_world(&mut world);
    let mut images = world.resource_mut::<Assets<Image>>();
    sheet.cover(CLOUDS, WATER);
    let over = Vec2::new(64.0, 64.0);
    let suns = [
        ("at noon", 52.0, 215.0, 0.0),
        ("a moment of drift later", 52.0, 215.0, 1.5),
        ("a little longer, laid shadows carried", 52.0, 215.0, 6.0),
        ("long enough to be laid anew", 52.0, 215.0, 30.0),
        ("the sun turned a hair", 52.05, 215.1, 30.5),
        ("the sun low in the evening", 12.0, 290.0, 31.0),
    ];
    for (when, elevation, azimuth, seconds) in suns {
        let light = sun_transform(Degrees(elevation), Degrees(azimuth), over);
        let drift = clouds::drift(WIND, seconds);
        let cast = sheet.lay(light, drift, &mut images);
        let shadows = images.get(&sheet.over_sun().image).expect("the shadows");
        let toward = cast.back().as_vec3();
        let from_light = cast.compute_affine().inverse();
        let mut off = 0.0;
        let mut points = 0.0;
        for x in (0..128).step_by(8) {
            for z in (0..128).step_by(8) {
                let ground = Vec3::new(x as f32, 0.0, z as f32);
                let up = ground + toward * CLOUD_BASE.0 / toward.y;
                let cloud = sheet.cloudiness((Vec2::new(up.x, up.z) - drift) / CLOUD_TILE.0);
                let local = from_light.transform_point3(ground);
                let lit = texel(
                    shadows,
                    Vec2::new((1.0 - local.x) / 2.0, (local.y + 1.0) / 2.0),
                );
                off += (lit - (1.0 - clouds::shade(WATER) * cloud)).abs();
                points += 1.0;
            }
        }
        assert!(
            off / points < 0.03,
            "{when} the shadows fall under the clouds: off by {} on average",
            off / points
        );
    }
}

#[test]
fn the_sky_never_shows_the_same_clouds_a_tile_over_whichever_way_one_looks() {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    let mut sheet = CloudSheet::from_world(&mut world);
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
                differs += (sheet.cloudiness(at) - sheet.cloudiness(at + step)).abs();
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
