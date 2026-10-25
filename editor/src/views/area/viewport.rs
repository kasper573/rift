//! The viewport of an area: the game's own renderer drawing the edited area into a picture the
//! view shows, from the view's orbit camera, on render layers of its own so several viewports can
//! stand in one world. The view asks for what it wants drawn each frame; the viewport stands the
//! scenery when the picture changes, swaps the terrain while a brush stroke shapes it, and keeps
//! the ghost of what is being placed and the cast of residents under the pointer.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::content::Content;
use engine::core::ground::{Heightfield, Paint};
use engine::core::render::camera;
use engine::core::render::world::mannequin::Mannequin;
use engine::core::render::world::mannequin::mannequin;
use engine::core::render::world::sky::{self, Daylight};
use engine::core::time::Seconds;
use engine::systems::area::scenery::{self, Scenery};
use engine::systems::area::{Area, Picture};
use ui::egui;

use super::camera::OrbitCamera;
use crate::app::Editor;

const FIRST_LAYER: usize = 400;

/// What a view asks its viewport to show this frame.
pub struct Request {
    pub key: String,
    pub size: [u32; 2],
    pub camera: OrbitCamera,
    pub picture: Arc<Picture>,
    /// The number of the snapshot the picture came from: a new one stands the scenery again.
    pub made: u64,
    pub content: Content,
    pub generation: u64,
    /// The terrain as a brush stroke is shaping it, shown in place of the picture's.
    pub stroke: Option<StrokeTerrain>,
    pub ghost: Option<Ghost>,
    pub cast: Vec<Standing>,
}

/// A terrain mid-stroke, numbered so it is laid again only when the stroke moves on.
#[derive(Clone)]
pub struct StrokeTerrain {
    pub revision: u64,
    pub heights: Arc<Heightfield>,
    pub paint: Arc<Paint>,
}

/// A prop following the pointer before it is placed.
#[derive(Clone, PartialEq)]
pub struct Ghost {
    pub model: String,
    pub transform: Transform,
}

/// A rig standing in the area as the view shows it: a resident or one of a population's preview,
/// looping its clip, or a model held at one moment of a clip on a turntable.
#[derive(Clone, PartialEq)]
pub struct Standing {
    pub model: String,
    pub clip: Option<usize>,
    pub transform: Transform,
    pub held: Option<Seconds>,
}

impl Standing {
    /// Whether the same rig in the same clip stands here, however it is placed or held.
    fn same_rig(&self, other: &Standing) -> bool {
        self.model == other.model && self.clip == other.clip
    }
}

/// The picture a viewport last drew, for the view to show and draw over.
#[derive(Clone, Copy)]
pub struct Shown {
    pub texture: egui::TextureId,
    pub size: [u32; 2],
    /// The camera the picture was drawn from, which the overlays project through so they agree
    /// with it.
    pub camera: OrbitCamera,
}

#[derive(Default)]
pub struct Viewports {
    shown: HashMap<String, Viewport>,
    requests: Vec<Request>,
    layers: Vec<usize>,
    mirrored: Option<u64>,
}

struct Viewport {
    layer: usize,
    root: Entity,
    camera: Entity,
    image: Handle<Image>,
    texture: egui::TextureId,
    size: [u32; 2],
    rendered: OrbitCamera,
    stood: Option<Stood>,
    stroke: Option<u64>,
    ghost: Option<(String, Entity)>,
    cast: Option<(Vec<Standing>, Vec<Entity>)>,
}

struct Stood {
    scenery: Scenery,
    made: u64,
    generation: u64,
    area: Arc<Area>,
}

impl Viewports {
    /// Asks for `request` to be drawn for the next frame.
    pub fn request(&mut self, request: Request) {
        self.requests.push(request);
    }

    /// The picture last drawn for the view `key`.
    pub fn shown(&self, key: &str) -> Option<Shown> {
        let viewport = self.shown.get(key)?;
        Some(Shown {
            texture: viewport.texture,
            size: viewport.size,
            camera: viewport.rendered,
        })
    }
}

