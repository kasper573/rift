//! The ground as meshes of its chunks, a vertex at every height sample a chunk keeps, each
//! painted by the ground material with the terrains laid under it, every terrain its colour under
//! the picture it repeats over the ground, blended across the paint's pixels as the paint blends.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDataOrder, TextureDimension, TextureFormat,
    TextureViewDescriptor, TextureViewDimension,
};
use bevy::shader::ShaderRef;
use image::RgbaImage;
use image::imageops::{self, FilterType};

use super::material::{SurfaceFinish, dielectric, embedded};
use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use crate::core::units::{GROUND_ORIGIN, Meters};

pub type GroundMaterial = ExtendedMaterial<StandardMaterial, GroundLook>;

/// The side of every picture laid over the ground, in pixels: all one size, so the ground draws
/// them from one stack.
pub const GROUND_PICTURE: u32 = 512;
/// How steeply a picture may be seen and stay sharp, in times the texels a pixel reads along the
/// ground.
const SHARPNESS: u16 = 16;

/// How one terrain paints the ground: its colour, and the picture laid over it, if any, repeating
/// every `tile` metres, its shades multiplied into the colour.
#[derive(Clone, Debug, PartialEq)]
pub struct GroundLayer {
    pub color: LinearRgba,
    pub picture: Option<(String, Meters)>,
}

/// What the ground is painted from: the terrains' pictures as one stack, white at the bottom for
/// a terrain that has none; for each terrain its colour and tile in the first row and its
/// picture's place in the stack in the second; and the paint of the chunk the material draws,
/// a terrain's place in the area's list for each pixel, with where its pixels lie.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub struct GroundLook {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub pictures: Handle<Image>,
    #[texture(102, sample_type = "u_int")]
    pub paint: Handle<Image>,
    #[texture(103, sample_type = "float", filterable = false)]
    pub layers: Handle<Image>,
    #[uniform(104)]
    pub grid: PaintGrid,
}

impl MaterialExtension for GroundLook {
    fn fragment_shader() -> ShaderRef {
        embedded("ground.wesl")
    }
}

/// Where a paint texture's pixels lie on the ground: `pixels_per_metre` to the metre from the
/// edge of the first at `origin`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct PaintGrid {
    pub origin: Vec2,
    pub pixels_per_metre: f32,
}

/// A ground whose pictures are still loading: its layers, and each picture in their order.
#[derive(Component)]
pub struct GroundPictures {
    layers: Vec<GroundLayer>,
    loading: Vec<Handle<Image>>,
}

/// The material every chunk of the ground is painted from in `layers`, the pictures white until
/// they load, and what waits for them to. It paints nothing itself: a chunk's material is
/// [`chunk_material`], sharing its pictures and layers.
pub fn ground_material(
    layers: &[GroundLayer],
    finish: &SurfaceFinish,
    assets: &AssetServer,
    images: &mut Assets<Image>,
) -> (GroundMaterial, GroundPictures) {
    let loading = layers
        .iter()
        .filter_map(|layer| Some(assets.load(layer.picture.as_ref()?.0.clone())))
        .collect();
    let material = ExtendedMaterial {
        base: dielectric(StandardMaterial::default(), finish),
        extension: GroundLook {
            pictures: images.add(stack(1, &[])),
            paint: images.add(paint_image(&[0], 1, 1)),
            layers: images.add(layers_image(layers, &vec![0; layers.len()])),
            grid: PaintGrid {
                origin: Vec2::ZERO,
                pixels_per_metre: 1.0,
            },
        },
    };
    let waiting = GroundPictures {
        layers: layers.to_vec(),
        loading,
    };
    (material, waiting)
}

/// A chunk of ground's samples: where they begin, how many lie along its side and how many
/// samples apart they are taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkSamples {
    pub first: (u32, u32),
    pub side: u32,
    pub stride: u32,
}

/// The material a chunk is painted with: the ground's pictures and layers, and the paint under
/// the chunk at the chunk's stride, a pixel beyond each edge so the paint blends across into the
/// chunks beside it.
pub fn chunk_material(
    ground: &GroundMaterial,
    paint: &PaintMap,
    chunk: ChunkSamples,
    images: &mut Assets<Image>,
) -> GroundMaterial {
    let (image, grid) = chunk_paint(paint, chunk);
    ExtendedMaterial {
        base: ground.base.clone(),
        extension: GroundLook {
            pictures: ground.extension.pictures.clone(),
            paint: images.add(image),
            layers: ground.extension.layers.clone(),
            grid,
        },
    }
}

/// Lays `paint` anew under the chunk `material` draws.
pub fn repaint_chunk(
    material: &mut GroundMaterial,
    paint: &PaintMap,
    chunk: ChunkSamples,
    images: &mut Assets<Image>,
) {
    let (image, grid) = chunk_paint(paint, chunk);
    material.extension.paint = images.add(image);
    material.extension.grid = grid;
}

