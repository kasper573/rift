//! A render of every area the editor maps or points at, drawn once by the game's renderer from
//! straight above, read back and kept as a texture until the area's files change. The editor's
//! alone: the game never draws one.

use std::collections::{HashMap, VecDeque};

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::prepass::{DepthPrepass, NormalPrepass};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::ReadbackOnce;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::math::{Pos, Rect as MRect};
use engine::core::render::camera::{self as world_camera, FOV};
use engine::core::render::world::sky::Daylight;
use engine::core::units::{Meters, TileSize};
use engine::systems::area::scenery::{self, Scenery};
use engine::systems::area::{self, AreaDef};
use ui::egui::{self, Color32, TextureId};

use super::taken;
use crate::app::Editor;
use crate::project::ProjectFiles;
use crate::views::navigation::Viewport;

/// How many pixels the longer side of an area's render spans.
pub const LONGEST: u32 = 512;
const LAYER: usize = 398;
/// How many frames the scenery gets to load and settle before it is read back.
const SETTLE_FRAMES: u32 = 8;
/// How many frames an area that never draws is waited on.
const GIVE_UP_FRAMES: u32 = 900;

/// An area as seen from above, ready to draw where a canvas puts its ground.
#[derive(Clone, Copy)]
pub struct AreaArt {
    pub texture: TextureId,
    pub bounds: MRect<Meters>,
}

impl AreaArt {
    /// Draws the area where `canvas` puts it, clipped to the painter.
    pub fn draw(&self, painter: &egui::Painter, canvas: Viewport<Meters>, tint: Color32) {
        painter.image(self.texture, canvas.rect(self.bounds), super::FULL_UV, tint);
    }
}

/// The renders made so far, by area path, and the queue of those still to draw.
#[derive(Default)]
pub struct AreaThumbnails {
    ready: HashMap<String, Ready>,
    queue: VecDeque<String>,
    drawing: Option<Drawing>,
}

struct Ready {
    stamp: String,
    texture: TextureId,
    image: Handle<Image>,
    bounds: MRect<Meters>,
}

struct Drawing {
    area: String,
    stamp: String,
    bounds: MRect<Meters>,
    scenery: Option<Scenery>,
    camera: Entity,
    target: Handle<Image>,
    readback: Option<Entity>,
    frames: u32,
}

impl AreaThumbnails {
    /// The render of the area at `area` as its files stand, which `stamp` names; queued for
    /// drawing when it has none of that yet.
    pub fn art(&mut self, area: &str, stamp: &str) -> Option<AreaArt> {
        if let Some(ready) = self.ready.get(area)
            && ready.stamp == stamp
        {
            return Some(AreaArt {
                texture: ready.texture,
                bounds: ready.bounds,
            });
        }
        let queued = self.queue.iter().any(|queued| queued == area)
            || self
                .drawing
                .as_ref()
                .is_some_and(|drawing| drawing.area == area && drawing.stamp == stamp);
        if !queued {
            self.queue.push_back(area.to_owned());
        }
        None
    }
}

/// What the files of the area bundle at `area` hash to, all together: the name of the render
/// those files make.
pub fn stamp(files: &ProjectFiles, area: &str) -> String {
    let hashes: String = files
        .files_of(area)
        .iter()
        .filter_map(|file| files.hash_of(file))
        .collect();
    crate::project::files::hash(hashes.as_bytes())
}

/// Draws the area at `path` into `rect`, `center` in its middle at `px_per_meter`, once it is
/// rendered; says whether it was.
pub fn draw_area(
    editor: &mut Editor,
    painter: &egui::Painter,
    path: &str,
    rect: egui::Rect,
    center: Pos<Meters>,
    px_per_meter: f32,
) -> bool {
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let stamp = stamp(&project.files, path);
    let Some(art) = editor.views.areas.art(path, &stamp) else {
        painter.ctx().request_repaint();
        return false;
    };
    let canvas = Viewport {
        screen: rect,
        center,
        zoom: px_per_meter / crate::views::navigation::PX_PER_METER,
    };
    art.draw(&painter.with_clip_rect(rect), canvas, Color32::WHITE);
    true
}

