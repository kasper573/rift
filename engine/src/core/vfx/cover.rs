//! A ground's cover as a shader stage: what the material grows or lies under, laid out by the
//! engine from the paint, moved and painted by a content shader the ground names, which reads the
//! weather the engine publishes: the wind, and the bodies treading through.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use super::render::{ParamsUniform, SiteUniform, VfxShaderKey};
use crate::core::render::world::sky::Daylight;
use crate::core::render::{CameraPreferences, CameraRig};

pub type CoverMaterial = ExtendedMaterial<StandardMaterial, CoverVfx>;

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct CoverVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[uniform(102)]
    pub weather: WeatherUniform,
    pub shader: Handle<Shader>,
}

/// The weather as the cover header's `weather` reads it: the wind as a direction scaled by its
/// strength, the moment, how far through its day the game is and how clouded the sky, and the
/// bodies treading through, each a ground point and the radius it flattens.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct WeatherUniform {
    pub wind: Vec2,
    pub time: f32,
    pub treads: u32,
    pub daylight: f32,
    pub overcast: f32,
    pub tread: [Vec4; TREADS],
}

impl Default for WeatherUniform {
    fn default() -> WeatherUniform {
        WeatherUniform {
            wind: Vec2::ZERO,
            time: 0.0,
            treads: 0,
            daylight: 0.5,
            overcast: 0.0,
            tread: [Vec4::ZERO; TREADS],
        }
    }
}

/// The weather over the shown area as the client draws it: the wind as a direction over the
/// ground (x east, y south) scaled by its strength, from calm at 0 to a gale at 1, and how much
/// cloud covers the sky.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Climate {
    pub wind: Vec2,
    pub overcast: f32,
}

/// A body that bends the cover it walks through, `radius` metres around its feet.
#[derive(Component, Clone, Copy, Debug)]
pub struct Treads {
    pub radius: f32,
}

impl From<&CoverVfx> for VfxShaderKey {
    fn from(vfx: &CoverVfx) -> VfxShaderKey {
        VfxShaderKey::of(&vfx.shader)
    }
}

impl MaterialExtension for CoverVfx {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        Ok(())
    }
}

/// The material a ground's cover is drawn with: the ground's own colour under the shader, seen
/// from both sides, blended so a sheet can fade at its edge.
/// A cover's material, coloured by its vertices: blended, for a sheet or blades that write no
/// depth and take no ink, or masked, for strewn props that stand solid like any other.
pub fn material(
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
    alpha_mode: AlphaMode,
) -> CoverMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            double_sided: true,
            cull_mode: None,
            alpha_mode,
            ..default()
        },
        extension: CoverVfx {
            site,
            params,
            weather: WeatherUniform::default(),
            shader,
        },
    }
}

/// Every frame the covers learn the wind, the moment and who treads through them. With motion
/// reduced the wind drops and time stands still, while treading bodies still part the blades.
pub(super) fn blow(
    time: Res<Time>,
    preferences: Res<CameraPreferences>,
    climate: Option<Res<Climate>>,
    daylight: Option<Res<Daylight>>,
    rig: Res<CameraRig>,
    treading: Query<(&GlobalTransform, &Treads)>,
    mut materials: ResMut<Assets<CoverMaterial>>,
) {
    if materials.is_empty() {
        return;
    }
    let subject = rig.target();
    let mut treads: Vec<(f32, Vec4)> = treading
        .iter()
        .map(|(transform, treads)| {
            let at = transform.translation();
            (
                at.distance_squared(subject),
                Vec4::new(at.x, at.z, treads.radius, 0.0),
            )
        })
        .collect();
    treads.sort_by(|a, b| a.0.total_cmp(&b.0));
    treads.truncate(TREADS);
    let mut weather = WeatherUniform::default();
    for (slot, (_, tread)) in treads.iter().enumerate() {
        weather.tread[slot] = *tread;
    }
    weather.treads = treads.len() as u32;
    if let Some(daylight) = daylight {
        weather.daylight = daylight.share();
    }
    if let Some(climate) = climate {
        weather.overcast = climate.overcast;
        if !preferences.reduce_motion {
            weather.wind = climate.wind;
        }
    }
    if !preferences.reduce_motion {
        weather.time = time.elapsed_secs();
    }
    for (_, material) in materials.iter_mut() {
        if material.extension.weather != weather {
            material.extension.weather = weather;
            material.extension.site.set_time(weather.time);
        }
    }
}
