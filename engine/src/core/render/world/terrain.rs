//! The ground as one mesh: a vertex at every height sample, painted by the ground material with
//! the terrains laid there, each its colour under the picture it repeats over the ground, blended
//! across the paint's pixels as the paint blends; and its heights as a picture, for what a shader
//! settles on the ground.

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

use super::material::{dielectric, embedded};
use crate::core::ground::{Heightfield, Paint, SAMPLES_PER_METER};
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
/// a terrain that has none; the paint, a terrain's place in the area's list for each pixel; and
/// for each terrain its colour and tile in the first row and its picture's place in the stack in
/// the second; and where the paint's pixels lie.
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
    pub ground: GroundUniform,
}

impl MaterialExtension for GroundLook {
    fn fragment_shader() -> ShaderRef {
        embedded("ground.wesl")
    }
}

/// A ground whose pictures are still loading: its layers, and each picture in their order.
#[derive(Component)]
pub struct GroundPictures {
    layers: Vec<GroundLayer>,
    loading: Vec<Handle<Image>>,
}

/// The material `paint` is drawn with in `layers`, the pictures white until they load, and what
/// waits for them to.
pub fn ground_material(
    layers: &[GroundLayer],
    paint: &Paint,
    assets: &AssetServer,
    images: &mut Assets<Image>,
) -> (GroundMaterial, GroundPictures) {
    let loading = layers
        .iter()
        .filter_map(|layer| Some(assets.load(layer.picture.as_ref()?.0.clone())))
        .collect();
    let material = ExtendedMaterial {
        base: dielectric(StandardMaterial::default()),
        extension: GroundLook {
            pictures: images.add(stack(1, &[])),
            paint: images.add(paint_image(paint)),
            layers: images.add(layers_image(layers, &vec![0; layers.len()])),
            ground: GroundUniform::lifted(0.0),
        },
    };
    let waiting = GroundPictures {
        layers: layers.to_vec(),
        loading,
    };
    (material, waiting)
}

/// Lays `paint` on the ground `material` draws.
pub fn repaint(
    material: &Handle<GroundMaterial>,
    paint: &Paint,
    materials: &mut Assets<GroundMaterial>,
    images: &mut Assets<Image>,
) {
    if let Some(mut material) = materials.get_mut(material) {
        material.extension.paint = images.add(paint_image(paint));
    }
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

/// The paint as a picture a pixel per paint pixel, each the terrain's place in the area's list.
fn paint_image(paint: &Paint) -> Image {
    let (columns, rows) = paint.pixels();
    let data = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| paint.index(column, row)))
        .collect();
    Image::new(
        Extent3d {
            width: columns.max(1),
            height: rows.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
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
    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: pictures.len() as u32 + 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
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

/// Where the ground's samples lie, as a shader settling a mesh on the ground reads them: the
/// metres the first stands at, how many stand to the metre, and how far above the ground to draw.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct GroundUniform {
    pub origin: Vec2,
    pub samples_per_metre: f32,
    pub lift: f32,
}

impl GroundUniform {
    pub fn lifted(lift: f32) -> GroundUniform {
        GroundUniform {
            origin: Vec2::new(GROUND_ORIGIN.x, GROUND_ORIGIN.y),
            samples_per_metre: SAMPLES_PER_METER as f32,
            lift,
        }
    }
}

/// The ground's heights as a picture a texel per sample, in metres.
pub fn height_image(heights: &Heightfield) -> Image {
    let (columns, rows) = heights.samples();
    let data = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| heights.sample(column, row).0))
        .flat_map(f32::to_le_bytes)
        .collect();
    Image::new(
        Extent3d {
            width: columns.max(1),
            height: rows.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// The terrain's mesh, in the world's metres from the ground's corner.
pub fn mesh(heights: &Heightfield) -> Mesh {
    let (columns, rows) = heights.samples();
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let mut positions = Vec::with_capacity((columns * rows) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for row in 0..rows {
        for column in 0..columns {
            let x = GROUND_ORIGIN.x + column as f32 * step;
            let z = GROUND_ORIGIN.y + row as f32 * step;
            positions.push([x, heights.sample(column, row).0, z]);
            let east =
                heights.sample(column + 1, row).0 - heights.sample(column.saturating_sub(1), row).0;
            let south =
                heights.sample(column, row + 1).0 - heights.sample(column, row.saturating_sub(1)).0;
            normals.push(Vec3::new(-east, 2.0 * step, -south).normalize().to_array());
            uvs.push([
                column as f32 / (columns - 1).max(1) as f32,
                row as f32 / (rows - 1).max(1) as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity(((columns - 1) * (rows - 1) * 6) as usize);
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let corner = row * columns + column;
            let (east, south, far) = (corner + 1, corner + columns, corner + columns + 1);
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