/// Draws the next queued area, one at a time, and takes finished ones.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    let files = &project.files;
    let thumbnails = &mut editor.views.areas;
    if let Some(drawing) = &mut thumbnails.drawing {
        drawing.frames += 1;
        let area = content
            .by_path::<AreaDef>(&drawing.area)
            .and_then(|id| area::loaded(&content, id).cloned());
        match (&drawing.scenery, area) {
            (None, Some(area)) => {
                if scenery::shaders_ready(world, &content, &area) {
                    let layers = RenderLayers::from_layers(&[LAYER]);
                    let stood = scenery::spawn(world, &content, &area, layers);
                    scenery::light(world, &area, Daylight::Fixed, stood, drawing.camera);
                    drawing.scenery = Some(stood);
                    drawing.frames = 0;
                }
            }
            (None, None) => {
                let drawing = thumbnails.drawing.take().expect("an area being drawn");
                drawing.despawn(world);
                return;
            }
            (Some(_), _) => {}
        }
        if drawing.scenery.is_some()
            && drawing.readback.is_none()
            && drawing.frames >= SETTLE_FRAMES
        {
            drawing.readback = Some(
                world
                    .spawn(ReadbackOnce::texture(drawing.target.clone()))
                    .id(),
            );
        }
        let read = drawing.readback.and_then(|readback| taken(world, readback));
        match read {
            Some(pixels) if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) => {
                drawing.readback = None;
            }
            Some(pixels) => {
                let drawing = thumbnails.drawing.take().expect("an area being drawn");
                let size = world
                    .resource::<Assets<Image>>()
                    .get(&drawing.target)
                    .map(|target| target.size())
                    .unwrap_or(UVec2::ONE);
                let (area, stamp, bounds) =
                    (drawing.area.clone(), drawing.stamp.clone(), drawing.bounds);
                drawing.despawn(world);
                let mut image = Image::new(
                    bevy::render::render_resource::Extent3d {
                        width: size.x,
                        height: size.y,
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
                if let Some(old) = thumbnails.ready.insert(
                    area,
                    Ready {
                        stamp,
                        texture,
                        image,
                        bounds,
                    },
                ) {
                    world
                        .resource_mut::<EguiUserTextures>()
                        .remove_image(&old.image);
                }
            }
            None => {}
        }
        if let Some(drawing) = &thumbnails.drawing
            && drawing.frames > GIVE_UP_FRAMES
        {
            let drawing = thumbnails.drawing.take().expect("an area being drawn");
            drawing.despawn(world);
        }
        return;
    }
    while let Some(key) = thumbnails.queue.pop_front() {
        let Some(area) = content
            .by_path::<AreaDef>(&key)
            .and_then(|id| area::loaded(&content, id).cloned())
        else {
            continue;
        };
        let stamp = stamp(files, &key);
        let bounds = area.size.bounds();
        let (width, height) = (area.size.width.max(1.0), area.size.height.max(1.0));
        let size = if width >= height {
            [
                LONGEST,
                ((LONGEST as f32 * height / width).round() as u32).max(1),
            ]
        } else {
            [
                ((LONGEST as f32 * width / height).round() as u32).max(1),
                LONGEST,
            ]
        };
        let target = {
            let mut image =
                Image::new_target_texture(size[0], size[1], TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            world.resource_mut::<Assets<Image>>().add(image)
        };
        let center = Vec3::new(width / 2.0, 0.0, height / 2.0);
        let distance = height / 2.0 / (FOV.radians().0 / 2.0).tan() * 1.02;
        let camera = world
            .spawn((
                Camera3d::default(),
                Camera {
                    order: -101,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                world_camera::projection(size[0], size[1]),
                Msaa::Off,
                DepthPrepass,
                NormalPrepass,
                Tonemapping::None,
                DebandDither::Disabled,
                RenderLayers::from_layers(&[LAYER]),
                Transform::from_translation(center + Vec3::Y * distance)
                    .looking_at(center, -Vec3::Z),
            ))
            .id();
        thumbnails.drawing = Some(Drawing {
            area: key,
            stamp,
            bounds,
            scenery: None,
            camera,
            target,
            readback: None,
            frames: 0,
        });
        break;
    }
}

impl Drawing {
    fn despawn(self, world: &mut World) {
        if let Some(scenery) = self.scenery {
            scenery::despawn(world, scenery);
        }
        world.despawn(self.camera);
    }
}
