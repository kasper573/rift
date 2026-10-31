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
use engine::core::audio::playback::{self, PlaySfx, SfxPlace};
use engine::core::content::Content;
use engine::core::ground::{Heightfield, Paint};
use engine::core::render::camera;
use engine::core::render::world::body::{self, Body, Figure};
use engine::core::render::world::layers::Layered;
use engine::core::render::world::mannequin::Mannequin;
use engine::core::render::world::mannequin::mannequin;
use engine::core::render::world::rig_clip;
use engine::core::render::world::sky::Daylight;
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::systems::actor::Action;
use engine::systems::actor::layering::{self, Layering};
use engine::systems::actor::model::{ActorModel, ModelId};
use engine::systems::actor::render;
use engine::systems::actor::rigging::BuiltSkeleton;
use engine::systems::actor::skeleton::Cue;
use engine::systems::actor::stance::HandIndex;
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
    pub body: Figure,
    pub clip: Option<StageClip>,
    pub transform: Transform,
    pub held: Option<Seconds>,
    /// The cues of its clip it crossed since the last frame, which it sounds where it stands as a
    /// body in the game does.
    pub crossed: Vec<Cue>,
    pub props: Vec<StandingProp>,
    /// The layers of its skeleton the rig runs over its clip, when a stage asks for them.
    pub layers: Option<StageLayers>,
}

/// A clip a stage plays on a rig: the animation at `animation` of the glTF file at `file`, played
/// as a skeleton plays it, through `pairs` of our joints and his when it is made on a source rig.
#[derive(Clone, Debug, PartialEq)]
pub struct StageClip {
    pub file: String,
    pub animation: usize,
    pub pairs: Option<Vec<(String, String)>>,
}

impl StageClip {
    /// The clip at `played` among `skeleton`'s, as the game plays it.
    pub fn played(skeleton: &BuiltSkeleton, played: usize) -> StageClip {
        let clip = &skeleton.clips[played];
        StageClip {
            file: clip.clip.file().as_str().to_owned(),
            animation: clip.animation,
            pairs: clip
                .source
                .map(|source| skeleton.def.sources[source].joint_pairs()),
        }
    }
}

/// What a stood rig's layers are told each frame, as the game tells a puppet's: the model whose
/// skeleton lists them, the action shown, and whether a foot came down.
#[derive(Clone, Copy, PartialEq)]
pub struct StageLayers {
    pub model: ModelId,
    pub action: Action,
    pub stepped: bool,
}

/// The layers a stood rig was asked to run, and whether they have been put on its joints.
#[derive(Component)]
struct StageLayering {
    asked: StageLayers,
    put: bool,
}

/// A prop a standing rig holds in `hand`: the scene of `model` under the rig's joint `joint`, placed
/// there by `transform`.
#[derive(Clone, PartialEq)]
pub struct StandingProp {
    pub hand: HandIndex,
    pub joint: String,
    pub model: String,
    pub transform: Transform,
}

/// The props a stood rig holds, and those hung so far.
#[derive(Component)]
struct StageProps {
    props: Vec<StandingProp>,
    hung: Vec<Entity>,
}

