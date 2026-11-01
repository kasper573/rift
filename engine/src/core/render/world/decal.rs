//! Marks painted on the ground: rings and discs that follow the terrain under them, for whom the
//! player is, hovers over or fights, where a walk ends and what an item lies on. A mark is a patch
//! of quads laid over the ground's samples where it stands, laid again as it moves, so it follows
//! what it marks at the cost of a small mesh.

use bevy::asset::{AssetPath, embedded_path};
use bevy::camera::primitives::MeshAabb;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::camera::GroundHeights;
use super::terrain;
use crate::core::math::Pos;
use crate::core::units::{Meters, Radians};

/// How far above the ground a decal is laid, so it draws over the terrain without fighting it.
pub const LIFT: f32 = 0.03;

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
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("decal.wesl")).with_source("embedded"),
        )
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

/// Where a mark's patch was laid, so it is laid again only when the mark moves.
#[derive(Component, Clone, Copy, PartialEq)]
pub(super) struct Laid {
    at: Pos<Meters>,
    size: Meters,
    yaw: Radians,
}

/// A decal as it is laid: its mark, and where it lies and what it is painted with once laid.
type Decal<'a> = (
    Entity,
    &'a GroundDecal,
    Option<&'a Laid>,
    Option<&'a MeshMaterial3d<DecalMaterial>>,
);

pub(super) fn lay_decals(
    ground: Res<GroundHeights>,
    decals: Query<Decal>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DecalMaterial>>,
    mut commands: Commands,
) {
    for (entity, decal, laid, material) in &decals {
        let placing = Laid {
            at: decal.at,
            size: decal.size,
            yaw: decal.yaw,
        };
        if laid != Some(&placing) {
            let patch = terrain::patch(
                &ground.heights,
                Vec2::new(decal.at.x, decal.at.y),
                Vec2::splat(decal.size.0),
                decal.yaw.0,
                LIFT,
            );
            let bounds = patch.get_aabb();
            let mut laid = commands.entity(entity);
            laid.try_insert((
                Mesh3d(meshes.add(patch)),
                placing,
                Transform::IDENTITY,
                Visibility::default(),
            ));
            if let Some(bounds) = bounds {
                laid.try_insert(bounds);
            }
        }
        let paint = decal.look.paint();
        match material {
            Some(material) => {
                if materials
                    .get(&material.0)
                    .is_some_and(|painted| painted.paint != paint)
                    && let Some(mut painted) = materials.get_mut(&material.0)
                {
                    painted.paint = paint;
                }
            }
            None => {
                commands
                    .entity(entity)
                    .try_insert(MeshMaterial3d(materials.add(DecalMaterial { paint })));
            }
        }
    }
}