/// Draws every viewport asked for this frame and lets go of the rest.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let requests = std::mem::take(&mut editor.views.area.viewports.requests);
    let viewports = &mut editor.views.area.viewports;
    let loaded = editor
        .project
        .as_ref()
        .and_then(|project| Some((project.content.clone()?, project.generation)));
    let gone: Vec<String> = viewports
        .shown
        .keys()
        .filter(|key| !requests.iter().any(|request| request.key == **key) || loaded.is_none())
        .cloned()
        .collect();
    for key in gone {
        if let Some(viewport) = viewports.shown.remove(&key) {
            viewports.layers.retain(|layer| *layer != viewport.layer);
            viewport.despawn(world);
        }
    }
    let Some((content, generation)) = loaded else {
        viewports.mirrored = None;
        return;
    };
    if viewports.mirrored != Some(generation) {
        world.insert_resource(content.clone());
        viewports.mirrored = Some(generation);
    }
    for request in requests {
        let viewport = viewports
            .shown
            .entry(request.key.clone())
            .or_insert_with(|| {
                let layer = (FIRST_LAYER..)
                    .find(|layer| !viewports.layers.contains(layer))
                    .unwrap_or(FIRST_LAYER);
                viewports.layers.push(layer);
                Viewport::spawn(world, layer, request.size)
            });
        viewport.draw(world, request);
    }
}

impl Viewport {
    fn spawn(world: &mut World, layer: usize, size: [u32; 2]) -> Viewport {
        let (image, texture) = render_target(world, size);
        let root = world
            .spawn((
                Transform::default(),
                Visibility::default(),
                bevy::app::Propagate(RenderLayers::from_layers(&[layer])),
            ))
            .id();
        let camera = world
            .spawn((
                Camera3d::default(),
                Camera {
                    order: -40 - (layer - FIRST_LAYER) as isize,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                camera::projection(size[0], size[1]),
                camera::picture(),
                RenderLayers::from_layers(&[layer]),
                Transform::default(),
            ))
            .id();
        Viewport {
            layer,
            root,
            camera,
            image,
            texture,
            size,
            rendered: OrbitCamera::default(),
            stood: None,
            stroke: None,
            ghost: None,
            cast: None,
        }
    }

    fn draw(&mut self, world: &mut World, request: Request) {
        if request.size != self.size && request.size[0] > 0 && request.size[1] > 0 {
            world
                .resource_mut::<EguiUserTextures>()
                .remove_image(&self.image);
            let (image, texture) = render_target(world, request.size);
            self.image = image;
            self.texture = texture;
            self.size = request.size;
            if let Ok(mut camera) = world.get_entity_mut(self.camera) {
                camera.insert((
                    RenderTarget::Image(self.image.clone().into()),
                    camera::projection(request.size[0], request.size[1]),
                ));
            }
        }
        let area = request.picture.area.clone();
        let stale = self.stood.as_ref().is_none_or(|stood| {
            stood.made != request.made || stood.generation != request.generation
        });
        if stale && scenery::shaders_ready(world, &request.content, &area) {
            if let Some(stood) = self.stood.take() {
                scenery::despawn(world, stood.scenery);
            }
            let stood = scenery::spawn(
                world,
                &request.content,
                &area,
                RenderLayers::from_layers(&[self.layer]),
            );
            world.entity_mut(stood.root).insert(ChildOf(self.root));
            let sun = scenery::light(world, &area, Daylight::Fixed, stood, self.camera);
            world.entity_mut(self.camera).insert(sky::haze(sun));
            self.stood = Some(Stood {
                scenery: stood,
                made: request.made,
                generation: request.generation,
                area: area.clone(),
            });
            self.stroke = None;
        }
        world.insert_resource(scenery::climate(&area));
        let ground = area.heights.height_at(request.camera.target);
        if let Some(mut transform) = world.get_mut::<Transform>(self.camera) {
            *transform = request.camera.transform(ground);
        }
        self.rendered = request.camera;
        self.shape(world, &request);
        self.haunt(world, request.ghost);
        self.people(world, request.cast);
    }

    /// Lays the terrain as the stroke shapes it, once per step of the stroke.
    fn shape(&mut self, world: &mut World, request: &Request) {
        let Some(stood) = &self.stood else {
            return;
        };
        match &request.stroke {
            Some(stroke) if self.stroke != Some(stroke.revision) => {
                let mesh = scenery::terrain_mesh(
                    &request.content,
                    &stood.area,
                    &stroke.heights,
                    &stroke.paint,
                );
                let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
                if let Ok(mut terrain) = world.get_entity_mut(stood.scenery.terrain) {
                    terrain.insert(Mesh3d(mesh));
                }
                self.stroke = Some(stroke.revision);
            }
            Some(_) => {}
            None => {
                if self.stroke.take().is_some() {
                    let mesh = scenery::terrain_mesh(
                        &request.content,
                        &stood.area,
                        &stood.area.heights,
                        &stood.area.paint,
                    );
                    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
                    if let Ok(mut terrain) = world.get_entity_mut(stood.scenery.terrain) {
                        terrain.insert(Mesh3d(mesh));
                    }
                }
            }
        }
    }

    /// Keeps the ghost of what is being placed under the pointer.
    fn haunt(&mut self, world: &mut World, ghost: Option<Ghost>) {
        let Some(ghost) = ghost else {
            if let Some((_, entity)) = self.ghost.take() {
                world.despawn(entity);
            }
            return;
        };
        let entity = match &self.ghost {
            Some((model, entity)) if *model == ghost.model => *entity,
            _ => {
                if let Some((_, entity)) = self.ghost.take() {
                    world.despawn(entity);
                }
                let scene = world
                    .resource::<AssetServer>()
                    .load(GltfAssetLabel::Scene(0).from_asset(ghost.model.clone()));
                let entity = world
                    .spawn((WorldAssetRoot(scene), ghost.transform, ChildOf(self.root)))
                    .id();
                self.ghost = Some((ghost.model.clone(), entity));
                entity
            }
        };
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            *transform = ghost.transform;
        }
    }

