//! An area's rain as a shader stage: shells round the eye, each a sphere the rain is drawn on,
//! blended over the world in front of which it falls, painted by a content shader the area's
//! climate names. Beside it, what every shader of the world reads of the rain: how much falls over
//! each stretch of the ground round the camera's subject and how wet it lies, and where the roofs
//! and canopies round the camera shelter the ground from it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use super::sky::Heavens;
use super::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};
use crate::core::render::camera::NEAR_PLANE;
use crate::core::units::Meters;

/// How near the eye a shell may lie: wholly beyond the camera's near plane, which clips it, though
/// the facets of its sphere dip inside its radius.
pub const NEAREST_SHELL: Meters = Meters(NEAR_PLANE.0 * 2.0);

/// How many cells of the rain field lie each way round the camera's subject, and how wide each
/// is: far enough for the rain seen on the horizon to fall where the field says.
pub const RAIN_SIDE: usize = 64;
pub const RAIN_CELL: Meters = Meters(128.0);

/// How many cells of the shelter lie each way round the camera, and how wide each is: as far as
/// rain is drawn falling, and fine enough to keep it off the ground under a roof.
pub const SHELTER_SIDE: usize = 128;
pub const SHELTER_CELL: Meters = Meters(1.0);

/// The top of a cell of the shelter where nothing stands over the ground.
pub const OPEN_TOP: f32 = -1.0e6;

/// How far under a shelter's top a point lies sheltered, in metres: the top itself stands in the
/// rain.
pub const SHELTER_DEPTH: f32 = 0.3;

/// The rain over the ground round the camera's subject: [`RAIN_SIDE`] cells each way from the
/// north-west `corner` (x east, y south), [`RAIN_CELL`] wide, row by row from the north; none held
/// while it is empty.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RainField {
    pub corner: Vec2,
    pub cells: Vec<RainCell>,
}

impl RainField {
    /// The rain over the cell holding the ground point `at`, none off the field.
    pub fn cell_at(&self, at: Vec2) -> Option<RainCell> {
        let cell = ((at - self.corner) / RAIN_CELL.0).floor();
        let side = RAIN_SIDE as f32;
        let inside = cell.cmpge(Vec2::ZERO).all() && cell.cmplt(Vec2::splat(side)).all();
        inside
            .then(|| {
                self.cells
                    .get(cell.y as usize * RAIN_SIDE + cell.x as usize)
            })
            .flatten()
            .copied()
    }

    /// How hard it rains over the cell holding the ground point `at`, none off the field.
    pub fn rain_at(&self, at: Vec2) -> f32 {
        self.cell_at(at).map_or(0.0, |cell| cell.rain)
    }
}

impl Shelter {
    /// Whether `point` lies under what stands over its cell, deeper than its top itself, which
    /// stands in the rain.
    pub fn covers(&self, point: Vec3) -> bool {
        let cell = ((point.xz() - self.corner) / SHELTER_CELL.0).floor();
        let side = SHELTER_SIDE as f32;
        let inside = cell.cmpge(Vec2::ZERO).all() && cell.cmplt(Vec2::splat(side)).all();
        inside
            .then(|| {
                self.tops
                    .get(cell.y as usize * SHELTER_SIDE + cell.x as usize)
            })
            .flatten()
            .is_some_and(|top| point.y < top - SHELTER_DEPTH)
    }
}

/// The rain over a cell: the share of the downpour the lie of the land gives it, how wet its
/// ground lies and how hard it rains there, each 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RainCell {
    pub share: f32,
    pub wetness: f32,
    pub rain: f32,
}

/// Where the ground round the camera lies under shelter from the rain: the height of the top of
/// what stands over each of [`SHELTER_SIDE`] cells each way from the north-west `corner`,
/// [`SHELTER_CELL`] wide, row by row from the north, or [`OPEN_TOP`] under the open sky; none held
/// while it is empty.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Shelter {
    pub corner: Vec2,
    pub tops: Vec<f32>,
}

/// A shader drawing the rain on its shells, with the parameters it is filled with, the weather it
/// reads, the sky's lights and the field of cloud drifting over the area, as the sky's dome has.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(VfxShaderKey)]
pub struct RainMaterial {
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

impl From<&RainMaterial> for VfxShaderKey {
    fn from(rain: &RainMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&rain.shader)
    }
}

impl Material for RainMaterial {
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

/// The shells the rain drawn with `material` falls on, a sphere at each of `shells` round
/// whatever camera looks at them, in one mesh from the farthest in, so each pixel blends the
/// nearer rain over the farther.
pub fn shells(
    meshes: &mut Assets<Mesh>,
    shells: &[Meters],
    material: Handle<RainMaterial>,
) -> impl Bundle {
    (
        Mesh3d(meshes.add(shell_mesh(shells))),
        MeshMaterial3d(material),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    )
}

fn shell_mesh(shells: &[Meters]) -> Mesh {
    let mut farthest_first = shells.to_vec();
    farthest_first.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sphere = Sphere::new(1.0).mesh().uv(32, 16);
    let unit: Vec<[f32; 3]> = sphere
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|positions| positions.as_float3())
        .map(<[[f32; 3]]>::to_vec)
        .unwrap_or_default();
    let unit_indices: Vec<u32> = sphere
        .indices()
        .map(|indices| indices.iter().map(|index| index as u32).collect())
        .unwrap_or_default();
    let mut positions = Vec::with_capacity(unit.len() * farthest_first.len());
    let mut indices = Vec::with_capacity(unit_indices.len() * farthest_first.len());
    for radius in farthest_first {
        let first = positions.len() as u32;
        positions.extend(
            unit.iter()
                .map(|at| (Vec3::from_array(*at) * radius.0).to_array()),
        );
        indices.extend(unit_indices.iter().map(|index| first + index));
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices))
}
