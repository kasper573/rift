//! A rendered thumbnail of every prop, for the palette, the prop picker and the row previews: the
//! model alone under a fixed day light from a fixed camera, drawn once by the game's renderer, read
//! back and kept as a texture until the model changes. The editor's alone: the game never draws
//! one.

use std::collections::{HashMap, VecDeque};

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::light::AmbientLight;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;
use bevy::render::gpu_readback::ReadbackOnce;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::content::media::GltfInfos;
use engine::core::geometry::Degrees;
use engine::core::render::camera::{self as world_camera, FOV};
use engine::core::render::world::lights::{self, AMBIENT_NITS, SUN_LUX};
use engine::core::render::world::sky::{self, Sky};
use engine::systems::area::prop::PropDef;
use ui::egui;

use crate::app::Editor;

pub const SIZE: u32 = 128;
const LAYER: usize = 399;
const YAW: Degrees = Degrees(30.0);
const PITCH: Degrees = Degrees(35.0);
const SUN_ELEVATION: Degrees = Degrees(50.0);
const SUN_AZIMUTH: Degrees = Degrees(300.0);
/// How many frames a model gets to load and settle before it is read back.
const SETTLE_FRAMES: u32 = 3;
/// How many frames a model that never loads, or never draws, is waited on.
const GIVE_UP_FRAMES: u32 = 600;

/// The thumbnails rendered so far, by prop path, and the queue of those still to draw.
#[derive(Default)]
pub struct PropThumbnails {
    ready: HashMap<String, Ready>,
    queue: VecDeque<String>,
    drawing: Option<Drawing>,
}

struct Ready {
    model_hash: String,
    texture: egui::TextureId,
    image: Handle<Image>,
}

struct Drawing {
    prop: String,
    model_hash: String,
    root: Entity,
    camera: Entity,
    target: Handle<Image>,
    frames: u32,
    readback: Option<Entity>,
}

impl PropThumbnails {
    /// The thumbnail of the prop at `prop`, queued for drawing when it has none yet.
    pub fn thumbnail(&mut self, prop: &str) -> Option<egui::TextureId> {
        match self.ready.get(prop) {
            Some(ready) => Some(ready.texture),
            None => {
                if !self.queue.iter().any(|queued| queued == prop)
                    && self
                        .drawing
                        .as_ref()
                        .is_none_or(|drawing| drawing.prop != prop)
                {
                    self.queue.push_back(prop.to_owned());
                }
                None
            }
        }
    }

    /// Forgets every thumbnail whose model changed, so it is drawn again when asked for.
    fn forget_stale(
        &mut self,
        editor_hashes: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<Handle<Image>> {
        let mut freed = Vec::new();
        self.ready.retain(|prop, ready| {
            let kept = editor_hashes(prop).as_deref() == Some(ready.model_hash.as_str());
            if !kept {
                freed.push(ready.image.clone());
            }
            kept
        });
        freed
    }
}

/// Draws the next queued thumbnail, one at a time, and takes finished ones.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    let files = &project.files;
    let model_hash = |prop: &str| -> Option<String> {
        let def = content.by_path::<PropDef>(prop)?.get(&content);
        files.hash_of(&def.model.path()).map(str::to_owned)
    };
    let freed = editor.form.prop_thumbnails.forget_stale(&model_hash);
    for image in freed {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&image);
    }
    let thumbnails = &mut editor.form.prop_thumbnails;
    if let Some(drawing) = &mut thumbnails.drawing {
        drawing.frames += 1;
        let loaded = world
            .get::<Children>(drawing.root)
            .is_some_and(|children| !children.is_empty());
        if loaded && drawing.readback.is_none() && drawing.frames >= SETTLE_FRAMES {
            drawing.readback = Some(
                world
                    .spawn(ReadbackOnce::texture(drawing.target.clone()))
                    .id(),
            );
        }
        let read = drawing
            .readback
            .and_then(|readback| super::taken(world, readback));
        let blank = read
            .as_ref()
            .is_some_and(|pixels| pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
        if blank {
            drawing.readback = None;
        } else if let Some(pixels) = read {
            let drawing = thumbnails.drawing.take().expect("a thumbnail being drawn");
            world.despawn(drawing.root);
            world.despawn(drawing.camera);
            let mut image = Image::new(
                bevy::render::render_resource::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                pixels,
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            );
            image.sampler = bevy::image::ImageSampler::linear();
            let image = world.resource_mut::<Assets<Image>>().add(image);
            let texture = world
                .resource_mut::<EguiUserTextures>()
                .add_image(EguiTextureHandle::Strong(image.clone()));
            thumbnails.ready.insert(
                drawing.prop,
                Ready {
                    model_hash: drawing.model_hash,
                    texture,
                    image,
                },
            );
        } else if drawing.frames > GIVE_UP_FRAMES {
            let drawing = thumbnails.drawing.take().expect("a thumbnail being drawn");
            world.despawn(drawing.root);
            world.despawn(drawing.camera);
        }
        return;
    }
    while let Some(prop) = thumbnails.queue.pop_front() {
        let Some(hash) = model_hash(&prop) else {
            continue;
        };
        let def = content
            .by_path::<PropDef>(&prop)
            .expect("a prop with a model hash")
            .get(&content);
        let path = def.model.path();
        let bounds = content
            .built::<GltfInfos>()
            .0
            .get(&path)
            .map(|info| info.bounds);
        let (center, radius) = match bounds {
            Some(bounds) => (
                (bounds.min + bounds.max) / 2.0,
                ((bounds.max - bounds.min).length() / 2.0).max(0.2),
            ),
            None => (Vec3::new(0.0, 0.5, 0.0), 1.0),
        };
        let target = {
            let mut image =
                Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            world.resource_mut::<Assets<Image>>().add(image)
        };
        let layers = RenderLayers::from_layers(&[LAYER]);
        let scene = world
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(path));
        let root = world
            .spawn((
                WorldAssetRoot(scene),
                Transform::default(),
                bevy::app::Propagate(layers.clone()),
            ))
            .id();
        world.spawn((
            lights::sun(Color::WHITE, SUN_LUX, SUN_ELEVATION, SUN_AZIMUTH),
            layers.clone(),
            ChildOf(root),
        ));
        let atmosphere = sky::atmosphere(
            &mut world.resource_mut::<Assets<ScatteringMedium>>(),
            Sky::Air,
            0.0,
        );
        world.spawn((atmosphere, layers.clone(), ChildOf(root)));
        let distance = radius / (FOV.radians().0 / 2.0).sin() * 1.1;
        let (sin_p, cos_p) = PITCH.radians().0.sin_cos();
        let back = Vec3::new(0.0, distance * sin_p, distance * cos_p);
        let eye = center + Quat::from_rotation_y(YAW.radians().0) * back;
        let camera = world
            .spawn((
                Camera3d::default(),
                Camera {
                    order: -100,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                world_camera::projection(SIZE, SIZE),
                world_camera::picture(),
                AmbientLight {
                    color: Color::WHITE,
                    brightness: AMBIENT_NITS,
                    ..default()
                },
                layers,
                Transform::from_translation(eye).looking_at(center, Vec3::Y),
            ))
            .id();
        thumbnails.drawing = Some(Drawing {
            prop,
            model_hash: hash,
            root,
            camera,
            target,
            frames: 0,
            readback: None,
        });
        break;
    }
}
