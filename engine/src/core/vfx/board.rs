//! The board every shader of the world reads beside its mesh: a picture a texel per value, which
//! the systems that know the values fill as the frame goes. Its changed texels are written over in
//! place on the GPU, so a frame in which nothing changed writes nothing, and the materials that
//! read it stand unchanged, none of their meshes drawn up again for it.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
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

use crate::core::render::sample::UploadedBytes;

/// How many texels a row of the board holds: it is read in order, row by row.
pub const BOARD_WIDTH: usize = 256;

/// Adds the board, first holding `initial`, and writes what changes of it to the GPU. Whoever
/// fills it keeps what it last published, and publishes the next through [`ShaderBoardWrites`].
pub struct ShaderBoardPlugin {
    pub initial: Vec<Vec4>,
}

/// The picture of the board, and what it held when it was made.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ShaderBoardImage {
    pub image: Handle<Image>,
    pub initial: Arc<[Vec4]>,
}

/// The texels of the board that changed since it was last written, run by run: where each run
/// starts and what it holds now.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ShaderBoardWrites(Vec<(usize, Vec<Vec4>)>);

impl ShaderBoardWrites {
    /// The writes that bring a board holding `held` to hold `texels`, or none when it holds them
    /// already.
    pub fn between(held: &[Vec4], texels: &[Vec4]) -> Option<ShaderBoardWrites> {
        let runs = changed_runs(held, texels);
        (!runs.is_empty()).then_some(ShaderBoardWrites(runs))
    }
}

/// The texels of a board as the picture holds them.
pub fn texel_bytes(texels: &[Vec4]) -> Vec<u8> {
    texels
        .iter()
        .flat_map(Vec4::to_array)
        .flat_map(f32::to_le_bytes)
        .collect()
}

impl Plugin for ShaderBoardPlugin {
    fn build(&self, app: &mut App) {
        let image = Image::new(
            Extent3d {
                width: BOARD_WIDTH as u32,
                height: self.initial.len().div_ceil(BOARD_WIDTH) as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            texel_bytes(&self.initial),
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        );
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(ShaderBoardImage {
            image,
            initial: self.initial.clone().into(),
        })
        .init_resource::<ShaderBoardWrites>()
        .add_plugins((
            ExtractResourcePlugin::<ShaderBoardImage>::default(),
            ExtractResourcePlugin::<ShaderBoardWrites>::default(),
        ));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<UploadedBytes>()
                .add_systems(Render, write_board.in_set(RenderSystems::PrepareResources));
        }
    }
}

/// Writes what changed of the board into its picture on the GPU, or all of it into a picture the
/// GPU holds anew.
fn write_board(
    writes: Res<ShaderBoardWrites>,
    board: Option<Res<ShaderBoardImage>>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
    mut uploaded: ResMut<UploadedBytes>,
    mut held: Local<Option<Vec<Vec4>>>,
    mut written: Local<Option<TextureId>>,
) {
    let Some(board) = board else {
        return;
    };
    let held = held.get_or_insert_with(|| board.initial.to_vec());
    if writes.is_changed() {
        for (start, run) in &writes.0 {
            held[*start..*start + run.len()].copy_from_slice(run);
        }
    }
    let Some(gpu) = images.get(&board.image) else {
        return;
    };
    if *written != Some(gpu.texture.id()) {
        *written = Some(gpu.texture.id());
        uploaded.0 += write_texels(&queue, gpu, 0, held);
    } else if writes.is_changed() {
        for (start, run) in &writes.0 {
            uploaded.0 += write_texels(&queue, gpu, *start, run);
        }
    }
}

/// Writes `texels` into the picture from the texel `start` on, a row at a time, returning the
/// bytes written.
fn write_texels(queue: &RenderQueue, gpu: &GpuImage, start: usize, texels: &[Vec4]) -> u64 {
    let mut at = start;
    let mut rest = texels;
    let mut bytes = 0;
    while !rest.is_empty() {
        let (row, column) = (at / BOARD_WIDTH, at % BOARD_WIDTH);
        let (line, after) = rest.split_at(rest.len().min(BOARD_WIDTH - column));
        let data = texel_bytes(line);
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: column as u32,
                    y: row as u32,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(data.len() as u32),
                rows_per_image: None,
            },
            Extent3d {
                width: line.len() as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        bytes += data.len() as u64;
        at += line.len();
        rest = after;
    }
    bytes
}

/// The runs of texels `now` holds that differ from `was`, joined across short stretches alike.
fn changed_runs(was: &[Vec4], now: &[Vec4]) -> Vec<(usize, Vec<Vec4>)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (index, (before, after)) in was.iter().zip(now).enumerate() {
        if before == after {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if index - *end <= JOINED_ACROSS => *end = index + 1,
            _ => runs.push((index, index + 1)),
        }
    }
    runs.into_iter()
        .map(|(start, end)| (start, now[start..end].to_vec()))
        .collect()
}

/// How many unchanged texels a run of changed ones is carried across rather than broken at: a
/// write costs more than the few texels it would leave out.
const JOINED_ACROSS: usize = 4;