/// Gives a chunk's material the ground's pictures and layers once they differ, as they do once
/// the pictures have loaded.
pub fn share_pictures(material: &mut GroundMaterial, ground: &GroundMaterial) -> bool {
    let (pictures, layers) = (&ground.extension.pictures, &ground.extension.layers);
    if material.extension.pictures == *pictures && material.extension.layers == *layers {
        return false;
    }
    material.extension.pictures = pictures.clone();
    material.extension.layers = layers.clone();
    true
}

/// Stacks every waiting ground's pictures once they have all loaded, or failed, which leaves
/// that terrain unpictured.
pub(super) fn stack_pictures(
    waiting: Query<(Entity, &GroundPictures, &MeshMaterial3d<GroundMaterial>)>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GroundMaterial>>,
    mut commands: Commands,
) {
    for (entity, ground, material) in &waiting {
        if ground
            .loading
            .iter()
            .any(|picture| images.get(picture).is_none() && !assets.load_state(picture).is_failed())
        {
            continue;
        }
        commands.entity(entity).remove::<GroundPictures>();
        let Some(mut material) = materials.get_mut(&material.0) else {
            continue;
        };
        let mut loaded = ground.loading.iter().map(|picture| {
            let image = images.get(picture)?.clone().try_into_dynamic().ok()?;
            Some(imageops::resize(
                &image.to_rgba8(),
                GROUND_PICTURE,
                GROUND_PICTURE,
                FilterType::Triangle,
            ))
        });
        let mut stacked = Vec::new();
        let places: Vec<u32> = ground
            .layers
            .iter()
            .map(|layer| {
                let picture = layer.picture.as_ref().and_then(|_| loaded.next().flatten());
                picture.map_or(0, |picture| {
                    stacked.push(picture);
                    stacked.len() as u32
                })
            })
            .collect();
        material.extension.pictures = images.add(stack(GROUND_PICTURE, &stacked));
        material.extension.layers = images.add(layers_image(&ground.layers, &places));
    }
}

/// The paint under a chunk, a pixel per `stride` of the paint's from a pixel before the chunk's
/// first sample to one past its last, each the terrain at the middle of the pixels it stands for.
fn chunk_paint(paint: &PaintMap, chunk: ChunkSamples) -> (Image, PaintGrid) {
    let stride = chunk.stride.max(1);
    let across = chunk.side.div_ceil(stride) + 2;
    let first = (
        chunk.first.0 as i64 - stride as i64,
        chunk.first.1 as i64 - stride as i64,
    );
    let middle = (stride / 2) as i64;
    let pixel = |column: u32, row: u32| {
        let at = (
            first.0 + column as i64 * stride as i64 + middle,
            first.1 + row as i64 * stride as i64 + middle,
        );
        paint.index(at.0.max(0) as u32, at.1.max(0) as u32)
    };
    let data: Vec<u8> = (0..across)
        .flat_map(|row| (0..across).map(move |column| (column, row)))
        .map(|(column, row)| pixel(column, row))
        .collect();
    let step = stride as f32 / SAMPLES_PER_METER as f32;
    let grid = PaintGrid {
        origin: Vec2::new(
            GROUND_ORIGIN.x + first.0 as f32 / SAMPLES_PER_METER as f32,
            GROUND_ORIGIN.y + first.1 as f32 / SAMPLES_PER_METER as f32,
        ),
        pixels_per_metre: 1.0 / step,
    };
    (paint_image(&data, across, across), grid)
}

