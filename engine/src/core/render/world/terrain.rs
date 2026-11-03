//! The ground as meshes of its chunks, a vertex at every height sample a chunk keeps, each
//! painted by the ground material with the terrains laid under it, every terrain its colour under
//! the picture it repeats over the ground, blended across the paint's pixels as the paint blends.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshTag;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
    TextureDataOrder, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
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
/// picture's place in the stack in the second; and the paint of every chunk that stands, a
/// terrain's place in the area's list for each pixel, each chunk's on the layer of `paints` its
/// [`MeshTag`] names, with where its pixels lie in the pixel of `grids` it names.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub struct GroundLook {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub pictures: Handle<Image>,
    #[texture(102, dimension = "2d_array", sample_type = "u_int")]
    pub paints: Handle<Image>,
    #[texture(103, sample_type = "float", filterable = false)]
    pub layers: Handle<Image>,
    #[texture(104, sample_type = "float", filterable = false)]
    pub grids: Handle<Image>,
}

impl MaterialExtension for GroundLook {
    fn fragment_shader() -> ShaderRef {
        embedded("ground.wesl")
    }
}

/// Where a chunk's paint pixels lie on the ground: `pixels_per_metre` to the metre from the edge
/// of the first at `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PaintGrid {
    origin: Vec2,
    pixels_per_metre: f32,
}

/// A ground whose pictures are still loading, or being stacked once they have: its layers, and
/// each picture in their order.
#[derive(Component)]
pub struct GroundPictures {
    layers: Vec<GroundLayer>,
    loading: Vec<Handle<Image>>,
    stacking: Option<Task<(Image, Image)>>,
}

