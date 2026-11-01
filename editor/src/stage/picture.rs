//! The stage's picture: an offscreen camera over the scene requested of it. Actors hang their
//! plates over where they stand; their rigs, the items and the vfx are drawn once the stage renders
//! the world in 3D.

use ui::egui;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::actor::plate::PlateLabel;

use super::{PosedActor, SIZE, ZOOM, to_screen};
use crate::views::navigation::PX_PER_METER;

const FIRST_LAYER: usize = 500;

pub struct Frame<'a> {
    pub content: &'a Content,
    pub center: Pos<Meters>,
    pub actors: Vec<PosedActor>,
    /// Whether the scene has vfx, which the stage cannot draw yet.
    pub has_vfx: bool,
}

pub struct Stage {
    pub texture: egui::TextureId,
    /// Plates to hang over the actors, by the window point each hangs from.
    pub plates: Vec<(Vec2, PlateLabel)>,
    pub problems: Vec<String>,
    pub index: usize,
    camera: Entity,
    image: Handle<Image>,
}

impl Stage {
    pub fn spawn(world: &mut World, index: usize) -> Stage {
        let (image, texture) = render_target(world, SIZE, ImageSampler::nearest());
        let layer = RenderLayers::none().with(FIRST_LAYER + index);
        let camera = world
            .spawn((
                Camera2d,
                RenderTarget::Image(image.clone().into()),
                Camera {
                    order: -50 - index as isize,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                Projection::Orthographic(OrthographicProjection {
                    scale: 1.0 / ZOOM,
                    ..OrthographicProjection::default_2d()
                }),
                Transform::default(),
                layer,
            ))
            .id();
        Stage {
            texture,
            plates: Vec::new(),
            problems: Vec::new(),
            index,
            camera,
            image,
        }
    }

    /// Brings the picture up to the frame: names kept for the window to letter over where each
    /// actor stands, and a word on what the stage cannot draw yet.
    pub fn render(&mut self, world: &mut World, frame: &Frame) {
        let eye = to_screen(frame.center);
        if let Some(mut transform) = world.get_mut::<Transform>(self.camera) {
            transform.translation.x = eye.x;
            transform.translation.y = eye.y;
        }
        self.problems.clear();
        if frame.has_vfx {
            self.problems
                .push("vfx play on the stage once it draws the world in 3D".to_owned());
        }
        self.plates = frame
            .actors
            .iter()
            .filter_map(|pose| {
                let height = pose.model.get(frame.content).hitbox.height;
                let top = to_screen(pose.at) + Vec2::new(0.0, height.0 * PX_PER_METER);
                pose.plate.clone().map(|label| (top, label))
            })
            .collect();
    }

    pub fn despawn(self, world: &mut World) {
        world.despawn(self.camera);
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&self.image);
    }
}

/// An offscreen picture of `size` pixels a camera renders to, and the egui texture that shows it.
pub(crate) fn render_target(
    world: &mut World,
    size: [u32; 2],
    sampler: ImageSampler,
) -> (Handle<Image>, egui::TextureId) {
    let mut image = Image::new_target_texture(
        size[0].max(1),
        size[1].max(1),
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.sampler = sampler;
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    (image, texture)
}