    /// Stands the cast where the view puts them, anew when it changes.
    /// Stands the cast: a rig already standing in its clip is moved and held where it is asked
    /// to, the rest are spawned anew.
    fn people(&mut self, world: &mut World, cast: Vec<Standing>) {
        if self
            .cast
            .as_ref()
            .is_some_and(|(standing, _)| *standing == cast)
        {
            return;
        }
        let (stood, entities) = self.cast.take().unwrap_or_default();
        let mut spare: Vec<(Standing, Entity)> = stood.into_iter().zip(entities).collect();
        let mut kept: Vec<Entity> = Vec::with_capacity(cast.len());
        for standing in &cast {
            let reused = spare
                .iter()
                .position(|(old, _)| old.same_rig(standing))
                .map(|index| spare.remove(index).1);
            let entity = match reused {
                Some(entity) => {
                    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                        *transform = standing.transform;
                    }
                    if let Some(mut mannequin) = world.get_mut::<Mannequin>(entity) {
                        mannequin.hold(standing.held);
                    }
                    entity
                }
                None => match standing.clip {
                    Some(clip) => {
                        let (scene, mut puppet) = mannequin(world, &standing.model, clip);
                        puppet.hold(standing.held);
                        world
                            .spawn((scene, puppet, standing.transform, ChildOf(self.root)))
                            .id()
                    }
                    None => {
                        let scene = world
                            .resource::<AssetServer>()
                            .load(GltfAssetLabel::Scene(0).from_asset(standing.model.clone()));
                        world
                            .spawn((
                                WorldAssetRoot(scene),
                                standing.transform,
                                ChildOf(self.root),
                            ))
                            .id()
                    }
                },
            };
            kept.push(entity);
        }
        for (_, entity) in spare {
            world.despawn(entity);
        }
        self.cast = Some((cast, kept));
    }

    fn despawn(self, world: &mut World) {
        if let Some(stood) = self.stood {
            scenery::despawn(world, stood.scenery);
        }
        world.despawn(self.root);
        world.despawn(self.camera);
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&self.image);
    }
}

/// An offscreen picture of `size` pixels a camera renders to, and the egui texture that shows it.
fn render_target(world: &mut World, size: [u32; 2]) -> (Handle<Image>, egui::TextureId) {
    let mut image = Image::new_target_texture(
        size[0].max(1),
        size[1].max(1),
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.sampler = ImageSampler::linear();
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    (image, texture)
}
