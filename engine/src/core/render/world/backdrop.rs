//! The far end of every ray: a sphere round whatever camera looks at it, drawn in the prepass
//! alone, behind everything. A sky blended over the air writes no motion, so the frames'
//! smoothing would blend each frame's stars with whatever stood at the same spot of the screen
//! the frame before, and smear them for as long as the camera turns. The backdrop tells it how
//! the sky moved.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::material::embedded;

/// The backdrop, to stand anywhere in a world whose sky is drawn over the air.
pub fn backdrop(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BackdropMaterial>,
) -> impl Bundle + use<> {
    (
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(16, 8))),
        MeshMaterial3d(materials.add(BackdropMaterial {})),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    )
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct BackdropMaterial {}

impl Material for BackdropMaterial {
    fn vertex_shader() -> ShaderRef {
        embedded("backdrop.wesl")
    }

    fn fragment_shader() -> ShaderRef {
        embedded("backdrop.wesl")
    }

    fn prepass_vertex_shader() -> ShaderRef {
        embedded("backdrop.wesl")
    }

    fn prepass_fragment_shader() -> ShaderRef {
        embedded("backdrop.wesl")
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}
