//! The stage's picture: an offscreen camera over sprites drawn with the game's frames, anchor and
//! row depths, and vfx pieces presented by the game's own presenters, mirroring whatever scene is
//! requested of it.

use std::collections::{HashMap, HashSet};
use ui::egui;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::prelude::*;
use bevy::shader::Shader;
use bevy_egui::EguiUserTextures;
use engine::core::content::{Content, fields_file};
use engine::core::geometry::Radians;
use engine::core::math::{Pos, Rgba};
use engine::core::render::{
    Animator, GroundDepth, RowDepth, RowDepths, ToScreen, dynamic_z, place_sprite,
};
use engine::core::time::{PlaybackRate, Seconds};
use engine::core::units::Meters;
use engine::core::vfx::render::{self as vfx_render, Presentation, SiteFrame, SiteTexture};
use engine::core::vfx::{
    ShaderError, SiteKind, VfxLayer, VfxParams, VfxShaderDef, VfxSite, shader_file,
};
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::actor::render::{ACTOR_ANCHOR, draw_actor_frame};
use engine::systems::actor::{Action, ActorModel, ModelId};
use engine::systems::area::{self, AreaId};
use engine::systems::item::ItemId;
use engine::systems::item::render::drop_sprite;

use super::{SIZE, ZOOM};
use crate::project::{ProjectFiles, hash};
use crate::views::area::scene;

const FIRST_LAYER: usize = 500;
const SPRITE_DEPTH_ABOVE_GROUND: f32 = 0.001;

/// What a staged sprite stands for: an actor the scene posed, or an entity of a live world.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Posed(usize),
    Live(Entity),
}

/// An actor as the stage draws it this frame.
pub struct ActorPose {
    pub source: Source,
    pub model: ModelId,
    pub action: Action,
    pub facing: Radians,
    pub tint: Rgba,
    pub attack_rate: PlaybackRate,
    pub at: Pos<Meters>,
    /// Held at this moment of the action, or animated from when the sprite came to stand in it.
    pub held: Option<Seconds>,
    pub plate: Option<PlateLabel>,
}

/// A vfx as the stage presents it this frame: its layers on a site, at a time, with its params.
pub struct VfxPose {
    pub owner: Source,
    pub layers: Vec<VfxLayer>,
    pub site: Site,
    pub heading: Radians,
    pub elapsed: Seconds,
    pub params: VfxParams,
}

/// Where a staged vfx plays, in the stage's own terms.
#[derive(Clone)]
pub enum Site {
    Actor(Source),
    World(VfxSite),
    Screen,
}

pub struct Frame<'a> {
    pub content: &'a Content,
    pub files: &'a ProjectFiles,
    pub area: AreaId,
    pub center: Pos<Meters>,
    pub now: f64,
    pub actors: Vec<ActorPose>,
    pub items: Vec<ItemId>,
    pub vfx: Vec<VfxPose>,
    pub drafts: HashMap<String, String>,
    pub behind: Option<SiteTexture>,
}

pub struct Stage {
    pub texture: egui::TextureId,
    /// Plates to hang over the actors, by the world point each hangs from.
    pub plates: Vec<(Vec2, PlateLabel)>,
    pub problems: Vec<String>,
    pub index: usize,
    camera: Entity,
    image: Handle<Image>,
    layer: RenderLayers,
    animator: Animator,
    actors: HashMap<Source, Staged>,
    items: Vec<Entity>,
    pieces: HashMap<Source, (String, Vec<Entity>)>,
    /// The last shader compiled for each shader key, by the hash of the text it was composed from.
    shaders: HashMap<String, (String, Result<Handle<Shader>, ShaderError>)>,
}

struct Staged {
    entity: Entity,
    model: ModelId,
}