impl Standing {
    /// Whether the same rig in the same clip stands here, however it is placed or held.
    fn same_rig(&self, other: &Standing) -> bool {
        self.body == other.body && self.clip == other.clip
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

/// How far around the camera's target the covers, props and lights stand, in the camera's
/// distance from it: the ground it takes in, and as far again around.
const VIEW_REACH_PER_DISTANCE: f32 = 4.0;

struct Stood {
    scenery: Scenery,
    made: u64,
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
    hang_props(world);
    run_layers(world);
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
        let stale = self
            .stood
            .as_ref()
            .is_none_or(|stood| stood.made != request.made);
        if stale && scenery::shaders_ready(world, &request.content, &area) {
            let stood = scenery::spawn(
                world,
                &request.content,
                &area,
                RenderLayers::from_layers(&[self.layer]),
            );
            if let Some(old) = self.stood.take() {
                scenery::despawn(world, old.scenery);
            }
            world.entity_mut(stood.root).insert(ChildOf(self.root));
            scenery::light(world, &area, Daylight::Fixed, stood, self.camera);
            self.stood = Some(Stood {
                scenery: stood,
                made: request.made,
                area: area.clone(),
            });
            self.stroke = None;
        }
        world.insert_resource(scenery::climate(&area));
        if let Some(stood) = &self.stood {
            scenery::view_from(
                world,
                stood.scenery,
                request.camera.target,
                Meters(request.camera.distance.0 * VIEW_REACH_PER_DISTANCE),
            );
        }
        let ground = area.heights.height_at(request.camera.target);
        if let Some(mut transform) = world.get_mut::<Transform>(self.camera) {
            *transform = request.camera.transform(ground);
        }
        self.rendered = request.camera;
        self.shape(world, &request);
        self.haunt(world, request.ghost);
        sound(world, &area, &request.cast);
        self.people(world, request.cast);
    }

    /// Lays the terrain as the stroke shapes it, once per step of the stroke.
    fn shape(&mut self, world: &mut World, request: &Request) {
        let Some(stood) = &self.stood else {
            return;
        };
        match &request.stroke {
            Some(stroke) if self.stroke != Some(stroke.revision) => {
                scenery::lay_ground(world, stood.scenery, &stroke.heights, &stroke.paint);
                self.stroke = Some(stroke.revision);
            }
            Some(_) => {}
            None => {
                if self.stroke.take().is_some() {
                    scenery::lay_ground(
                        world,
                        stood.scenery,
                        &stood.area.heights,
                        &stood.area.paint,
                    );
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
                let scene = engine::core::assets::model_scene(
                    world.resource::<AssetServer>(),
                    ghost.model.clone(),
                );
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
                    rehang(world, entity, &standing.props);
                    ask_layers(world, entity, standing.layers);
                    entity
                }
                None => {
                    let rig = world
                        .spawn((
                            standing.transform,
                            Visibility::default(),
                            StageProps {
                                props: standing.props.clone(),
                                hung: Vec::new(),
                            },
                            ChildOf(self.root),
                        ))
                        .id();
                    if let Some(clip) = &standing.clip {
                        let tree = standing.body.rig.clone();
                        let made = rig_clip::rig_clip(
                            world,
                            tree.clone(),
                            &clip.file,
                            clip.animation,
                            rig_clip::maker(tree, clip.pairs.clone()),
                        );
                        let mut mannequin = mannequin(world, made);
                        mannequin.hold(standing.held);
                        world.entity_mut(rig).insert(mannequin);
                    }
                    body::dress(world, rig, &standing.body);
                    ask_layers(world, rig, standing.layers);
                    rig
                }
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

/// Sounds the cues each of `cast` crossed, on the ground of `area` under it, as the game sounds a
/// body's. A view's stage is the one thing heard, so its sounds play centred rather than from where
/// the rig stands.
fn sound(world: &mut World, area: &Area, cast: &[Standing]) {
    for standing in cast {
        let crossed: Vec<&Cue> = standing.crossed.iter().collect();
        let at = camera::ground_pos(standing.transform.translation);
        let (sounds, _) = render::cue_sounds(&crossed, Some(area), at);
        for sound in sounds {
            playback::play(
                world,
                PlaySfx {
                    place: SfxPlace::Interface,
                    ..sound
                },
            );
        }
    }
}

/// Gives the stood rig `rig` the props `props`: moved in place where the same props are held
/// already, else taken down to be hung anew.
fn rehang(world: &mut World, rig: Entity, props: &[StandingProp]) {
    let Some(mut held) = world.get_mut::<StageProps>(rig) else {
        return;
    };
    let same = held.props.len() == props.len()
        && held
            .props
            .iter()
            .zip(props)
            .all(|(old, new)| old.joint == new.joint && old.model == new.model);
    if same {
        held.props = props.to_vec();
        let moved: Vec<(Entity, Transform)> = held
            .hung
            .iter()
            .zip(props)
            .map(|(&prop, standing)| (prop, standing.transform))
            .collect();
        for (prop, transform) in moved {
            if let Some(mut at) = world.get_mut::<Transform>(prop) {
                *at = transform;
            }
        }
        return;
    }
    let gone = std::mem::take(&mut held.hung);
    held.props = props.to_vec();
    for prop in gone {
        if let Ok(prop) = world.get_entity_mut(prop) {
            prop.despawn();
        }
    }
}

/// Hangs each stood rig's props under their joints once its scene has spawned them.
fn hang_props(world: &mut World) {
    let waiting: Vec<(Entity, Vec<StandingProp>)> = world
        .query::<(Entity, &StageProps)>()
        .iter(world)
        .filter(|(_, held)| held.hung.is_empty() && !held.props.is_empty())
        .map(|(rig, held)| (rig, held.props.clone()))
        .collect();
    for (rig, props) in waiting {
        let joints: Vec<Option<Entity>> = props
            .iter()
            .map(|prop| {
                world
                    .get::<Body>(rig)
                    .and_then(|body| body.joint(&prop.joint))
            })
            .collect();
        if joints.iter().any(Option::is_none) {
            continue;
        }
        let hung: Vec<Entity> = props
            .iter()
            .zip(joints.into_iter().flatten())
            .map(|(prop, joint)| {
                let scene = engine::core::assets::model_scene(
                    world.resource::<AssetServer>(),
                    prop.model.clone(),
                );
                world
                    .spawn((
                        WorldAssetRoot(scene),
                        prop.transform,
                        Visibility::default(),
                        ChildOf(joint),
                    ))
                    .id()
            })
            .collect();
        if let Some(mut held) = world.get_mut::<StageProps>(rig) {
            held.hung = hung;
        }
    }
}

/// Tells the stood rig `rig` what its layers run during this frame, or takes them away.
fn ask_layers(world: &mut World, rig: Entity, asked: Option<StageLayers>) {
    let Ok(mut entity) = world.get_entity_mut(rig) else {
        return;
    };
    match asked {
        Some(asked) => match entity.get_mut::<StageLayering>() {
            Some(mut layering) if layering.asked.model == asked.model => layering.asked = asked,
            _ => {
                entity
                    .insert(StageLayering { asked, put: false })
                    .remove::<(Layered, Layering)>();
            }
        },
        None => {
            entity.remove::<(StageLayering, Layered, Layering)>();
        }
    }
}

/// Puts each stood rig's layers on its joints once its scene has spawned them, and drives them as
/// the game drives a puppet's.
fn run_layers(world: &mut World) {
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let waiting: Vec<(Entity, ModelId)> = world
        .query::<(Entity, &StageLayering)>()
        .iter(world)
        .filter(|(_, layering)| !layering.put)
        .map(|(rig, layering)| (rig, layering.asked.model))
        .collect();
    for (rig, model) in waiting {
        let Some(model) = ActorModel::loaded(&content, model) else {
            continue;
        };
        let Some(joints) = world.get::<Body>(rig).map(|body| body.joints().clone()) else {
            continue;
        };
        let rigged = model
            .skeleton()
            .rig
            .joints()
            .all(|joint| joints.contains_key(&model.skeleton().rig.nodes[joint].name));
        if !rigged {
            continue;
        }
        let mut entity = world.entity_mut(rig);
        if let Some((layered, layering)) = layering::layered(model, &joints) {
            entity.insert((layered, layering));
        }
        if let Some(mut layering) = entity.get_mut::<StageLayering>() {
            layering.put = true;
        }
    }
    for (mut layered, layering, stage, props) in world
        .query::<(&mut Layered, &Layering, &StageLayering, Option<&StageProps>)>()
        .iter_mut(world)
    {
        let holding: Vec<HandIndex> = props
            .map_or(&[][..], |props| props.props.as_slice())
            .iter()
            .map(|prop| prop.hand)
            .collect();
        layering::drive(
            &mut layered,
            layering,
            stage.asked.action,
            &holding,
            stage.asked.stepped,
        );
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
