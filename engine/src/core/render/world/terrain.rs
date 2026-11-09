//! How the ground is painted: every terrain its colour under the picture it repeats over the
//! ground, the pictures stacked as they load; and the meshes laid over the ground for what is drawn
//! on it.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDataOrder, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use image::RgbaImage;
use image::imageops::{self, FilterType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::gpu::draw::{GroundDraw, GroundPaint};
use super::material::SurfaceFinish;
use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::units::Meters;

/// The side of every picture laid over the ground, in pixels: all one size, so the ground draws
/// them from one stack.
pub const GROUND_PICTURE: u32 = 512;
/// How steeply a picture may be seen and stay sharp, in times the texels a pixel reads along the
/// ground.
const SHARPNESS: u16 = 16;

/// How one terrain paints the ground: its colour, the picture laid over it, if any, repeating
/// every `tile` metres, its shades multiplied into the colour, and how it looks soaked, if rain
/// wets it.
#[derive(Clone, Debug, PartialEq)]
pub struct GroundLayer {
    pub color: LinearRgba,
    pub picture: Option<(String, Meters)>,
    pub wet: Option<WetLook>,
}

/// How a surface looks soaked through: `darkening` of its colour left, 0 to 1, as wet earth
/// darkens, and as rough as `roughness`, 0 a mirror to 1 matte, as water films it; between its
/// own look and this as it lies wet.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WetLook {
    pub darkening: f32,
    pub roughness: f32,
}

/// A ground whose pictures are still loading, or being stacked once they have: its layers, and
/// each picture in their order.
#[derive(Component)]
pub struct GroundPictures {
    layers: Vec<GroundLayer>,
    loading: Vec<Handle<Image>>,
    stacking: Option<Task<(Image, Image)>>,
}

/// What the ground is painted with in `layers`, the pictures white until they load, and what waits
/// for them to.
pub fn ground_paint(
    layers: &[GroundLayer],
    finish: &SurfaceFinish,
    weather: Handle<Image>,
    assets: &AssetServer,
    images: &mut Assets<Image>,
) -> (GroundPaint, GroundPictures) {
    let loading = layers
        .iter()
        .filter_map(|layer| Some(assets.load(layer.picture.as_ref()?.0.clone())))
        .collect();
    let paint = GroundPaint {
        pictures: images.add(stack(1, &[])),
        layers: images.add(layers_image(layers, &vec![0; layers.len()])),
        weather,
        roughness: finish.roughness,
        reflectance: finish.reflectance,
    };
    let waiting = GroundPictures {
        layers: layers.to_vec(),
        loading,
        stacking: None,
    };
    (paint, waiting)
}

/// Stacks every waiting ground's pictures once they have all loaded, or failed, which leaves
/// that terrain unpictured: on a task, so the frames keep coming while each is scaled down level
/// by level.
pub(super) fn stack_pictures(
    mut waiting: Query<(Entity, &mut GroundPictures, &mut GroundDraw)>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    for (entity, mut ground, mut draw) in &mut waiting {
        if let Some(task) = &mut ground.stacking {
            let Some((pictures, layers)) = check_ready(task) else {
                continue;
            };
            commands.entity(entity).remove::<GroundPictures>();
            draw.paint.pictures = images.add(pictures);
            draw.paint.layers = images.add(layers);
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

/// Each layer's colour and tile in the first row, and the place of its picture in the stack in
/// the second.
fn layers_image(layers: &[GroundLayer], places: &[u32]) -> Image {
    let looks = layers.iter().map(|layer| {
        let tile = layer.picture.as_ref().map_or(1.0, |(_, tile)| tile.0);
        [layer.color.red, layer.color.green, layer.color.blue, tile]
    });
    let places = places.iter().map(|&place| [place as f32, 0.0, 0.0, 0.0]);
    let soaked = layers.iter().map(|layer| match layer.wet {
        Some(wet) => [wet.darkening, wet.roughness, 1.0, 0.0],
        None => [1.0, 1.0, 0.0, 0.0],
    });
    let data = looks
        .chain(places)
        .chain(soaked)
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    Image::new(
        Extent3d {
            width: layers.len().max(1) as u32,
            height: 3,
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
    let indices: Vec<u32> = grid_indices(across, down).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The triangles of a grid of vertices `across` × `down`, laid row by row: two a quad, each wound
/// to face up.
pub(super) fn grid_indices(across: u32, down: u32) -> impl Iterator<Item = u32> {
    (0..down.saturating_sub(1)).flat_map(move |row| {
        (0..across.saturating_sub(1)).flat_map(move |column| {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            [corner, south, east, east, south, far]
        })
    })
}
