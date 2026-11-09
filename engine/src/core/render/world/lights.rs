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

use crate::core::render::Look;
use crate::core::render::video::{ShadowQuality, VideoPreferences};
use crate::core::units::{Degrees, Meters, Seconds};

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
const LUMENS_PER_UNIT: f32 = 4_000.0;
/// How far the sun's shadows reach from the camera at the farthest.
const SHADOW_REACH: Meters = Meters(70.0);

/// The share of the shadows' reach the finer of two cascades takes, nearest the camera.
const NEAR_SHARE: f32 = 25.0 / 70.0;

/// How near the camera the shadows reach at the least.
const LEAST_REACH: Meters = Meters(1.0);

/// How many steps a shadows' reach takes as it doubles: the shadow map's texels grow and shrink
/// with it, so it moves only by steps, and only on to the next once the view needs it.
const REACH_STEPS_A_DOUBLING: f32 = 4.0;

/// How many rays along each edge of a camera's view find how far it sees the ground.
const RIM_RAYS: u32 = 8;

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
        shadow_cascades(ShadowQuality::High, ShadowReach::FARTHEST),
        ShadowReach::FARTHEST,
        VolumetricLight,
        // The light stands a little over the horizon when its sun or moon has sunk to it, so the
        // sky draws them where they truly stand rather than leaving it to the light.
        SunDisk::OFF,
        sun_transform(elevation, azimuth, Vec2::ZERO),
    )
}

/// How far off its camera a sun's shadows reach: as far as the camera sees the ground, and no
/// farther than [`SHADOW_REACH`]. The sun's shadow map spans what its cascades reach, so the nearer
/// the farthest ground in view, the finer its texels: a camera looking down on the ground from
/// near it spends them all on the ground it sees. The sun turns as the day goes, and each frame
/// its shadows are drawn anew under it, so a texel coarse beside the screen's pixels would show
/// the edges of every shadow shifting from frame to frame.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShadowReach(pub Meters);

impl ShadowReach {
    pub const FARTHEST: ShadowReach = ShadowReach(SHADOW_REACH);

    /// The reach a camera with `clip_from_view` at `eye` needs, where `ground` says where a ray
    /// first meets the ground, if it does: as far along the camera's gaze as the farthest ground on
    /// the rim of its view. Over a field of heights, the higher up the view a point lies the
    /// farther the ground it shows, so no ground in view lies farther than the rim's.
    pub fn seen(
        clip_from_view: Mat4,
        eye: &GlobalTransform,
        ground: impl Fn(Ray3d) -> Option<Vec3>,
    ) -> ShadowReach {
        let forward = *eye.forward();
        let mut farthest = 0.0f32;
        for corner in rim() {
            let Some(landed) = ray_through(clip_from_view, eye, corner).and_then(&ground) else {
                return ShadowReach::FARTHEST;
            };
            farthest = farthest.max((landed - eye.translation()).dot(forward));
        }
        ShadowReach::stepped_to(Meters(farthest))
    }

    /// The reach that follows this one to `needed`: on up a step as soon as the view needs more,
    /// and down only once it needs two steps less, so a view on the edge of a step keeps to it.
    pub fn following(self, needed: ShadowReach) -> ShadowReach {
        let step = 2.0f32.powf(1.0 / REACH_STEPS_A_DOUBLING);
        if needed.0.0 > self.0.0 || needed.0.0 * step * step < self.0.0 {
            needed
        } else {
            self
        }
    }

    fn stepped_to(farthest: Meters) -> ShadowReach {
        let steps = (farthest.0.max(LEAST_REACH.0).log2() * REACH_STEPS_A_DOUBLING).ceil();
        ShadowReach(Meters(
            (steps / REACH_STEPS_A_DOUBLING).exp2().min(SHADOW_REACH.0),
        ))
    }
}

/// Every sun and moon casts its shadows as the player's video preferences ask, as far as its
/// camera sees, anew whenever either changes.
pub(super) fn cast_shadows(
    video: Res<VideoPreferences>,
    look: Option<Res<Look>>,
    mut lights: Query<(
        &mut DirectionalLight,
        &mut CascadeShadowConfig,
        Option<Ref<ShadowReach>>,
    )>,
) {
    let Some(look) = look else {
        return;
    };
    for (mut light, mut cascades, reach) in &mut lights {
        let reached = reach.as_ref().is_some_and(Ref::is_changed);
        if !light.is_added() && !video.is_changed() && !look.is_changed() && !reached {
            continue;
        }
        light.shadow_maps_enabled = video.shadows != ShadowQuality::Off;
        light.soft_shadow_size =
            (video.shadows == ShadowQuality::High).then_some(look.light.sun_width);
        *cascades = shadow_cascades(
            video.shadows,
            reach.map_or(ShadowReach::FARTHEST, |reach| *reach),
        );
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

/// The cascades `quality` asks for out to `reach`: a fine one near the camera at the highest, and
/// one otherwise.
fn shadow_cascades(quality: ShadowQuality, reach: ShadowReach) -> CascadeShadowConfig {
    CascadeShadowConfigBuilder {
        num_cascades: if quality == ShadowQuality::High { 2 } else { 1 },
        maximum_distance: reach.0.0,
        first_cascade_far_bound: reach.0.0 * NEAR_SHARE,
        ..default()
    }
    .build()
}

/// Points round the rim of a view, in normalized device coordinates.
fn rim() -> impl Iterator<Item = Vec2> {
    (0..RIM_RAYS).flat_map(|nth| {
        let along = nth as f32 / RIM_RAYS as f32 * 2.0 - 1.0;
        [
            Vec2::new(along, 1.0),
            Vec2::new(1.0, -along),
            Vec2::new(-along, -1.0),
            Vec2::new(-1.0, along),
        ]
    })
}

/// The ray from a camera with `clip_from_view` at `eye` through `ndc` of its view.
fn ray_through(clip_from_view: Mat4, eye: &GlobalTransform, ndc: Vec2) -> Option<Ray3d> {
    let view_from_clip = clip_from_view.inverse();
    let near = view_from_clip.project_point3(ndc.extend(1.0));
    let far = view_from_clip.project_point3(ndc.extend(f32::EPSILON));
    let direction = Dir3::new(eye.affine().transform_vector3(far - near)).ok()?;
    Some(Ray3d::new(eye.transform_point(near), direction))
}
