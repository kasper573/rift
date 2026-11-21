//! A field every shader of a kind reads beside its mesh: a square picture of two values a texel,
//! read smoothly between its texels, which the system that knows the values fills whole whenever
//! they change. It is written over in place on the GPU, so the materials that read it stand
//! unchanged, none of their meshes drawn up again for it.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
    TextureDimension, TextureFormat, TextureId,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use half::f16;

use crate::core::render::sample::UploadedBytes;

/// Adds the field, `side` texels each way and first holding nothing, and writes it to the GPU
/// whenever it is published anew through [`ShaderFieldWrite`].
pub struct ShaderFieldPlugin {
    pub side: usize,
}

/// The picture of the field.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ShaderFieldImage {
    pub image: Handle<Image>,
    pub side: usize,
}

/// What the field holds now, row by row, as it was last published.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ShaderFieldWrite(Option<Arc<[[f32; 2]]>>);

impl ShaderFieldWrite {
    /// A field holding `first` in the red of its texels and `second` in their green.
    pub fn holding(first: &[f32], second: &[f32]) -> ShaderFieldWrite {
        ShaderFieldWrite(Some(
            first.iter().zip(second).map(|(a, b)| [*a, *b]).collect(),
        ))
    }
}

impl Plugin for ShaderFieldPlugin {
    fn build(&self, app: &mut App) {
        let side = self.side as u32;
        let mut image = Image::new(
            Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0; self.side * self.side * TEXEL_BYTES],
            TextureFormat::Rg16Float,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::linear();
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(ShaderFieldImage {
            image,
            side: self.side,
        })
        .init_resource::<ShaderFieldWrite>()
        .add_plugins((
            ExtractResourcePlugin::<ShaderFieldImage>::default(),
            ExtractResourcePlugin::<ShaderFieldWrite>::default(),
        ));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<UploadedBytes>()
                .add_systems(Render, write_field.in_set(RenderSystems::PrepareResources));
        }
    }
}

/// Writes the field into its picture on the GPU when it was published anew, or when the GPU
/// holds the picture anew.
fn write_field(
    write: Res<ShaderFieldWrite>,
    field: Option<Res<ShaderFieldImage>>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
    mut uploaded: ResMut<UploadedBytes>,
    mut written: Local<Option<TextureId>>,
) {
    let (Some(field), Some(values)) = (field, &write.0) else {
        return;
    };
    let Some(gpu) = images.get(&field.image) else {
        return;
    };
    let fresh = *written != Some(gpu.texture.id());
    if !fresh && !write.is_changed() {
        return;
    }
    *written = Some(gpu.texture.id());
    let side = field.side as u32;
    let data: Vec<u8> = values
        .iter()
        .flatten()
        .flat_map(|value| f16::from_f32(*value).to_le_bytes())
        .collect();
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &gpu.texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &data,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * TEXEL_BYTES as u32),
            rows_per_image: None,
        },
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    uploaded.0 += data.len() as u64;
}

const TEXEL_BYTES: usize = 2 * size_of::<f16>();