/// The material every chunk of the ground is painted with in `layers`, the pictures white until
/// they load, and what waits for them to. It paints no chunk until [`ChunkPaints`] lays its paint,
/// `paint_side` pixels square.
pub fn ground_material(
    layers: &[GroundLayer],
    finish: &SurfaceFinish,
    assets: &AssetServer,
    images: &mut Assets<Image>,
    paint_side: u32,
) -> (GroundMaterial, GroundPictures) {
    let loading = layers
        .iter()
        .filter_map(|layer| Some(assets.load(layer.picture.as_ref()?.0.clone())))
        .collect();
    let material = ExtendedMaterial {
        base: dielectric(StandardMaterial::default(), finish),
        extension: GroundLook {
            pictures: images.add(stack(1, &[])),
            paints: images.add(paints_image(paint_side, FIRST_LAYERS, Vec::new())),
            layers: images.add(layers_image(layers, &vec![0; layers.len()])),
            grids: images.add(grids_image(FIRST_LAYERS, Vec::new())),
        },
    };
    let waiting = GroundPictures {
        layers: layers.to_vec(),
        loading,
        stacking: None,
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

/// The paint under each chunk of a ground that stands, each on a layer of the stack its
/// material reads, at the place the chunk's [`MeshTag`] names: so every chunk draws with the
/// ground's one material, which the renderer then draws them all with alike. A chunk's paint goes
/// to the GPU as its layer alone, through [`LaidPaints`]; the stack is kept here as well, to start
/// a larger one from when more chunks stand than it has room for.
pub struct ChunkPaints {
    material: Handle<GroundMaterial>,
    /// The pixels along a chunk's paint, every chunk's alike.
    side: u32,
    free: Vec<u32>,
    taken: u32,
    paints: Vec<u8>,
    grids: Vec<u8>,
}

impl ChunkPaints {
    /// The paints of the ground painted with `material`, as [`ground_material`] made it for
    /// `side` pixels square.
    pub fn new(material: Handle<GroundMaterial>, side: u32) -> ChunkPaints {
        ChunkPaints {
            material,
            side,
            free: Vec::new(),
            taken: 0,
            paints: vec![0; (side * side * FIRST_LAYERS) as usize],
            grids: vec![0; FIRST_LAYERS as usize * GRID_BYTES],
        }
    }

    pub fn material(&self) -> &Handle<GroundMaterial> {
        &self.material
    }

    /// Lays a chunk's `paint` on a free layer, the one the tag returned names.
    pub fn lay(&mut self, world: &mut World, paint: &ChunkPaint) -> MeshTag {
        let layer = self.free.pop().unwrap_or_else(|| {
            self.taken += 1;
            self.taken - 1
        });
        self.write(world, paint, layer);
        MeshTag::new(layer)
    }

    /// Lays a chunk's `paint` anew on the layer `tag` names.
    pub fn relay(&mut self, world: &mut World, paint: &ChunkPaint, tag: &MeshTag) {
        self.write(world, paint, tag.value);
    }

    /// Frees the layer `tag` names, for the next chunk to stand.
    pub fn lift(&mut self, tag: &MeshTag) {
        self.free.push(tag.value);
    }

    fn layers(&self) -> u32 {
        (self.grids.len() / GRID_BYTES) as u32
    }

    fn write(&mut self, world: &mut World, paint: &ChunkPaint, layer: u32) {
        if layer >= self.layers() {
            let layers = (layer + 1).next_power_of_two().max(FIRST_LAYERS);
            self.paints
                .resize((self.side * self.side * layers) as usize, 0);
            self.grids.resize(layers as usize * GRID_BYTES, 0);
            let mut images = world.resource_mut::<Assets<Image>>();
            let paints = images.add(paints_image(self.side, layers, self.paints.clone()));
            let grids = images.add(grids_image(layers, self.grids.clone()));
            if let Some(mut material) = world
                .resource_mut::<Assets<GroundMaterial>>()
                .get_mut(&self.material)
            {
                material.extension.paints = paints;
                material.extension.grids = grids;
            }
        }
        let side = self.side as usize;
        let ChunkPaint { data, across, grid } = paint;
        let first = layer as usize * side * side;
        let laid = &mut self.paints[first..first + side * side];
        for (row, pixels) in data.chunks(*across as usize).enumerate().take(side) {
            let width = pixels.len().min(side);
            laid[row * side..row * side + width].copy_from_slice(&pixels[..width]);
        }
        let at = layer as usize * GRID_BYTES;
        let pixel = [grid.origin.x, grid.origin.y, grid.pixels_per_metre, 0.0];
        for (index, value) in pixel.iter().enumerate() {
            self.grids[at + index * 4..at + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let Some(material) = world
            .resource::<Assets<GroundMaterial>>()
            .get(&self.material)
        else {
            return;
        };
        let layer = LaidLayer {
            ground: self.material.id(),
            paints: material.extension.paints.id(),
            grids: material.extension.grids.id(),
            layer,
            side: self.side,
            paint: self.paints[first..first + side * side].to_vec(),
            grid: self.grids[at..at + GRID_BYTES].to_vec(),
        };
        if let Some(mut laid) = world.get_resource_mut::<LaidPaints>() {
            laid.0.push(layer);
        }
    }
}

/// The layers of the grounds' paints laid since the render world last took them, each to be
/// written into its texture alone rather than the whole stack sent again.
#[derive(Resource, Default)]
pub struct LaidPaints(Vec<LaidLayer>);

/// One chunk's paint and grid, to write into the layer it lies on, of the textures the ground
/// painted with `ground` held as it was laid.
struct LaidLayer {
    ground: AssetId<GroundMaterial>,
    paints: AssetId<Image>,
    grids: AssetId<Image>,
    layer: u32,
    side: u32,
    paint: Vec<u8>,
    grid: Vec<u8>,
}

/// Takes the layers laid in the main world into the render world, and forgets those waiting on
/// the textures of a ground that is gone.
pub(super) fn extract_laid(mut main: ResMut<MainWorld>, mut laid: ResMut<LaidPaints>) {
    if let Some(mut taken) = main.get_resource_mut::<LaidPaints>() {
        laid.0.append(&mut taken.0);
    }
    if let Some(grounds) = main.get_resource::<Assets<GroundMaterial>>() {
        laid.0.retain(|layer| grounds.contains(layer.ground));
    }
}

/// Writes each layer laid into its textures once they are on the GPU, keeping it until they
/// are. A layer laid for textures a ground has since outgrown is dropped: their larger successors
/// were made holding it.
pub(super) fn write_laid(
    mut laid: ResMut<LaidPaints>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    let latest: HashMap<AssetId<GroundMaterial>, AssetId<Image>> = laid
        .0
        .iter()
        .map(|layer| (layer.ground, layer.paints))
        .collect();
    laid.0.retain(|layer| {
        if latest.get(&layer.ground) != Some(&layer.paints) {
            return false;
        }
        let (Some(paints), Some(grids)) = (images.get(layer.paints), images.get(layer.grids))
        else {
            return true;
        };
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &paints.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: 0,
                    y: 0,
                    z: layer.layer,
                },
                aspect: TextureAspect::All,
            },
            &layer.paint,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layer.side),
                rows_per_image: Some(layer.side),
            },
            Extent3d {
                width: layer.side,
                height: layer.side,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &grids.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: layer.layer,
                    y: 0,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &layer.grid,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GRID_BYTES as u32),
                rows_per_image: Some(1),
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        false
    });
}

