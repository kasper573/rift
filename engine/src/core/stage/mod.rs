//! The stage the interface shows its lit things on: a small scene in front of the window, under
//! one lamp, where each thing is drawn by a camera of its own into a picture the interface shows
//! where the thing belongs. Cards lie on it, and globes stand on it.

use std::f32::consts::PI;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Exposure, RenderTarget, ScalingMode, SubCameraView};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::ImageSampler;
use bevy::light::AmbientLight;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::transform::TransformSystems;
use bevy_egui::{EguiPostUpdateSet, EguiTextureHandle, EguiUserTextures};
use ui::egui::{self, Pos2, Rect};

use crate::core::card::CardLamp;
use crate::core::render::Look;
use crate::core::render::world::sky::{Daylight, SunLight, adaptation};
use crate::core::render::world::{DRAWN_FRAMES_AFTER, RenderProgress, lights};
use crate::core::units::Degrees;

pub mod card;
pub mod globe;

/// The light of the sky over the player, where one stands. Whatever shows the sky tells it, and
/// tells it anew only once the light has changed to the eye; a lamp that follows the sky is hung
/// by it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct StageSkyLight(pub Option<SkyLampLight>);

/// A sky's light for the stage: its sun, or by night its moon, as it shines under a clear sky,
/// and the bearing its sun stands at at noon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyLampLight {
    pub shining: SunLight,
    pub noon: Degrees,
}

pub struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StageSkyLight>()
            .init_resource::<StageLayers>()
            // What stands on the stage is set as the interface draws, and must be where the
            // interface shows it in the frame that shows it: a pass later, the picture of a
            // thing dragged would trail the place the interface shows it at.
            .configure_sets(
                PostUpdate,
                EguiPostUpdateSet::EndPass
                    .before(TransformSystems::Propagate)
                    .before(bevy::camera::CameraUpdateSystems),
            )
            .add_plugins((card::CardStagePlugin, globe::GlobeStagePlugin));
    }
}

/// Where the stage's eye and lamp stand over the window, in the points the interface is drawn
/// in, and how many of them the window is high.
#[derive(Clone, Copy, PartialEq)]
struct Scene {
    eye: f32,
    lamp: Lamp,
    height: f32,
}

/// The stage's lamp: where it hangs, its colour, how much of a thing's light is its at its
/// fullest, and how much of that it shines with now.
#[derive(Clone, Copy, PartialEq)]
struct Lamp {
    at: Vec3,
    color: Color,
    share: f32,
    shines: f32,
}

impl Lamp {
    /// The lamp as the game's settings hang it, or the light of the sky where the lamp follows
    /// one and one stands.
    fn over(world: &World, hung: &CardLamp, screen: Rect) -> Lamp {
        let at = lamp_at(hung, screen);
        let written = Lamp {
            at,
            color: hung.color.color(),
            share: hung.strength,
            shines: 1.0,
        };
        let sky = world
            .get_resource::<StageSkyLight>()
            .and_then(|light| light.0)
            .filter(|_| hung.follows_sky);
        let (Some(sky), Some(look)) = (sky, world.get_resource::<Look>()) else {
            return written;
        };
        let light = sky.shining;
        let daylight = world
            .get_resource::<Daylight>()
            .copied()
            .unwrap_or_default();
        let opened = adaptation(daylight, light, look);
        let shines = light.illuminance / look.light.sun_lux * 2f32.powf(opened.0);
        let toward = lights::toward(light.elevation, light.azimuth);
        let ahead = lights::toward(Degrees(0.0), sky.noon);
        let aside = lights::toward(Degrees(0.0), Degrees(sky.noon.0 + 90.0));
        let away = at.length();
        Lamp {
            at: Vec3::new(toward.dot(aside), toward.y, toward.dot(ahead)) * away,
            color: light.color,
            shines: shines.clamp(0.0, 1.0),
            ..written
        }
    }

    /// The share of a thing's light that falls on it evenly, whatever way it faces.
    fn even(&self) -> f32 {
        1.0 - self.share
    }
}

/// How a picture of the stage is drawn: through which lens, how its edges are smoothed, and how
/// much light falls on what it shows from everywhere at once.
#[derive(Clone, Copy)]
struct PictureLook {
    lens: Lens,
    smoothing: Msaa,
    even: f32,
}

