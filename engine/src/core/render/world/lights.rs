//! The lights of the world: one sun, an ambient glow, and up to a few point lights, some of which
//! flicker.

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLight, PointLight};
use bevy::prelude::*;

use crate::core::geometry::Degrees;
use crate::core::time::Seconds;

/// The sun's illuminance in lux: with the camera's exposure, a face square to the sun is fully
/// lit, the top step of the toon ramp.
pub const SUN_LUX: f32 = 3200.0;
/// How bright the ambient light is for an ambient colour of full white, in candela per square
/// metre: a shaded face sits at the toon ramp's lowest step.
pub const AMBIENT_NITS: f32 = 650.0;
/// Lumens per unit of a content light's intensity.
pub const LUMENS_PER_UNIT: f32 = 10_000.0;
/// How far the sun's shadows reach from the camera.
const SHADOW_REACH: f32 = 70.0;

/// A sun shining from `elevation` above the horizon at `azimuth` clockwise from north, with soft
/// shadows.
pub fn sun(color: Color, illuminance: f32, elevation: Degrees, azimuth: Degrees) -> impl Bundle {
    (
        DirectionalLight {
            color,
            illuminance,
            shadow_maps_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            maximum_distance: SHADOW_REACH,
            first_cascade_far_bound: SHADOW_REACH / 2.0,
            ..default()
        }
        .build(),
        sun_transform(elevation, azimuth),
    )
}

/// Where a sun `elevation` above the horizon at `azimuth` clockwise from north shines from.
pub fn sun_transform(elevation: Degrees, azimuth: Degrees) -> Transform {
    let (sin_e, cos_e) = elevation.radians().0.sin_cos();
    let (sin_a, cos_a) = azimuth.radians().0.sin_cos();
    let toward_sun = Vec3::new(sin_a * cos_e, sin_e, -cos_a * cos_e);
    Transform::from_translation(toward_sun * 50.0).looking_at(Vec3::ZERO, Vec3::Y)
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
