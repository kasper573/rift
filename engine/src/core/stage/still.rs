//! Stills: a picture of something standing alone in a daylight of its own, taken once by the
//! world's renderer and kept for the interface to show wherever it has a use for it.

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::ImageSampler;
use bevy::light::AmbientLight;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use ui::egui;

use super::StageLayers;
use crate::core::render::Look;
use crate::core::render::camera::{self as world_camera};
use crate::core::render::world::sky::{self, Sky};
use crate::core::render::world::{RenderProgress, lights};
use crate::core::units::Degrees;

/// What a still shows of what stands under it: the square `side` across round `center`, seen
/// from the front.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StillFrame {
    pub center: Vec3,
    pub side: f32,
}

/// A still being taken: its camera and its light, with what it shows standing under it.
#[derive(Component)]
pub struct StillTaking {
    layer: usize,
    image: Handle<Image>,
    texture: egui::TextureId,
    /// How many frames the renderer had drawn when what the still shows first stood whole.
    whole_since: Option<u64>,
}

/// Sets a still of `frame` up where the world begins, on a layer of its own, and tells what its
/// subject is to stand under; nothing before the game's look is known.
pub fn stand(world: &mut World, frame: StillFrame) -> Option<Entity> {
    let look = *world.get_resource::<Look>()?;
    let layer = world.resource_mut::<StageLayers>().take();
    let layers = RenderLayers::layer(layer);
    let mut image = Image::new_target_texture(SIDE, SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    let still = world
        .spawn((
            Transform::default(),
            Visibility::default(),
            Propagate(layers.clone()),
            StillTaking {
                layer,
                image: image.clone(),
                texture,
                whole_since: None,
            },
        ))
        .id();
    world.spawn((
        lights::sun(Color::WHITE, look.light.sun_lux, SUN_ELEVATION, SUN_AZIMUTH),
        layers.clone(),
        ChildOf(still),
    ));
    let atmosphere = sky::atmosphere(
        &mut world.resource_mut::<Assets<ScatteringMedium>>(),
        Sky::Air,
        DRY_AIR,
        &look.sky,
    );
    world.spawn((atmosphere, layers.clone(), ChildOf(still)));
    let away = frame.side / 2.0 / (LENS.radians().0 / 2.0).tan();
    world.spawn((
        Camera3d::default(),
        Camera {
            order: ORDER,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image.into()),
        world_camera::lens(LENS, SIDE, SIDE),
        world_camera::picture(look.picture),
        world_camera::CameraFocus(frame.center),
        AmbientLight {
            color: Color::WHITE,
            brightness: look.light.ambient_nits,
            ..default()
        },
        layers,
        Transform::from_translation(frame.center + Vec3::Z * away)
            .looking_at(frame.center, Vec3::Y),
        ChildOf(still),
    ));
    Some(still)
}

/// The picture of the still `still`, for the interface to draw from then on, once the renderer
/// has drawn what it shows, which `whole` says all stands there: taking it clears the still and
/// everything under it away.
pub fn take(world: &mut World, still: Entity, whole: bool) -> Option<egui::TextureId> {
    let progress = world.get_resource::<RenderProgress>()?;
    let (drawn, compiling) = (progress.drawn(), progress.compiling());
    let mut taking = world.get_mut::<StillTaking>(still)?;
    if !whole {
        taking.whole_since = None;
        return None;
    }
    let since = *taking.whole_since.get_or_insert(drawn);
    if compiling > 0 || drawn < since + SETTLED_AFTER {
        return None;
    }
    let texture = taking.texture;
    clear(world, still);
    Some(texture)
}

/// Gives up on the still `still`: clears it away, its picture with it.
pub fn strike(world: &mut World, still: Entity) {
    if let Some(image) = clear(world, still) {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&image);
    }
}

/// Clears the still `still` and everything under it away, and tells the image it drew into.
fn clear(world: &mut World, still: Entity) -> Option<Handle<Image>> {
    let mut standing = world.get_entity_mut(still).ok()?;
    let taking = standing.take::<StillTaking>()?;
    standing.despawn();
    world.resource_mut::<StageLayers>().give_back(taking.layer);
    Some(taking.image)
}

/// How many pixels a still is across: enough for the largest the interface shows one.
const SIDE: u32 = 96;
/// A lens narrow enough to show what it frames nearly without perspective, as a portrait is
/// drawn.
const LENS: Degrees = Degrees(12.0);
/// A still's light falls from above and a little to one side of its eye, so what it shows has a
/// lit side and a shaded one.
const SUN_ELEVATION: Degrees = Degrees(40.0);
const SUN_AZIMUTH: Degrees = Degrees(150.0);
/// How much a still's air scatters the light, of dry air's dust: as dry air does.
const DRY_AIR: f32 = 1.0;
/// How many frames a still is drawn for once what it shows stands whole, for the picture's
/// smoothing over frames to settle.
const SETTLED_AFTER: u64 = 8;
/// A still is drawn before every camera that draws to the window.
const ORDER: isize = -100;
