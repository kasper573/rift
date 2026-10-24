use std::collections::HashMap;
use ui::egui;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};

use crate::project::ProjectFiles;

/// The project's images as bevy holds them for the stage, each kept until its file changes.
#[derive(Default)]
pub struct Images(HashMap<String, (String, Handle<Image>)>);

impl Images {
    /// The image at `path` among the project's files, decoded once per version of the file.
    pub fn handle(&mut self, files: &ProjectFiles, path: &str, world: &mut World) -> Handle<Image> {
        let (Some(bytes), Some(hash)) = (files.get(path), files.hash_of(path)) else {
            return Handle::default();
        };
        if let Some((known, handle)) = self.0.get(path)
            && known == hash
        {
            return handle.clone();
        }
        let handle = match image::load_from_memory(bytes) {
            Ok(decoded) => {
                let rgba = decoded.into_rgba8();
                let mut image = Image::new(
                    Extent3d {
                        width: rgba.width(),
                        height: rgba.height(),
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    rgba.into_raw(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                image.sampler = ImageSampler::nearest();
                world.resource_mut::<Assets<Image>>().add(image)
            }
            Err(_) => Handle::default(),
        };
        self.0
            .insert(path.to_owned(), (hash.to_owned(), handle.clone()));
        handle
    }
}

/// An offscreen picture of `alloc` pixels a camera renders to, and the egui texture that shows it.
pub fn render_target(world: &mut World, alloc: [u32; 2]) -> (Handle<Image>, egui::TextureId) {
    let mut image =
        Image::new_target_texture(alloc[0], alloc[1], TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::nearest();
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    (image, texture)
}
