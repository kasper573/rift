//! The lights of the world: one sun with its shadows, cloud shadows and shafts of light, an
//! ambient glow, and up to a few point lights, some of which flicker.

use bevy::light::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLight, PointLight, SunDisk,
    VolumetricLight,
};
use bevy::pbr::MAX_CASCADES_PER_LIGHT;
use bevy::prelude::*;

use super::clouds::{LOWEST_LIGHT, SHADOW_SPAN};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::ground::Heightfield;
use crate::core::math::Rect;
use crate::core::render::camera;
use crate::core::render::video::{ShadowQuality, VideoPreferences};
use crate::core::units::{Degrees, Meters, Seconds};

/// How strong the world's lights are.
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
}
/// Lumens per unit of a content light's intensity.
const LUMENS_PER_UNIT: f32 = 4_000.0;
/// How far the sun's shadows reach where no camera's view says how far it sees, and the farthest
/// they reach in a single cascade, which spread any farther loses the shapes of what casts them.
const CLOSE_REACH: Meters = Meters(70.0);

/// The share of the shadows' reach the finest cascade takes, nearest the camera, and the farthest
/// it reaches.
const NEAR_SHARE: f32 = 25.0 / 70.0;
const NEAR_REACH: Meters = Meters(25.0);

/// How many times farther each cascade reaches than the one before it, at the most: a reach that
/// asks for more takes another cascade. A cascade's texels are spread evenly over all it reaches,
/// so where it begins they are this many times coarser beside the view's pixels than where it
/// ends.
const CASCADE_GROWTH: f32 = 4.3;

/// How near the camera the shadows reach at the least.
const LEAST_REACH: Meters = Meters(1.0);

/// How many steps a shadows' reach takes as it doubles: the shadow map's texels grow and shrink
/// with it, so it moves only by steps, and only on to the next once the view needs it.
const REACH_STEPS_A_DOUBLING: f32 = 4.0;

/// How many rays along each edge of a camera's view find how far it sees the ground.
const RIM_RAYS: u32 = 8;

/// A sun shining from `elevation` above the horizon at `azimuth` clockwise from north: shadows
/// in cascades, and shafts of light through the air.
pub fn sun(color: Color, illuminance: f32, elevation: Degrees, azimuth: Degrees) -> impl Bundle {
    (
        DirectionalLight {
            color,
            illuminance,
            shadow_maps_enabled: true,
            ..default()
        },
        shadow_cascades(ShadowQuality::High, ShadowReach::CLOSE),
        ShadowReach::CLOSE,
        VolumetricLight,
        // The light stands a little over the horizon when its sun or moon has sunk to it, so the
        // sky draws them where they truly stand rather than leaving it to the light.
        SunDisk::OFF,
        sun_transform(elevation, azimuth, Vec2::ZERO),
    )
}

/// How far off its camera a sun's shadows reach: as far as the camera sees the ground, and no
/// farther than its cascades hold the shapes of what casts them. The sun's shadow map spans what
/// its cascades reach, so the nearer the farthest ground in view, the finer its texels: a camera
/// looking down on the ground from near it spends them all on the ground it sees, and one looking
/// out over the land spreads them to the horizon. The sun turns as the day goes, and each frame
/// its shadows are drawn anew under it, so a texel coarse beside the screen's pixels would show
/// the edges of every shadow shifting from frame to frame.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShadowReach(pub Meters);

impl ShadowReach {
    /// The reach of a sun no camera's view has reached yet.
    pub const CLOSE: ShadowReach = ShadowReach(CLOSE_REACH);

    /// The farthest a sun's shadows reach: where the last of all the cascades a light casts
    /// ends, each grown as far past the one before it as a cascade may.
    pub fn farthest() -> ShadowReach {
        let grown = CASCADE_GROWTH.powi(MAX_CASCADES_PER_LIGHT as i32 - 1);
        ShadowReach(Meters(NEAR_REACH.0 * grown))
    }

