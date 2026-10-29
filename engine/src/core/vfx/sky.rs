//! An area's sky as a shader stage: a dome round the camera at the far end of every ray, blended
//! over the air, painted by a content shader the area names with what lies beyond the air (the
//! weather's clouds, from the field of cloud drifting over the area, the moon and the stars), and
//! told where the sun or the moon stands, how deep the night is and the weather the covers read.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use crate::core::render::world::clouds::{CLOUD_BASE, CLOUD_TILE};

use super::render::{ParamsUniform, SiteUniform, VfxShaderKey};

/// A shader painting the sky's dome, with the parameters it is filled with, the weather it reads,
/// the sky's lights, and the field of cloud drifting over the area.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(VfxShaderKey)]
pub struct SkyMaterial {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub heavens: Heavens,
    #[texture(104)]
    #[sampler(105)]
    pub clouds: Handle<Image>,
    pub shader: Handle<Shader>,
}

/// The sky's lights as the sky header reads them: the unit way toward the light the world is lit
/// by (the sun, or by night the full moon), how much of it shines (none below the horizon), its
/// colour, the unit way toward the moon, how far into the night the sky is, 0 day to 1 full night,
/// and the field of cloud: the height it lies at, the ground a tile of it covers and how far the
/// wind has carried it.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct Heavens {
    pub toward: Vec3,
    pub shining: f32,
    pub color: Vec4,
    pub moon: Vec3,
    pub night: f32,
    pub cloud_base: f32,
    pub cloud_tile: f32,
    pub drift: Vec2,
}

impl Default for Heavens {
    fn default() -> Heavens {
        Heavens {
            toward: Vec3::Y,
            shining: 0.0,
            color: Vec4::ZERO,
            moon: Vec3::NEG_Y,
            night: 0.0,
            cloud_base: CLOUD_BASE.0,
            cloud_tile: CLOUD_TILE.0,
            drift: Vec2::ZERO,
        }
    }
}

/// The dome the sky is painted on.
#[derive(Component)]
pub struct SkyDome;

impl From<&SkyMaterial> for VfxShaderKey {
    fn from(sky: &SkyMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&sky.shader)
    }
}

impl Material for SkyMaterial {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The dome the sky drawn with `material` is painted on, round whatever camera looks at it.
pub fn dome(meshes: &mut Assets<Mesh>, material: Handle<SkyMaterial>) -> impl Bundle {
    (
        SkyDome,
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 24))),
        MeshMaterial3d(material),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    )
}

impl Heavens {
    /// The world lit from `toward` in `color`, `strength` times as strong as the full sun, shining
    /// fully once the light clears the horizon; the moon standing toward `moon`, `night` of the
    /// way into the night, and the clouds carried `drift` along.
    pub fn new(
        toward: Vec3,
        color: Color,
        strength: f32,
        moon: Vec3,
        night: f32,
        drift: Vec2,
    ) -> Heavens {
        let linear = color.to_linear() * strength;
        Heavens {
            toward,
            shining: (toward.y * 10.0 + 0.5).clamp(0.0, 1.0),
            color: Vec4::new(linear.red, linear.green, linear.blue, 1.0),
            moon,
            night,
            drift,
            ..Heavens::default()
        }
    }
}