/// A paint as a picture a pixel per entry, each the terrain's place in the area's list.
fn paint_image(data: &[u8], columns: u32, rows: u32) -> Image {
    Image::new(
        Extent3d {
            width: columns.max(1),
            height: rows.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data.to_vec(),
        TextureFormat::R8Uint,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Each layer's colour and tile in the first row, and the place of its picture in the stack in
/// the second.
fn layers_image(layers: &[GroundLayer], places: &[u32]) -> Image {
    let looks = layers.iter().map(|layer| {
        let tile = layer.picture.as_ref().map_or(1.0, |(_, tile)| tile.0);
        [layer.color.red, layer.color.green, layer.color.blue, tile]
    });
    let places = places.iter().map(|&place| [place as f32, 0.0, 0.0, 0.0]);
    let data = looks
        .chain(places)
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    Image::new(
        Extent3d {
            width: layers.len().max(1) as u32,
            height: 2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// `pictures`, each `side` pixels square, stacked over a white one, every one with its smaller
/// copies down to a pixel, for the ground to draw sharp up close and calm far off; repeating
/// every way.
fn stack(side: u32, pictures: &[RgbaImage]) -> Image {
    let white = RgbaImage::from_pixel(side, side, image::Rgba([255; 4]));
    let levels = side.max(1).ilog2() + 1;
    let layers = pictures.len() as u32 + 1;
    let data = std::iter::once(&white)
        .chain(pictures)
        .flat_map(|picture| {
            (0..levels).scan(picture.clone(), |level, _| {
                let bytes = level.as_raw().clone();
                let half = (level.width() / 2).max(1);
                *level = imageops::resize(level, half, half, FilterType::Triangle);
                Some(bytes)
            })
        })
        .flatten()
        .collect();
    let mut image = Image::new_uninit(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.data_order = TextureDataOrder::LayerMajor;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        anisotropy_clamp: SHARPNESS,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// The mesh of the chunk of ground `columns`×`rows` samples from sample `first`, taking every
/// `stride`-th sample, in the world's metres from the ground's corner, with a skirt hanging from
/// its edges so a coarser chunk beside it shows no gap where their heights differ.
pub fn chunk(
    heights: &Heightfield,
    first: (u32, u32),
    columns: u32,
    rows: u32,
    stride: u32,
) -> Mesh {
    let step = stride as f32 / SAMPLES_PER_METER as f32;
    let (across, down) = (columns.div_ceil(stride) + 1, rows.div_ceil(stride) + 1);
    let last = (first.0 + columns, first.1 + rows);
    let sample_at = |column: u32, row: u32| {
        (
            (first.0 + column * stride).min(last.0),
            (first.1 + row * stride).min(last.1),
        )
    };
    let skirt = 2 * (across + down);
    let mut positions = Vec::with_capacity((across * down + skirt) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for row in 0..down {
        for column in 0..across {
            let (c, r) = sample_at(column, row);
            let at = Heightfield::sample_pos(c, r);
            positions.push([at.x, heights.sample(c, r).0, at.y]);
            let east =
                heights.sample(c + stride, r).0 - heights.sample(c.saturating_sub(stride), r).0;
            let south =
                heights.sample(c, r + stride).0 - heights.sample(c, r.saturating_sub(stride)).0;
            normals.push(Vec3::new(-east, 2.0 * step, -south).normalize().to_array());
            uvs.push([
                column as f32 / (across - 1).max(1) as f32,
                row as f32 / (down - 1).max(1) as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity((((across - 1) * (down - 1) + skirt) * 6) as usize);
    for row in 0..down - 1 {
        for column in 0..across - 1 {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    let rim: Vec<u32> = (0..across)
        .chain((1..down).map(|row| row * across + across - 1))
        .chain(
            (0..across - 1)
                .rev()
                .map(|column| (down - 1) * across + column),
        )
        .chain((1..down - 1).rev().map(|row| row * across))
        .collect();
    let hem = positions.len() as u32;
    for &vertex in &rim {
        let [x, y, z] = positions[vertex as usize];
        positions.push([x, y - SKIRT, z]);
        normals.push(normals[vertex as usize]);
        uvs.push(uvs[vertex as usize]);
    }
    for (index, &vertex) in rim.iter().enumerate() {
        let next = rim[(index + 1) % rim.len()];
        let (low, low_next) = (hem + index as u32, hem + ((index + 1) % rim.len()) as u32);
        indices.extend([vertex, low, next, next, low, low_next]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// How far a chunk's skirt hangs below its edge: past any height step between the samples a
/// coarser neighbour skips.
const SKIRT: f32 = 4.0;

/// A patch of ground `size` metres across, centred on `at` and turned by `yaw`, as a grid of
/// quads half a metre apart laid `lift` above the terrain, for what is drawn on the ground. Its uv
/// runs 0..1 over the patch, y down as a picture's does.
pub fn patch(heights: &Heightfield, at: Vec2, size: Vec2, yaw: f32, lift: f32) -> Mesh {
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let across = ((size.x / step).ceil() as u32).max(1) + 1;
    let down = ((size.y / step).ceil() as u32).max(1) + 1;
    let (sin, cos) = yaw.sin_cos();
    let mut positions = Vec::with_capacity((across * down) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for row in 0..down {
        for column in 0..across {
            let u = column as f32 / (across - 1) as f32;
            let v = row as f32 / (down - 1) as f32;
            let local = Vec2::new((u - 0.5) * size.x, (v - 0.5) * size.y);
            let turned = Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos);
            let ground = at + turned;
            let height = heights
                .height_at(crate::core::math::Pos::new(ground.x, ground.y))
                .0;
            positions.push([ground.x, height + lift, ground.y]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity(((across - 1) * (down - 1) * 6) as usize);
    for row in 0..down - 1 {
        for column in 0..across - 1 {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}
