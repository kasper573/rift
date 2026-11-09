//! A ground's cover: its material, drawn through the cover shader its ground names, and where its
//! pieces strew copies of their props over the paint. Its sheet is drawn over the ground's own
//! patches, its blades from those the GPU lays out, and its copies from the scenery's arena, each
//! an instance of its prop's model, its tint and seed riding on the instance and how far up the
//! prop a vertex stands measured from the prop's height.

use bevy::gltf::GltfMaterial;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use super::lod::GRAIN_PIXELS;
use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rng};
use crate::core::units::{GROUND_ORIGIN, Meters, Radians};
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};

/// A cover's material: the engine's lit surface under the content shader its ground names.
pub type CoverMaterial = ExtendedMaterial<StandardMaterial, CoverVfx>;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct CoverVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub strewn: StrewnUniform,
    pub shader: Handle<Shader>,
}

impl From<&CoverVfx> for VfxShaderKey {
    fn from(vfx: &CoverVfx) -> VfxShaderKey {
        VfxShaderKey::of(&vfx.shader)
    }
}

impl MaterialExtension for CoverVfx {
    /// Every stage is the cover's shader, its vertices carrying their marks to the pixel.
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        descriptor.vertex.shader_defs.push("VERTEX_UVS_B".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
            fragment.shader_defs.push("VERTEX_UVS_B".into());
        }
        Ok(())
    }
}

/// What every strewn copy a material draws shares: which piece of its cover it is, the colours its
/// tint lies between, and how high its prop stands, for how far up it a vertex stands. Nothing on
/// a sheet's or blades' material, whose vertices carry their own.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct StrewnUniform {
    pub color_from: Vec4,
    pub color_to: Vec4,
    pub piece: u32,
    pub height: f32,
}

/// A strewn piece as its copies' material draws it: how it is spread, and how high its prop
/// stands above its origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrewnPiece {
    pub spread: Spread,
    pub height: Meters,
}

/// A cover's material, coloured by its vertices: blended, for a sheet or blades that write no
/// depth and take no ink, or masked, for the copies of a prop `strewn`, which stand solid like any
/// other. Both of a face's sides are drawn and lit alike by the normal its piece
/// carries, never turned over on the far side, since foliage points its normals where its light
/// should fall rather than off its faces.
pub fn material(
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
    weather: Handle<Image>,
    strewn: Option<&StrewnPiece>,
) -> CoverMaterial {
    let alpha_mode = match strewn {
        Some(_) => AlphaMode::Mask(0.5),
        None => AlphaMode::Blend,
    };
    let strewn = strewn.map_or_else(StrewnUniform::default, |strewn| StrewnUniform {
        color_from: strewn.spread.color.0.to_vec4(),
        color_to: strewn.spread.color.1.to_vec4(),
        piece: strewn.spread.piece,
        height: strewn.height.0.max(MEASURED_HEIGHT),
    });
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            double_sided: false,
            cull_mode: None,
            alpha_mode,
            ..default()
        },
        extension: CoverVfx {
            site,
            params,
            weather,
            strewn,
            shader,
        },
    }
}

/// Cells of paint along a chunk's side: a chunk strews alike whatever lies around it.
pub const CHUNK_CELLS: u32 = 16;

/// A chunk of the paint: `columns` and `rows` of half-metre cells from the cell at `first`, each
/// painted `share(column, row)` of the material, 0 to 1.
pub struct Patch<'a> {
    pub heights: &'a Heightfield,
    pub first: (u32, u32),
    pub columns: u32,
    pub rows: u32,
    pub share: &'a dyn Fn(u32, u32) -> f32,
}

/// How a piece is spread over the ground: which piece of its cover it is, how many copies a
/// square metre holds at full weight, and the scales and colours each copy is drawn between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub piece: u32,
    pub per_square_meter: f32,
    pub size: (f32, f32),
    pub color: (LinearRgba, LinearRgba),
}

impl Spread {
    /// Whether a view at `pixels_per_meter` tells the copies, `extent` across at their size, apart:
    /// whether the largest span [`GRAIN_PIXELS`] and copies stand at least that far apart.
    pub fn told_apart(&self, extent: f32, pixels_per_meter: f32) -> bool {
        extent * self.size.1 * pixels_per_meter >= GRAIN_PIXELS
            && self.pixels_apiece(pixels_per_meter) >= GRAIN_PIXELS * GRAIN_PIXELS
    }

    /// The spread as a view at `pixels_per_meter` shows it where it doesn't tell the copies,
    /// `extent` across at their size, apart: as few copies as stand [`GRAIN_PIXELS`] apart and span
    /// about as much, each grown to cover as much as those it stands in for.
    pub fn grained(&self, extent: f32, pixels_per_meter: f32) -> Spread {
        let across = extent * (self.size.0 + self.size.1) / 2.0 * pixels_per_meter;
        let apart = self.pixels_apiece(pixels_per_meter).sqrt();
        let grown = GRAIN_PIXELS / across.min(apart).max(f32::EPSILON);
        Spread {
            per_square_meter: self.per_square_meter / (grown * grown),
            size: (self.size.0 * grown, self.size.1 * grown),
            ..*self
        }
    }

    /// How many pixels of ground each copy has to itself at full weight, seen at
    /// `pixels_per_meter`: all a copy may show where copies wider than they stand apart hide
    /// each other.
    pub fn pixels_apiece(&self, pixels_per_meter: f32) -> f32 {
        pixels_per_meter * pixels_per_meter / self.per_square_meter.max(f32::EPSILON)
    }
}

