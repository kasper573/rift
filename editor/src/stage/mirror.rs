//! The stage's picture: an offscreen camera over the scene requested of it. Actors hang their
//! plates over where they stand; their rigs, the items and the vfx are drawn once the stage renders
//! the world in 3D.

use ui::egui;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::prelude::*;
use bevy_egui::EguiUserTextures;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::actor::ModelId;
use engine::systems::actor::plate::PlateLabel;

use super::{SIZE, ZOOM, images, to_screen};

const FIRST_LAYER: usize = 500;

/// An actor as the stage places it this frame: where it stands and the plate over its head.
pub struct ActorPose {
    pub model: ModelId,
    pub at: Pos<Meters>,
    pub plate: Option<PlateLabel>,
}

pub struct Frame<'a> {
    pub content: &'a Content,
    pub center: Pos<Meters>,
    pub actors: Vec<ActorPose>,
    /// What the scene asks for that the stage cannot draw yet.
    pub not_yet: Vec<&'static str>,
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
        let (image, texture) = images::render_target(world, SIZE);
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
        if !frame.not_yet.is_empty() {
            self.problems.push(format!(
                "{} play on the stage once it draws the world in 3D",
                frame.not_yet.join(" and ")
            ));
        }
        self.plates = frame
            .actors
            .iter()
            .filter_map(|pose| {
                let height = pose.model.get(frame.content).hitbox.height;
                let top = to_screen(pose.at) + Vec2::new(0.0, height.0 * super::PX_PER_METER);
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
