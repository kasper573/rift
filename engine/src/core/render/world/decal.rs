//! Marks painted on the ground: rings and discs that follow the terrain under them, for whom the
//! player is, hovers over or fights, where a walk ends and what an item lies on. A mark is one
//! quad, cut into a grid and settled on the ground by its shader from the ground's heights, so it
//! follows what it marks at the cost of a transform.

use bevy::asset::{AssetPath, embedded_path};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::camera::GroundHeights;
use super::terrain::GroundUniform;
use crate::core::geometry::Radians;
use crate::core::math::Pos;
use crate::core::units::Meters;

/// How far above the ground a decal is laid, so it draws over the terrain without fighting it.
pub const LIFT: f32 = 0.03;
/// How many quads across a mark's quad is cut into, so it settles on the ground's every sample.
const GRID: u32 = 8;

/// A mark on the ground at `at`, `size` metres across, drawn as its `look`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct GroundDecal {
    pub at: Pos<Meters>,
    pub size: Meters,
    pub yaw: Radians,
    pub look: DecalLook,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecalLook {
    /// A ring `width` of the radius thick, in `dashes` pieces when more than none.
    Ring {
        color: Color,
        width: f32,
        dashes: u32,
    },
    /// A disc fading out from its centre.
    Glow { color: Color },
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub struct DecalMaterial {
    #[uniform(0)]
    pub paint: DecalPaint,
    #[uniform(1)]
    pub ground: GroundUniform,
    #[texture(2, sample_type = "float", filterable = false)]
    pub heights: Handle<Image>,
}

#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct DecalPaint {
    pub color: Vec4,
    /// The look as the shader reads it: 0 a ring, 1 a glow.
    pub look: u32,
    pub width: f32,
    pub dashes: f32,
}

impl Material for DecalMaterial {
    fn vertex_shader() -> ShaderRef {
        shader()
    }

    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn depth_bias(&self) -> f32 {
        1.0
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

impl DecalLook {
    fn paint(self) -> DecalPaint {
        match self {
            DecalLook::Ring {
                color,
                width,
                dashes,
            } => DecalPaint {
                color: color.to_linear().to_vec4(),
                look: 0,
                width,
                dashes: dashes as f32,
            },
            DecalLook::Glow { color } => DecalPaint {
                color: color.to_linear().to_vec4(),
                look: 1,
                width: 0.0,
                dashes: 0.0,
            },
        }
    }
}

impl GroundDecal {
    /// Where the mark's quad stands: over `at`, turned by its yaw and as wide as its size, its
    /// height left to the shader.
    fn placing(&self) -> Transform {
        Transform {
            translation: Vec3::new(self.at.x, 0.0, self.at.y),
            rotation: Quat::from_rotation_y(-self.yaw.0),
            scale: Vec3::new(self.size.0, 1.0, self.size.0),
        }
    }
}

/// The quad every mark is drawn on: a metre across, cut into a grid.
#[derive(Resource)]
pub(super) struct DecalMesh(Handle<Mesh>);

impl FromWorld for DecalMesh {
    fn from_world(world: &mut World) -> DecalMesh {
        DecalMesh(
            world
                .resource_mut::<Assets<Mesh>>()
                .add(Plane3d::default().mesh().subdivisions(GRID)),
        )
    }
}

/// A decal as it is laid: its mark, and where it stands and what it is painted with once laid.
type Decal<'a> = (
    Entity,
    &'a GroundDecal,
    Option<&'a mut Transform>,
    Option<&'a MeshMaterial3d<DecalMaterial>>,
);

pub(super) fn lay_decals(
    ground: Res<GroundHeights>,
    quad: Res<DecalMesh>,
    mut decals: Query<Decal>,
    mut materials: ResMut<Assets<DecalMaterial>>,
    mut commands: Commands,
) {
    for (entity, decal, transform, material) in &mut decals {
        let placing = decal.placing();
        let paint = decal.look.paint();
        match (transform, material) {
            (Some(mut transform), Some(material)) => {
                if *transform != placing {
                    *transform = placing;
                }
                let stale = materials.get(&material.0).is_some_and(|painted| {
                    painted.paint != paint || painted.heights != ground.image
                });
                if stale && let Some(mut painted) = materials.get_mut(&material.0) {
                    painted.paint = paint;
                    painted.heights = ground.image.clone();
                }
            }
            _ => {
                commands.entity(entity).try_insert((
                    Mesh3d(quad.0.clone()),
                    MeshMaterial3d(materials.add(DecalMaterial {
                        paint,
                        ground: GroundUniform::lifted(LIFT),
                        heights: ground.image.clone(),
                    })),
                    placing,
                    Visibility::default(),
                ));
            }
        }
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("decal.wesl")).with_source("embedded"))
}