/// Where a strewn copy of a prop stands on the ground, how far it is turned about the up axis and
/// grown, how far its tint lies from its spread's first colour to its second (0 to 1), how heavily
/// its cell is painted and the seed it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub at: Vec3,
    pub yaw: Radians,
    pub scale: f32,
    pub tint: f32,
    pub weight: f32,
    pub seed: f32,
}

/// What a cover's sheet lies on: the ground, a hair over it, or the area's water, wherever it
/// stands over the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetOn {
    Ground,
    Water,
}

/// How far from the shore a sheet on water tells its points they lie at most: past it, every
/// point lies this far out.
pub const SHORE_REACH: Meters = Meters(16.0);

/// Where the patch's strewn copies stand, each cell holding its share of the spread, turned any
/// way, each at its own size and tint, scattered the same way for the same `seed`, with how
/// heavily its cell is painted and a seed of its own. Empty when nothing is strewn.
pub fn strew_places(patch: &Patch, spread: &Spread, seed: u64) -> Vec<Place> {
    let mut rng = Rng::new(seed);
    places(patch, spread.per_square_meter, &mut rng)
        .into_iter()
        .map(|(at, weight)| {
            let ground = patch.heights.height_at(at).0;
            let yaw = rng.rand_float() * std::f32::consts::TAU;
            let (size, tint) = draw(spread, &mut rng);
            Place {
                at: Vec3::new(at.x, ground, at.y),
                yaw: Radians(yaw),
                scale: size,
                tint,
                weight,
                seed: rng.rand_float(),
            }
        })
        .collect()
}

/// The material a strewn piece's copies of a part of their prop's model are drawn in: `strewn`'s,
/// in the texture the part's glTF `material` draws with. None while `strewn` isn't loaded.
pub fn copy_material(
    world: &mut World,
    strewn: &Handle<CoverMaterial>,
    material: Option<&Handle<GltfMaterial>>,
) -> Option<Handle<CoverMaterial>> {
    let texture = material.and_then(|material| {
        world
            .resource::<Assets<GltfMaterial>>()
            .get(material)?
            .base_color_texture
            .clone()
    });
    world.resource_scope(|world, mut variants: Mut<CoverVariants>| {
        variants.of(
            &mut world.resource_mut::<Assets<CoverMaterial>>(),
            strewn,
            texture,
        )
    })
}

/// The cover materials made from a strewn piece's own, in each texture a prop's material draws
/// with.
#[derive(Resource, Default)]
pub(super) struct CoverVariants(HashMap<CoverVariant, Handle<CoverMaterial>>);

impl CoverVariants {
    /// `cover` in `texture` where the prop's material draws with one; none while `cover` isn't
    /// loaded.
    fn of(
        &mut self,
        covers: &mut Assets<CoverMaterial>,
        cover: &Handle<CoverMaterial>,
        texture: Option<Handle<Image>>,
    ) -> Option<Handle<CoverMaterial>> {
        let Some(texture) = texture else {
            return Some(cover.clone());
        };
        let key = CoverVariant {
            cover: cover.id(),
            texture: texture.id(),
        };
        if let Some(made) = self.0.get(&key) {
            return Some(made.clone());
        }
        let mut material = covers.get(cover)?.clone();
        material.base.base_color_texture = Some(texture);
        let made = covers.add(material);
        self.0.insert(key, made.clone());
        Some(made)
    }
}

/// Forgets the textured cover materials made for cover materials that are gone, so the pictures
/// they draw with go with them.
pub(super) fn forget_dropped(
    mut covers: MessageReader<AssetEvent<CoverMaterial>>,
    mut variants: ResMut<CoverVariants>,
) {
    let gone = super::dropped(&mut covers);
    if !gone.is_empty() {
        variants.0.retain(|key, _| !gone.contains(&key.cover));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CoverVariant {
    cover: AssetId<CoverMaterial>,
    texture: AssetId<Image>,
}

/// The least height a strewn prop is measured up, so a flat one still measures.
const MEASURED_HEIGHT: f32 = 1e-3;

/// A copy's own draw from its spread: its scale, and how far its colour lies between the spread's.
fn draw(spread: &Spread, rng: &mut Rng) -> (f32, f32) {
    let size = spread.size.0 + (spread.size.1 - spread.size.0) * rng.rand_float();
    (size, rng.rand_float())
}

/// A cell's places for `per_square_meter` of something at full weight, with the weight each
/// stands in: jittered within the cell, as many as the weight's share, the remainder by chance.
fn places(patch: &Patch, per_square_meter: f32, rng: &mut Rng) -> Vec<(Pos<Meters>, f32)> {
    let cell = 1.0 / SAMPLES_PER_METER as f32;
    let per_cell = per_square_meter * cell * cell;
    let mut placed = Vec::new();
    for row in patch.first.1..patch.first.1 + patch.rows {
        for column in patch.first.0..patch.first.0 + patch.columns {
            let weight = (patch.share)(column, row).clamp(0.0, 1.0);
            let wanted = per_cell * weight;
            let count = wanted.floor() as u32 + u32::from(rng.rand_float() < wanted.fract());
            for _ in 0..count {
                placed.push((
                    Pos::new(
                        GROUND_ORIGIN.x + (column as f32 + rng.rand_float()) * cell,
                        GROUND_ORIGIN.y + (row as f32 + rng.rand_float()) * cell,
                    ),
                    weight,
                ));
            }
        }
    }
    placed
}
