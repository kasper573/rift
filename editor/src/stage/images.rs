use ui::egui;

use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};

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
