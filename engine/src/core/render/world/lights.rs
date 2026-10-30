//! The lights of the world: one sun with soft shadows, cloud shadows and shafts of light, an
//! ambient glow, and up to a few point lights, some of which flicker.

use bevy::light::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLight, PointLight, SunDisk,
    VolumetricLight,
};
use bevy::pbr::MAX_CASCADES_PER_LIGHT;
use bevy::prelude::*;

use super::clouds::{LOWEST_LIGHT, SHADOW_SPAN};
use crate::core::geometry::Degrees;
use crate::core::render::video::{ShadowQuality, VideoPreferences};
use crate::core::time::Seconds;

/// The sun's illuminance in lux: with the camera's exposure, a white face square to the sun is
/// lit to the top of the screen's range.
pub const SUN_LUX: f32 = 2600.0;
/// How bright the ambient light is for an ambient colour of full white, in candela per square
/// metre: outside, the fill that keeps a shadow a few steps under the sun rather than black;
/// inside, where no sky reaches, the whole of the light between the lamps.
pub const AMBIENT_NITS: f32 = 800.0;
/// Lumens per unit of a content light's intensity.
pub const LUMENS_PER_UNIT: f32 = 4_000.0;
/// How far the sun's shadows reach from the camera.
const SHADOW_REACH: f32 = 70.0;

/// How far the near shadow cascade reaches, at the finer of the two.
const NEAR_SHADOWS: f32 = 25.0;
/// How wide the sun is as a source of light, which is how far its shadows soften from what casts
/// them.
const SUN_WIDTH: f32 = 1.5;

/// A sun shining from `elevation` above the horizon at `azimuth` clockwise from north: soft
/// shadows from two cascades, and shafts of light through the air.
pub fn sun(color: Color, illuminance: f32, elevation: Degrees, azimuth: Degrees) -> impl Bundle {
    (
        DirectionalLight {
            color,
            illuminance,
            shadow_maps_enabled: true,
            soft_shadow_size: soft_shadows(ShadowQuality::High),
            ..default()
        },
        shadow_cascades(ShadowQuality::High),
        VolumetricLight,
        // The light stands a little over the horizon when its sun or moon has sunk to it, so the
        // sky draws them where they truly stand rather than leaving it to the light.
        SunDisk::OFF,
        sun_transform(elevation, azimuth, Vec2::ZERO),
    )
}

/// Every sun and moon casts its shadows as the player's video preferences ask, anew whenever they
/// change.
pub(super) fn cast_shadows(
    video: Res<VideoPreferences>,
    mut lights: Query<(&mut DirectionalLight, &mut CascadeShadowConfig)>,
) {
    for (mut light, mut cascades) in &mut lights {
        if !light.is_added() && !video.is_changed() {
            continue;
        }
        light.shadow_maps_enabled = video.shadows != ShadowQuality::Off;
        light.soft_shadow_size = soft_shadows(video.shadows);
        *cascades = shadow_cascades(video.shadows);
    }
}

/// Where a sun `elevation` above the horizon at `azimuth` clockwise from north shines from, the
/// field of cloud's shadows laid over the ground round `over` along its rays, squeezed across them
/// as the light sinks so they reach as far at every hour. A light on the horizon shines from a
/// little over it, for the clouds to cast shadows at all.
pub fn sun_transform(elevation: Degrees, azimuth: Degrees, over: Vec2) -> Transform {
    let elevation = Degrees(elevation.0.max(LOWEST_LIGHT));
    let toward_sun = toward(elevation, azimuth);
    let over = Vec3::new(over.x, 0.0, over.y);
    let span = SHADOW_SPAN.0 / 2.0;
    let slant = elevation.radians().0.sin();
    Transform::from_translation(over + toward_sun * 50.0)
        .looking_at(over, Vec3::Y)
        .with_scale(Vec3::new(span, span * slant, span))
}

/// The unit way toward a light `elevation` above the horizon at `azimuth` clockwise from north.
pub fn toward(elevation: Degrees, azimuth: Degrees) -> Vec3 {
    let (sin_e, cos_e) = elevation.radians().0.sin_cos();
    let (sin_a, cos_a) = azimuth.radians().0.sin_cos();
    Vec3::new(sin_a * cos_e, sin_e, -cos_a * cos_e)
}

/// A point light `intensity` units bright reaching `radius` metres.
pub fn point(color: Color, intensity: f32, radius: f32) -> PointLight {
    PointLight {
        color,
        intensity: intensity * LUMENS_PER_UNIT,
        range: radius,
        radius: 0.05,
        shadow_maps_enabled: false,
        ..default()
    }
}

/// A light that wavers between dark and full: `amount` of its intensity comes and goes.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Flicker {
    pub amount: f32,
    pub full: f32,
    pub phase: f32,
}

pub(super) fn flicker(time: Res<Time>, mut lights: Query<(&Flicker, &mut PointLight)>) {
    let now = Seconds(time.elapsed_secs());
    for (flicker, mut light) in &mut lights {
        let t = now.0 * 7.0 + flicker.phase;
        let wave =
            (t.sin() * 0.5 + (t * 2.3 + 1.7).sin() * 0.3 + (t * 5.1).sin() * 0.2) * 0.5 + 0.5;
        light.intensity = flicker.full * (1.0 - flicker.amount * wave);
    }
}

fn soft_shadows(quality: ShadowQuality) -> Option<f32> {
    (quality == ShadowQuality::High).then_some(SUN_WIDTH)
}

/// The cascades `quality` asks for, as many as the platform draws: one that draws fewer draws only
/// the nearest, which would leave everything past it unshadowed.
fn shadow_cascades(quality: ShadowQuality) -> CascadeShadowConfig {
    let wanted = if quality == ShadowQuality::High { 2 } else { 1 };
    CascadeShadowConfigBuilder {
        num_cascades: wanted.min(MAX_CASCADES_PER_LIGHT),
        maximum_distance: SHADOW_REACH,
        first_cascade_far_bound: NEAR_SHADOWS,
        ..default()
    }
    .build()
}