/// How a picture's camera looks at the stage.
#[derive(Clone, Copy)]
enum Lens {
    /// From the stage's eye over the window's middle, so a thing away from the middle is seen a
    /// little from its side, as it would be on a table.
    FromTheEye,
    /// Straight on from right in front of what it shows, with no perspective: what lies level
    /// there is seen exactly edge-on.
    StraightOn,
}

/// The picture the stage takes of one thing on it: the image the interface shows, the camera
/// that draws it and the lamp that lights it, on a render layer of the thing's own.
#[derive(Default)]
struct Picture {
    layer: Option<usize>,
    side: u32,
    image: Option<Handle<Image>>,
    texture: Option<egui::TextureId>,
    camera: Option<Entity>,
    lamp: Option<Entity>,
}

impl Picture {
    /// The render layer what the picture shows is drawn on, taken when first asked for.
    fn layer(&mut self, world: &mut World) -> usize {
        *self
            .layer
            .get_or_insert_with(|| world.resource_mut::<StageLayers>().take())
    }

    fn texture(&self) -> Option<egui::TextureId> {
        self.texture
    }

    /// Points the camera at the part of the window `view` covers, through the look's lens, into
    /// a picture of that size, with the lamp where the scene hangs it.
    fn frame(&mut self, world: &mut World, view: &SubCameraView, scene: &Scene, look: PictureLook) {
        let layer = self.layer(world);
        let lamp = scene.lamp;
        let side = view.size.x;
        if self.side != side || self.image.is_none() {
            if let Some(old) = self.image.take() {
                world.resource_mut::<EguiUserTextures>().remove_image(&old);
            }
            let mut image =
                Image::new_target_texture(side, side, TextureFormat::Rgba8UnormSrgb, None);
            image.sampler = ImageSampler::nearest();
            let image = world.resource_mut::<Assets<Image>>().add(image);
            self.texture = Some(
                world
                    .resource_mut::<EguiUserTextures>()
                    .add_image(EguiTextureHandle::Strong(image.clone())),
            );
            self.image = Some(image);
            self.side = side;
        }
        let Some(image) = self.image.clone() else {
            return;
        };
        let (near, far) = (scene.eye * DEPTH.0, scene.eye * DEPTH.1);
        let (lens, stands, part) = match look.lens {
            Lens::FromTheEye => (
                Projection::Perspective(PerspectiveProjection {
                    fov: 2.0 * (0.5 * scene.height / scene.eye).atan(),
                    near,
                    far,
                    ..default()
                }),
                Transform::from_xyz(0.0, 0.0, scene.eye),
                Some(*view),
            ),
            Lens::StraightOn => {
                let pixels = view.full_size.y as f32 / scene.height;
                let middle = (view.offset + view.size.as_vec2() / 2.0
                    - view.full_size.as_vec2() / 2.0)
                    / pixels;
                let span = view.size.as_vec2() / pixels;
                (
                    Projection::Orthographic(OrthographicProjection {
                        near,
                        far,
                        scaling_mode: ScalingMode::Fixed {
                            width: span.x,
                            height: span.y,
                        },
                        ..OrthographicProjection::default_3d()
                    }),
                    Transform::from_xyz(middle.x, -middle.y, scene.eye),
                    None,
                )
            }
        };
        let camera = *self.camera.get_or_insert_with(|| {
            world
                .spawn((
                    Camera3d::default(),
                    Camera {
                        order: -1,
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        ..default()
                    },
                    Tonemapping::None,
                    DebandDither::Disabled,
                    look.smoothing,
                    Exposure {
                        ev100: PLAIN_EXPOSURE,
                    },
                    RenderLayers::layer(layer),
                ))
                .id()
        });
        let mut camera = world.entity_mut(camera);
        camera.insert((
            RenderTarget::Image(image.into()),
            lens,
            stands,
            GlobalTransform::from(stands),
            AmbientLight {
                brightness: look.even,
                ..default()
            },
        ));
        if let Some(mut drawn) = camera.get_mut::<Camera>()
            && drawn.sub_camera_view != part
        {
            drawn.sub_camera_view = part;
        }
        let hangs = Transform::from_translation(lamp.at);
        let light = PointLight {
            color: lamp.color,
            // What lights a matte face squarely under the lamp to the lamp's share of its colour.
            intensity: lamp.share * lamp.shines * 4.0 * PI * PI * lamp.at.z * lamp.at.z,
            range: LAMP_RANGE * scene.height,
            radius: LAMP_WIDTH * scene.height,
            shadow_maps_enabled: false,
            ..default()
        };
        match self.lamp {
            Some(lit) => {
                world
                    .entity_mut(lit)
                    .insert((light, hangs, GlobalTransform::from(hangs)));
            }
            None => {
                self.lamp = Some(
                    world
                        .spawn((
                            light,
                            hangs,
                            GlobalTransform::from(hangs),
                            RenderLayers::layer(layer),
                        ))
                        .id(),
                );
            }
        }
    }