/// How many chunks' paint a ground holds room for at first; it doubles whenever more stand.
const FIRST_LAYERS: u32 = 64;
/// The bytes of a chunk's grid: its pixel of four floats.
const GRID_BYTES: usize = 16;

/// Stacks every waiting ground's pictures once they have all loaded, or failed, which leaves
/// that terrain unpictured: on a task, so the frames keep coming while each is scaled down level
/// by level.
pub(super) fn stack_pictures(
    mut waiting: Query<(Entity, &mut GroundPictures, &MeshMaterial3d<GroundMaterial>)>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GroundMaterial>>,
    mut commands: Commands,
) {
    for (entity, mut ground, material) in &mut waiting {
        if let Some(task) = &mut ground.stacking {
            let Some((pictures, layers)) = check_ready(task) else {
                continue;
            };
            commands.entity(entity).remove::<GroundPictures>();
            if let Some(mut material) = materials.get_mut(&material.0) {
                material.extension.pictures = images.add(pictures);
                material.extension.layers = images.add(layers);
            }
            continue;
        }
        if ground
            .loading
            .iter()
            .any(|picture| images.get(picture).is_none() && !assets.load_state(picture).is_failed())
        {
            continue;
        }
        let loaded: Vec<Option<Image>> = ground
            .loading
            .iter()
            .map(|picture| images.get(picture).cloned())
            .collect();
        let layers = ground.layers.clone();
        ground.stacking =
            Some(AsyncComputeTaskPool::get().spawn(async move { stacked(&layers, loaded) }));
    }
}

/// The pictures of `layers`, `loaded` in their order, as one stack, and the image saying where in
/// it each layer's lies.
fn stacked(layers: &[GroundLayer], loaded: Vec<Option<Image>>) -> (Image, Image) {
    let mut loaded = loaded.into_iter().map(|image| {
        let image = image?.try_into_dynamic().ok()?;
        Some(imageops::resize(
            &image.to_rgba8(),
            GROUND_PICTURE,
            GROUND_PICTURE,
            FilterType::Triangle,
        ))
    });
    let mut stacked = Vec::new();
    let places: Vec<u32> = layers
        .iter()
        .map(|layer| {
            let picture = layer.picture.as_ref().and_then(|_| loaded.next().flatten());
            picture.map_or(0, |picture| {
                stacked.push(picture);
                stacked.len() as u32
            })
        })
        .collect();
    (
        stack(GROUND_PICTURE, &stacked),
        layers_image(layers, &places),
    )
}

/// The paint under a chunk, as [`ChunkPaints`] lays it: a pixel per `stride` of the paint's from a
/// pixel before the chunk's first sample to one past its last, `across` along each side, each the
/// terrain at the middle of the pixels it stands for.
pub struct ChunkPaint {
    data: Vec<u8>,
    across: u32,
    grid: PaintGrid,
}

impl ChunkPaint {
    pub fn of(paint: &PaintMap, chunk: ChunkSamples) -> ChunkPaint {
        let stride = chunk.stride.max(1);
        let across = chunk.side.div_ceil(stride) + 2;
        let first = (
            chunk.first.0 as i64 - stride as i64,
            chunk.first.1 as i64 - stride as i64,
        );
        let middle = (stride / 2) as i64;
        let mut paint = paint.reader();
        let mut pixel = |column: u32, row: u32| {
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
        ChunkPaint { data, across, grid }
    }
}

/// `layers` chunks' paint, each `side` pixels square, a pixel per entry, each the terrain's place
/// in the area's list: the layers `held` holds first, and none painted after them.
fn paints_image(side: u32, layers: u32, mut held: Vec<u8>) -> Image {
    held.resize((side * side * layers) as usize, 0);
    let mut image = Image::new(
        Extent3d {
            width: side.max(1),
            height: side.max(1),
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        held,
        TextureFormat::R8Uint,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image
}

/// `layers` chunks' grids, a pixel each, the grids `held` holds first.
fn grids_image(layers: u32, mut held: Vec<u8>) -> Image {
    held.resize(layers as usize * GRID_BYTES, 0);
    Image::new(
        Extent3d {
            width: layers.max(1),
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        held,
        TextureFormat::Rgba32Float,
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
/// its edges so a coarser chunk beside it shows no gap where their heights differ. Its paint is
/// read by where it lies, so it carries no uv.
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
    let mut heights = heights.reader();
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
    }
    for (index, &vertex) in rim.iter().enumerate() {
        let next = rim[(index + 1) % rim.len()];
        let (low, low_next) = (hem + index as u32, hem + ((index + 1) % rim.len()) as u32);
        indices.extend([vertex, low, next, next, low, low_next]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices));
    mesh.compress_indices();
    mesh
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
