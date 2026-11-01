//! The lights of the world: one sun with soft shadows, cloud shadows and shafts of light, an
//! ambient glow, and up to a few point lights, some of which flicker.

use bevy::light::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLight, PointLight, SunDisk,
    VolumetricLight,
};
use bevy::prelude::*;

use super::clouds::{LOWEST_LIGHT, SHADOW_SPAN};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::geometry::Degrees;
use crate::core::render::Look;
use crate::core::render::video::{ShadowQuality, VideoPreferences};
use crate::core::time::Seconds;

/// How strong the world's lights are, and how soft the sun's shadows.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LightLook {
    /// The sun's illuminance in lux: with the camera's exposure, a white face square to the sun
    /// is lit to the top of the screen's range.
    pub sun_lux: f32,
    /// How bright the ambient light is for an ambient colour of full white, in candela per
    /// square metre: outside, the fill that keeps a shadow a few steps under the sun rather than
    /// black; inside, where no sky reaches, the whole of the light between the lamps.
    pub ambient_nits: f32,
    /// How wide the sun is as a source of light, which is how far its shadows soften from what
    /// casts them.
    pub sun_width: f32,
}
/// Lumens per unit of a content light's intensity.
pub const LUMENS_PER_UNIT: f32 = 4_000.0;
/// How far the sun's shadows reach from the camera.
const SHADOW_REACH: f32 = 70.0;

/// How far the near shadow cascade reaches, at the finer of the two.
const NEAR_SHADOWS: f32 = 25.0;

/// A sun shining from `elevation` above the horizon at `azimuth` clockwise from north: soft
/// shadows from two cascades, and shafts of light through the air.
pub fn sun(color: Color, illuminance: f32, elevation: Degrees, azimuth: Degrees) -> impl Bundle {
    (
        DirectionalLight {
            color,
            illuminance,
            shadow_maps_enabled: true,
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
    look: Option<Res<Look>>,
    mut lights: Query<(&mut DirectionalLight, &mut CascadeShadowConfig)>,
) {
    let Some(look) = look else {
        return;
    };
    for (mut light, mut cascades) in &mut lights {
        if !light.is_added() && !video.is_changed() && !look.is_changed() {
            continue;
        }
        light.shadow_maps_enabled = video.shadows != ShadowQuality::Off;
        light.soft_shadow_size =
            (video.shadows == ShadowQuality::High).then_some(look.light.sun_width);
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

/// The cascades `quality` asks for: a fine one near the camera at the highest, and one otherwise.
fn shadow_cascades(quality: ShadowQuality) -> CascadeShadowConfig {
    CascadeShadowConfigBuilder {
        num_cascades: if quality == ShadowQuality::High { 2 } else { 1 },
        maximum_distance: SHADOW_REACH,
        first_cascade_far_bound: NEAR_SHADOWS,
        ..default()
    }
    .build()
}