    /// The reach a camera with `clip_from_view` at `eye` needs over ground of `heights` within
    /// `bounds`: as far along the camera's gaze as the farthest ground the rim of its view can
    /// show. Over a field of heights, the higher up the view a point lies the farther the ground
    /// it shows, so no ground in view lies farther than the rim's.
    pub fn seen(
        clip_from_view: Mat4,
        eye: &GlobalTransform,
        heights: &Heightfield,
        bounds: Rect<Meters>,
    ) -> ShadowReach {
        let forward = *eye.forward();
        let farthest = ShadowReach::farthest().0;
        let seen = rim()
            .filter_map(|corner| {
                let ray = ray_through(clip_from_view, eye, corner)?;
                camera::ground_sight(heights, bounds, ray, farthest)
            })
            .map(|seen| (seen - eye.translation()).dot(forward))
            .fold(0.0, f32::max);
        ShadowReach::stepped_to(Meters(seen))
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
            (steps / REACH_STEPS_A_DOUBLING)
                .exp2()
                .min(ShadowReach::farthest().0.0),
        ))
    }
}

/// Every sun and moon casts its shadows as the player's video preferences ask, as far as its
/// camera sees, anew whenever either changes.
pub(super) fn cast_shadows(
    video: Res<VideoPreferences>,
    mut lights: Query<(
        &mut DirectionalLight,
        &mut CascadeShadowConfig,
        Option<Ref<ShadowReach>>,
    )>,
) {
    for (mut light, mut cascades, reach) in &mut lights {
        let reached = reach.as_ref().is_some_and(Ref::is_changed);
        if !light.is_added() && !video.is_changed() && !reached {
            continue;
        }
        light.shadow_maps_enabled = video.shadows != ShadowQuality::Off;
        *cascades = shadow_cascades(
            video.shadows,
            reach.map_or(ShadowReach::CLOSE, |reach| *reach),
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

/// A lamp: a point light `intensity` units bright that reaches `radius`, `flicker` of its
/// brightness coming and going, and what stands in it casting shadows when `shadows`, where the
/// player's video preferences draw any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub color: Color,
    pub intensity: f32,
    pub radius: Meters,
    pub flicker: f32,
    pub shadows: bool,
}

impl Lamp {
    /// Lights the lamp where the world begins, to be stood where it shines from, its flicker
    /// `phase` into its wave.
    pub fn spawn(self, world: &mut World, phase: f32) -> Entity {
        let light = PointLight {
            color: self.color,
            intensity: self.intensity * LUMENS_PER_UNIT,
            range: self.radius.0,
            radius: 0.05,
            shadow_maps_enabled: false,
            shadow_map_near_z: OWN_BODY.0,
            ..default()
        };
        let full = light.intensity;
        let mut lamp = world.spawn((light, Transform::default(), Visibility::default()));
        if self.flicker > 0.0 {
            lamp.insert(Flicker {
                amount: self.flicker,
                full,
                phase,
            });
        }
        if self.shadows {
            lamp.insert(ShadowCasting);
        }
        lamp.id()
    }
}

/// How near a lamp nothing casts a shadow from: a lamp shines from inside what carries it, the
/// head of a torch or the glass of a lantern, which would otherwise shade all it lights.
const OWN_BODY: Meters = Meters(0.3);

/// A lamp whose light casts shadows.
#[derive(Component)]
pub struct ShadowCasting;

/// Every lamp that casts shadows casts them while the player's video preferences draw shadows.
pub(super) fn cast_lamp_shadows(
    video: Res<VideoPreferences>,
    mut lamps: Query<&mut PointLight, With<ShadowCasting>>,
) {
    let wanted = video.shadows != ShadowQuality::Off;
    for mut lamp in &mut lamps {
        if lamp.shadow_maps_enabled != wanted {
            lamp.shadow_maps_enabled = wanted;
        }
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

/// The cascades `quality` asks for out to `reach`: at the highest a fine one near the camera, and
/// past it as many more as reach the rest without outgrowing one another; otherwise one alone,
/// reaching no farther than close by.
fn shadow_cascades(quality: ShadowQuality, reach: ShadowReach) -> CascadeShadowConfig {
    let (reach, most) = match quality {
        ShadowQuality::High => (reach.0.0, MAX_CASCADES_PER_LIGHT),
        ShadowQuality::Low | ShadowQuality::Off => (reach.0.0.min(CLOSE_REACH.0), 1),
    };
    let near = (reach * NEAR_SHARE).min(NEAR_REACH.0);
    let past_near = ((reach / near).ln() / CASCADE_GROWTH.ln()).ceil() as usize;
    CascadeShadowConfigBuilder {
        num_cascades: (1 + past_near).clamp(most.min(2), most),
        maximum_distance: reach,
        first_cascade_far_bound: near,
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