    /// Has the picture taken anew this pass, or kept as it is.
    fn take(&self, world: &mut World, anew: bool) {
        if let Some(mut camera) = self
            .camera
            .and_then(|camera| world.get_mut::<Camera>(camera))
            && camera.is_active != anew
        {
            camera.is_active = anew;
        }
    }

    /// Takes the camera and the lamp off the stage. The picture stays as it was last taken.
    fn strike(&mut self, world: &mut World) {
        for gone in [self.camera.take(), self.lamp.take()].into_iter().flatten() {
            world.despawn(gone);
        }
    }

    /// Lets go of the picture and of its render layer: nothing shows it any more.
    fn forget(&mut self, world: &mut World) {
        self.strike(world);
        if let Some(image) = self.image.take() {
            world
                .resource_mut::<EguiUserTextures>()
                .remove_image(&image);
        }
        self.texture = None;
        if let Some(layer) = self.layer.take() {
            world.resource_mut::<StageLayers>().give_back(layer);
        }
    }
}

/// The render layers the stage's pictures are taken on, one each.
#[derive(Resource, Default)]
struct StageLayers(Vec<usize>);

impl StageLayers {
    fn take(&mut self) -> usize {
        let layer = (FIRST_LAYER..)
            .find(|layer| !self.0.contains(layer))
            .unwrap_or(FIRST_LAYER);
        self.0.push(layer);
        layer
    }

    fn give_back(&mut self, layer: usize) {
        self.0.retain(|taken| *taken != layer);
    }
}

/// Whether the renderer has had the frames a change takes to reach the screen since it was
/// first asked of `since`, and is compiling no pipeline.
fn rendered(world: &World, since: &mut Option<u64>) -> bool {
    let Some(progress) = world.get_resource::<RenderProgress>() else {
        return false;
    };
    let from = *since.get_or_insert(progress.drawn());
    progress.compiling() == 0 && progress.drawn() >= from + DRAWN_FRAMES_AFTER
}

/// The part of the window a square of `side` pixels about `center` covers, and that square's top
/// left corner in pixels.
fn view_about(center: Pos2, side: u32, pixels: f32, window: UVec2) -> (SubCameraView, egui::Vec2) {
    let corner = (center.to_vec2() * pixels - egui::Vec2::splat(side as f32 / 2.0)).round();
    let view = SubCameraView {
        full_size: window,
        offset: Vec2::new(corner.x, corner.y),
        size: UVec2::splat(side),
    };
    (view, corner)
}

/// A point of the interface on the stage: the window's middle at the origin, up being up.
fn on_stage(at: Pos2, screen: Rect) -> Vec3 {
    let middle = screen.center();
    Vec3::new(at.x - middle.x, middle.y - at.y, 0.0)
}

fn lamp_at(lamp: &CardLamp, screen: Rect) -> Vec3 {
    let at = screen.min + screen.size() * egui::vec2(lamp.across, lamp.down);
    on_stage(at, screen) + Vec3::Z * lamp.height * screen.height()
}

/// The exposure under which Bevy's lighting comes out as it went in.
const PLAIN_EXPOSURE: f32 = -0.263_034_4;
/// The first render layer the stage's pictures are taken on.
const FIRST_LAYER: usize = 8;
/// How much nearer and farther than the window's plane the stage's cameras draw, as shares of
/// their distance from it.
const DEPTH: (f32, f32) = (0.1, 10.0);
/// How many passes a picture is taken anew once what it shows has changed: the pass of the
/// change, and those it takes to reach the screen.
const DUE_AFTER_A_CHANGE: u8 = DRAWN_FRAMES_AFTER as u8 + 1;
/// How wide the lamp is, in window heights: a point would leave a polished surface a highlight
/// a pixel wide.
const LAMP_WIDTH: f32 = 0.04;
/// How far the lamp's light carries, in window heights: past everything on the stage.
const LAMP_RANGE: f32 = 20.0;
