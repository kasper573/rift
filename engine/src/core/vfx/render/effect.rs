//! The material every effect layer is drawn with: the layer's effect shader over a shape of its
//! own, a hull just off the surfaces of what it coats, or a patch of ground, unlit and laid over
//! what lies behind it.

use bevy::asset::{AssetPath, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Face,
    RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderDefVal, ShaderRef};

use super::super::VfxBlend;
use super::super::uniform::{EffectFrame, ParamsUniform, VfxShaderKey};

#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(EffectKey)]
pub struct EffectMaterial {
    #[uniform(0)]
    pub frame: EffectFrame,
    #[uniform(1)]
    pub params: ParamsUniform,
    pub shader: Handle<Shader>,
    pub blend: VfxBlend,
    /// Drawn over a thing's surface, seen only from outside it.
    pub coat: bool,
}

/// What an effect's pipeline is built for: its shader, how it blends, and whether it coats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectKey {
    shader: VfxShaderKey,
    add: bool,
    coat: bool,
}

impl From<&EffectMaterial> for EffectKey {
    fn from(material: &EffectMaterial) -> EffectKey {
        EffectKey {
            shader: VfxShaderKey::of(&material.shader),
            add: material.blend == VfxBlend::Add,
            coat: material.coat,
        }
    }
}

impl Material for EffectMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("effect_vertex.wesl")).with_source("embedded"),
        )
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            VfxBlend::Add => AlphaMode::Add,
            VfxBlend::Alpha => AlphaMode::Blend,
        }
    }

    fn enable_shadows() -> bool {
        false
    }

    fn enable_prepass() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let EffectKey { shader, add, coat } = key.bind_group_data;
        descriptor.primitive.cull_mode = coat.then_some(Face::Back);
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader.handle();
            if add {
                fragment.shader_defs.push(ShaderDefVal::from("FX_ADD"));
            }
            // Light adds to what lies behind it and leaves its alpha be, so a glow over the sky
            // keeps the sky's.
            let blend = if add {
                BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent {
                        src_factor: BlendFactor::Zero,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                }
            } else {
                BlendState::ALPHA_BLENDING
            };
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend);
            }
        }
        Ok(())
    }
}