impl Stage {
    pub fn spawn(world: &mut World, index: usize) -> Stage {
        let (image, texture) = scene::render_target(world, SIZE);
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
                layer.clone(),
            ))
            .id();
        Stage {
            texture,
            plates: Vec::new(),
            problems: Vec::new(),
            index,
            camera,
            image,
            layer,
            animator: Animator::default(),
            actors: HashMap::new(),
            items: Vec::new(),
            pieces: HashMap::new(),
            shaders: HashMap::new(),
        }
    }

    /// Brings the picture up to the frame: sprites posed and placed as the game places them, vfx
    /// presented over them, and names kept for the window to letter.
    pub fn render(&mut self, world: &mut World, scenes: &mut scene::AreaScenes, frame: &Frame) {
        let area = area::load(frame.content, frame.area);
        world.insert_resource(GroundDepth(area.sort_base()));
        world.insert_resource(RowDepths {
            height: area.size.height,
            base: area.sort_base(),
        });
        let eye = frame.center.to_screen();
        if let Some(mut transform) = world.get_mut::<Transform>(self.camera) {
            transform.translation.x = eye.x;
            transform.translation.y = eye.y;
        }
        self.problems.clear();
        self.plates = frame
            .actors
            .iter()
            .filter_map(|pose| {
                let hitbox = pose.model.get(frame.content).hitbox.tiles();
                let top = plate::head(pose.at, hitbox).to_screen();
                pose.plate.clone().map(|label| (top, label))
            })
            .collect();
        self.sync_actors(world, scenes, frame, area.size.height, area.sort_base());
        self.sync_items(world, scenes, frame);
        self.sync_vfx(world, scenes, frame);
    }

    pub fn despawn(mut self, world: &mut World) {
        self.clear(world);
        world.despawn(self.camera);
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&self.image);
    }

    fn clear(&mut self, world: &mut World) {
        for staged in self.actors.drain().map(|(_, staged)| staged) {
            world.despawn(staged.entity);
        }
        for item in self.items.drain(..) {
            world.despawn(item);
        }
        for piece in self.pieces.drain().flat_map(|(_, (_, pieces))| pieces) {
            world.despawn(piece);
        }
    }

    fn sync_actors(
        &mut self,
        world: &mut World,
        scenes: &mut scene::AreaScenes,
        frame: &Frame,
        height: f32,
        base: f32,
    ) {
        let wanted: HashSet<Source> = frame.actors.iter().map(|pose| pose.source).collect();
        let gone: Vec<Source> = self
            .actors
            .iter()
            .filter(|(source, staged)| {
                !wanted.contains(source)
                    || frame
                        .actors
                        .iter()
                        .any(|pose| pose.source == **source && pose.model != staged.model)
            })
            .map(|(source, _)| *source)
            .collect();
        for source in gone {
            if let Some(staged) = self.actors.remove(&source) {
                world.despawn(staged.entity);
            }
        }
        for pose in &frame.actors {
            let model = ActorModel::get(frame.content, pose.model);
            let image = scene::image_handle(scenes, frame.files, model.sheet().as_str(), world);
            let entity = match self.actors.get(&pose.source) {
                Some(staged) => staged.entity,
                None => {
                    let entity = world
                        .spawn((
                            Sprite {
                                image: image.clone(),
                                ..default()
                            },
                            ACTOR_ANCHOR,
                            Transform::default(),
                            RowDepth::default(),
                            self.layer.clone(),
                        ))
                        .id();
                    self.actors.insert(
                        pose.source,
                        Staged {
                            entity,
                            model: pose.model,
                        },
                    );
                    entity
                }
            };
            let elapsed = match pose.held {
                Some(held) => held,
                None => {
                    self.animator
                        .advance(entity, pose.action as u64, Seconds(frame.now as f32))
                        .elapsed
                }
            };
            let mut staged = world.entity_mut(entity);
            if let Some(mut sprite) = staged.get_mut::<Sprite>() {
                if sprite.image != image {
                    sprite.image = image;
                }
                draw_actor_frame(
                    &mut sprite,
                    model,
                    pose.action,
                    pose.facing,
                    elapsed,
                    pose.attack_rate,
                );
                let alpha = sprite.color.alpha();
                sprite.color = pose.tint.color().with_alpha(alpha);
            }
            if let Some(mut transform) = staged.get_mut::<Transform>() {
                place_sprite(&mut transform, pose.at);
            }
            staged.insert(RowDepth(dynamic_z(height, base, Meters(pose.at.y))));
        }
    }

    fn sync_items(&mut self, world: &mut World, scenes: &mut scene::AreaScenes, frame: &Frame) {
        if self.items.len() != frame.items.len() {
            for item in self.items.drain(..) {
                world.despawn(item);
            }
            for _ in &frame.items {
                let entity = world
                    .spawn((
                        Transform::default(),
                        RowDepth::default(),
                        self.layer.clone(),
                    ))
                    .id();
                self.items.push(entity);
            }
        }
        let count = frame.items.len() as f32;
        for (index, (item, entity)) in frame.items.iter().zip(&self.items).enumerate() {
            let icon = item.get(frame.content).icon.as_str();
            let image = scene::image_handle(scenes, frame.files, icon, world);
            let at = Pos::<Meters>::new(
                frame.center.x + (index as f32 - (count - 1.0) / 2.0) * 1.5,
                frame.center.y,
            );
            let depth = world.resource::<GroundDepth>().0 + SPRITE_DEPTH_ABOVE_GROUND;
            let mut staged = world.entity_mut(*entity);
            staged.insert(drop_sprite(image));
            if let Some(mut transform) = staged.get_mut::<Transform>() {
                place_sprite(&mut transform, at);
            }
            staged.insert(RowDepth(depth));
        }
    }

    fn sync_vfx(&mut self, world: &mut World, scenes: &mut scene::AreaScenes, frame: &Frame) {
        let wanted: HashMap<Source, String> = frame
            .vfx
            .iter()
            .map(|pose| (pose.owner, layers_signature(&pose.layers)))
            .collect();
        let outdated: Vec<Source> = self
            .pieces
            .iter()
            .filter(|(owner, (known, _))| wanted.get(*owner) != Some(known))
            .map(|(owner, _)| *owner)
            .collect();
        for owner in outdated {
            for piece in self
                .pieces
                .remove(&owner)
                .into_iter()
                .flat_map(|(_, pieces)| pieces)
            {
                world.despawn(piece);
            }
        }
        let mut overdrawn = HashSet::new();
        for pose in &frame.vfx {
            if !self.pieces.contains_key(&pose.owner) {
                let files = frame.files;
                let pieces = vfx_render::pieces_for(world, &pose.layers, &mut |world, sheet| {
                    scene::image_handle(scenes, files, sheet.as_str(), world)
                });
                for piece in &pieces {
                    world.entity_mut(*piece).insert(self.layer.clone());
                }
                self.pieces
                    .insert(pose.owner, (wanted[&pose.owner].clone(), pieces));
            }
            let pieces = self.pieces[&pose.owner].1.clone();
            let site = self.frame_of(world, &pose.site, pose.heading, frame);
            let mut compiled: HashMap<String, Handle<Shader>> = HashMap::new();
            for layer in &pose.layers {
                if let VfxLayer::Shader { shader, .. } = layer {
                    let key = shader.path(frame.content).to_owned();
                    match self.compile(world, &key, frame.files, &frame.drafts) {
                        Ok(handle) => {
                            compiled.insert(key, handle);
                        }
                        Err(problem) => self.problems.push(problem),
                    }
                }
            }
            let content = frame.content;
            vfx_render::present_layers(
                world,
                &Presentation {
                    layers: &pose.layers,
                    pieces: &pieces,
                    frame: site.as_ref(),
                    elapsed: pose.elapsed,
                    params: &pose.params,
                    content,
                    screen_layer: &self.layer,
                },
                &|shader| compiled.get(shader.path(content)).cloned(),
                &mut overdrawn,
            );
        }
        vfx_render::settle_overdrawn(world, &overdrawn);
    }

    /// The site frame of a staged actor, measured as the game measures a sprite's.
    pub fn actor_frame(&self, world: &World, source: Source) -> Option<SiteFrame> {
        let staged = self.actors.get(&source)?;
        SiteFrame::sprite(world, staged.entity, SiteKind::Actor)
    }

    fn frame_of(
        &self,
        world: &World,
        site: &Site,
        heading: Radians,
        frame: &Frame,
    ) -> Option<SiteFrame> {
        match site {
            Site::Actor(source) => self
                .actor_frame(world, *source)
                .map(|actor| actor.headed(heading)),
            Site::World(VfxSite::Actor(entity)) => self
                .actor_frame(world, Source::Live(*entity))
                .map(|actor| actor.headed(heading)),
            Site::World(VfxSite::Area(footprint)) => Some(SiteFrame {
                depth: Some(world.resource::<GroundDepth>().0),
                ..SiteFrame::area(footprint)
            }),
            Site::World(VfxSite::Path(points)) => {
                let south = points.iter().map(|point| point.y).fold(f32::MIN, f32::max);
                Some(SiteFrame {
                    depth: Some(world.resource::<RowDepths>().at(Meters(south))),
                    ..SiteFrame::path(points)
                })
            }
            Site::World(VfxSite::Object(_) | VfxSite::Screen | VfxSite::Widget(_)) => None,
            Site::Screen => {
                let size = Vec2::new(SIZE[0] as f32, SIZE[1] as f32) / ZOOM;
                let mut screen = SiteFrame::screen(size, frame.behind.clone(), 1.0);
                screen.center = frame.center.to_screen();
                Some(screen)
            }
        }
    }

    /// The shader row as it stands in the files, compiled from its draft or its file, kept until
    /// either changes.
    fn compile(
        &mut self,
        world: &mut World,
        key: &str,
        files: &ProjectFiles,
        drafts: &HashMap<String, String>,
    ) -> Result<Handle<Shader>, String> {
        let row = files
            .json(&fields_file(key))
            .ok_or_else(|| format!("no shader row {key}"))?;
        let def: VfxShaderDef =
            serde_json::from_value((*row).clone()).map_err(|_| format!("{key} does not parse"))?;
        let source = drafts
            .get(key)
            .cloned()
            .or_else(|| files.text(&shader_file(key)))
            .unwrap_or_default();
        let composed = def.compose(&source);
        let fingerprint = hash(composed.as_bytes());
        let fresh = self
            .shaders
            .get(key)
            .is_none_or(|(known, _)| *known != fingerprint);
        if fresh {
            let verdict = def.check_source(&source).map(|composed| {
                let path = format!("stage/{key}-{fingerprint}.wgsl");
                world
                    .resource_mut::<Assets<Shader>>()
                    .add(Shader::from_wgsl(composed, path))
            });
            self.shaders.insert(key.to_owned(), (fingerprint, verdict));
        }
        match &self.shaders[key].1 {
            Ok(handle) => Ok(handle.clone()),
            Err(error) => Err(format!("{key}: {}", error.message)),
        }
    }
}

fn layers_signature(layers: &[VfxLayer]) -> String {
    layers
        .iter()
        .map(|layer| match layer {
            VfxLayer::Sprite { sheet, .. } => format!("sprite:{}", sheet.as_str()),
            VfxLayer::Shader { shader, .. } => format!("shader:{}", shader.index()),
        })
        .collect::<Vec<_>>()
        .join(",")
}
