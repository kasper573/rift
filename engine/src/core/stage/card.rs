//! Cards on the stage. A card paints in the interface as ever; its triangles are then taken off
//! its layer and made a mesh on the stage, lit by its lamp and turned toward the pointer, and the
//! interface shows the lit picture where the card was painted.

use std::collections::{HashMap, HashSet};
use std::mem::Discriminant;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path};
use bevy::camera::SubCameraView;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, UvChannel};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPostUpdateSet;
use ui::card::CardTilt;
use ui::egui::{self, LayerId, Pos2, Rect};
use ui::motion::{CardPose, LiftedRun};

use super::{
    DUE_AFTER_A_CHANGE, Lamp, Lens, Picture, PictureLook, Scene, on_stage, rendered, view_about,
};
use crate::core::card::{CardHandling, CardMaterial};
use crate::core::content::ImageRef;
use crate::core::gui;

/// The lamp a card on the stage is lit by, as it hangs this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HungCardLamp {
    /// Where over the window it hangs, as shares of the window's width and height from its top
    /// left corner, and how far in front of the window, in window heights.
    pub across: f32,
    pub down: f32,
    pub height: f32,
    pub color: Color,
    /// How much of a card's light is the lamp's.
    pub strength: f32,
}

/// A card as it is put on the stage this pass.
#[derive(Clone, Debug)]
pub struct StagedCard {
    /// What tells the card from every other on the stage.
    pub key: egui::Id,
    /// The card's paint, taken off the layer it painted on.
    pub runs: Vec<LiftedRun>,
    /// Where the card lies when it is not turned.
    pub flat: Rect,
    /// Where the card is on its way to or from there, when it is.
    pub pose: Option<CardPose>,
    /// Whether the card turns toward the pointer. One that doesn't lies flat.
    pub turns: bool,
    pub material: CardMaterial,
}

/// What every card's mesh is drawn with: Bevy's own surface, and on top of it what that lacks.
pub type CardSurface = ExtendedMaterial<StandardMaterial, CardPrint>;

/// What a card's surface is beyond Bevy's: the interface's own colours as it blends them, the
/// clip rectangle its triangles were painted under, and a foil's colours.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct CardPrint {
    #[texture(100)]
    #[sampler(101)]
    print: Handle<Image>,
    #[uniform(102)]
    light: PrintLight,
    #[texture(103)]
    #[sampler(104)]
    foil: Handle<Image>,
}

#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
struct PrintLight {
    /// The clip rectangle, in the card's own measure: its corners as the second uv gives them.
    clip: Vec4,
    lamp: Vec3,
    /// The share of a card's light that falls on it evenly.
    even: f32,
    /// How much of the print a foil's colours cover.
    foil: f32,
}

impl MaterialExtension for CardPrint {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("print.wesl")).with_source("embedded"),
        )
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

pub(super) struct CardStagePlugin;

impl Plugin for CardStagePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "print.wesl");
        app.add_plugins(MaterialPlugin::<CardSurface>::default())
            .init_resource::<CardStage>()
            .add_systems(
                PostUpdate,
                strike
                    .after(EguiPostUpdateSet::EndPass)
                    .before(TransformSystems::Propagate),
            );
    }
}

/// Puts a card's triangles on the stage, and paints its lit picture on `onto`, where the card
/// belongs among the interface's layers.
///
/// The renderer takes a few frames to draw a card it has not drawn before. Until it has, a
/// card at rest is painted as the interface painted it, and one on its way is not painted at
/// all: `false` then, for whoever moves it to wait with the move.
pub fn stage(world: &mut World, gui: &egui::Context, card: StagedCard, onto: LayerId) -> bool {
    let runs = card.runs;
    if runs.is_empty() || !card.flat.is_positive() {
        return true;
    }
    let handling = *world.resource::<CardHandling>();
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .map(|window| window.resolution.physical_size());
    let Ok(window) = window else {
        return true;
    };
    let screen = gui.content_rect();
    let pixels = gui.pixels_per_point();
    let frame = world.resource::<Time>().delta_secs();
    let resting = card.pose.is_none();
    let pose = card.pose.unwrap_or(CardPose {
        center: card.flat.center(),
        scale: 1.0,
        angle: 0.0,
        opacity: 1.0,
    });
    let pointer = gui
        .pointer_latest_pos()
        .filter(|at| card.turns && resting && gui.layer_id_at(*at) == Some(onto));
    let shows = runs
        .iter()
        .map(|run| run.mesh.calc_bounds().intersect(run.clip));
    let side = reach_of(shows, card.flat).length();
    let side = 2.0 * side * pose.scale.max(1.0) * SLACK * pixels;
    let side = ((side / GRAIN).ceil() * GRAIN).max(GRAIN) as u32;
    let (view, corner) = view_about(pose.center, side, pixels, window);
    let scene = Scene {
        eye: handling.eye * screen.height(),
        lamp: Lamp::over(world, &handling.lamp, screen),
        height: screen.height(),
    };

    let mut staged = world
        .resource_mut::<CardStage>()
        .cards
        .remove(&card.key)
        .unwrap_or_default();
    staged.kept = true;
    staged.struck = None;
    staged.tilt = staged.tilt.toward(
        pointer,
        card.flat,
        handling.tilt.0,
        handling.settles_in.0,
        frame,
    );
    let layer = staged.picture.layer(world);
    let placed = Transform {
        translation: on_stage(pose.center, screen),
        rotation: Quat::from_rotation_z(-pose.angle)
            * Quat::from_rotation_y(staged.tilt.aside.to_radians())
            * Quat::from_rotation_x(staged.tilt.above.to_radians()),
        scale: Vec3::splat(pose.scale),
    };
    let shot = Shot {
        view,
        scene,
        screen,
        placed,
    };
    let reframed = staged.shot != Some(shot);
    if reframed {
        staged.picture.frame(world, &view, &scene, PICTURE_LOOK);
        staged.shot = Some(shot);
    }
    let laid = Laid {
        flat: card.flat,
        reach: reach_of(runs.iter().map(|run| run.mesh.calc_bounds()), card.flat),
        pixels,
        placed,
        moved: reframed,
        layer,
        look: SurfaceLook::of(world, &card.material, &scene.lamp),
        lamp: scene.lamp.at,
    };
    let relaid = lay_runs(world, &mut staged, runs, &laid);

    let was_drawn = staged.drawn;
    staged.drawn = staged.drawn || drawn(world, &mut staged, &card.material, &laid.look);
    staged.due = if reframed || relaid || staged.drawn != was_drawn {
        DUE_AFTER_A_CHANGE
    } else {
        staged.due.saturating_sub(1)
    };
    staged.picture.take(world, staged.due > 0 || !staged.drawn);
    let shown = match staged.picture.texture() {
        Some(texture) if staged.drawn => {
            let lies = Rect::from_min_size(
                Pos2::new(corner.x, corner.y) / pixels,
                egui::Vec2::splat(side as f32 / pixels),
            );
            gui.layer_painter(onto).add(egui::Shape::image(
                texture,
                lies,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                egui::Color32::WHITE.gamma_multiply(pose.opacity),
            ));
            true
        }
        _ if resting => {
            gui.graphics_mut(|graphics| {
                let painted = graphics.entry(onto);
                for run in &staged.runs {
                    painted.add(run.lifted.clip, egui::Shape::mesh(run.lifted.mesh.clone()));
                }
            });
            true
        }
        _ => false,
    };
    world
        .resource_mut::<CardStage>()
        .cards
        .insert(card.key, staged);
    shown
}

/// The lamp the card of `key` is lit by, while it is on the stage.
pub fn lamp_of(world: &World, key: egui::Id) -> Option<HungCardLamp> {
    let shot = world.get_resource::<CardStage>()?.cards.get(&key)?.shot?;
    let (lamp, screen) = (shot.scene.lamp, shot.screen);
    Some(HungCardLamp {
        across: 0.5 + lamp.at.x / screen.width(),
        down: 0.5 - lamp.at.y / screen.height(),
        height: lamp.at.z / screen.height(),
        color: lamp.color,
        strength: lamp.share * lamp.shines,
    })
}

/// How far the card of `key` is turned toward the pointer.
pub fn tilt_of(world: &World, key: egui::Id) -> CardTilt {
    world
        .get_resource::<CardStage>()
        .and_then(|stage| stage.cards.get(&key))
        .map_or(CardTilt::FLAT, |staged| staged.tilt)
}

/// The cards on the stage, each by its key.
#[derive(Resource, Default)]
struct CardStage {
    cards: HashMap<egui::Id, Staged>,
    reliefs: HashMap<String, Handle<Image>>,
    /// The kinds of material the renderer has drawn a card in. A card of a kind it has yet to
    /// draw waits for the renderer to be ready for it.
    drawn: HashSet<Discriminant<CardMaterial>>,
}

/// A card on the stage: its picture and the camera that draws it, its lamp, and its triangles
/// run by run.
#[derive(Default)]
struct Staged {
    kept: bool,
    /// When the card was last put on the stage, once it no longer is.
    struck: Option<f64>,
    /// How many frames the renderer had drawn when everything the card is drawn with was first
    /// at hand, and whether it has drawn the card since.
    at_hand: Option<u64>,
    drawn: bool,
    /// What the card's picture was last taken of, and how many more passes it is taken anew: a
    /// card that lies as it lay, painted as it was, keeps the picture it has.
    shot: Option<Shot>,
    due: u8,
    tilt: CardTilt,
    picture: Picture,
    runs: Vec<StagedRun>,
}

struct StagedRun {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<CardSurface>,
    lifted: LiftedRun,
    flat: Rect,
    reach: egui::Vec2,
    print: Handle<Image>,
    look: SurfaceLook,
    lamp: Vec3,
}

/// Everything a card's picture is taken of but the card's own paint: the part of the window it
/// covers, the scene with its lamp, the window in the interface's points, and where the card
/// lies in it.
#[derive(Clone, Copy, PartialEq)]
struct Shot {
    view: SubCameraView,
    scene: Scene,
    screen: Rect,
    placed: Transform,
}

/// Where a card's runs of triangles are laid this pass, and what they are drawn with.
struct Laid {
    flat: Rect,
    /// How far from the middle of `flat` the card's triangles reach, aside and above.
    reach: egui::Vec2,
    pixels: f32,
    placed: Transform,
    /// Whether that is not where it lay last pass.
    moved: bool,
    layer: usize,
    look: SurfaceLook,
    lamp: Vec3,
}

/// A material as the stage's surface takes it.
#[derive(Clone, Debug, PartialEq)]
struct SurfaceLook {
    roughness: f32,
    reflectance: f32,
    metallic: f32,
    clearcoat: f32,
    clearcoat_roughness: f32,
    relief: Option<Handle<Image>>,
    foil: Option<(Handle<Image>, f32)>,
    even: f32,
}

impl SurfaceLook {
    fn of(world: &mut World, material: &CardMaterial, lamp: &Lamp) -> SurfaceLook {
        let even = lamp.even();
        let plain = SurfaceLook {
            roughness: 1.0,
            reflectance: 0.0,
            metallic: 0.0,
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            relief: None,
            foil: None,
            even,
        };
        match material {
            CardMaterial::Matte => plain,
            CardMaterial::Satin { gloss } => SurfaceLook {
                roughness: lerp(SATIN_ROUGH, *gloss),
                reflectance: SHEEN,
                ..plain
            },
            CardMaterial::Lacquer { gloss } => SurfaceLook {
                clearcoat: 1.0,
                clearcoat_roughness: lerp(COAT_ROUGH, *gloss),
                ..plain
            },
            CardMaterial::Metal { polish } => SurfaceLook {
                roughness: lerp(METAL_ROUGH, *polish),
                metallic: 1.0,
                even: even * METAL_EVEN,
                ..plain
            },
            CardMaterial::Foil { pattern, strength } => SurfaceLook {
                roughness: FOIL_ROUGH,
                reflectance: SHEEN,
                foil: Some((measure(world, pattern), *strength * lamp.shines)),
                ..plain
            },
            CardMaterial::Embossed { relief, gloss } => SurfaceLook {
                roughness: lerp(RELIEF_ROUGH, *gloss),
                reflectance: SHEEN,
                relief: Some(measure(world, relief)),
                ..plain
            },
        }
    }

    fn surface(&self, print: Handle<Image>, clip: Vec4, lamp: Vec3, over: usize) -> CardSurface {
        ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: self.roughness,
                reflectance: self.reflectance,
                metallic: self.metallic,
                clearcoat: self.clearcoat,
                clearcoat_perceptual_roughness: self.clearcoat_roughness,
                normal_map_texture: self.relief.clone(),
                normal_map_channel: UvChannel::Uv1,
                alpha_mode: AlphaMode::Premultiplied,
                double_sided: false,
                cull_mode: None,
                fog_enabled: false,
                depth_bias: over as f32 * OVER,
                ..default()
            },
            extension: CardPrint {
                print,
                light: PrintLight {
                    clip,
                    lamp,
                    even: self.even,
                    foil: self.foil.as_ref().map_or(0.0, |(_, strength)| *strength),
                },
                foil: self
                    .foil
                    .as_ref()
                    .map_or_else(Handle::default, |(pattern, _)| pattern.clone()),
            },
        }
    }
}

/// A satin's, a lacquer coat's, a metal's and a relief's roughness, from no gloss to full.
const SATIN_ROUGH: (f32, f32) = (0.75, 0.4);
const COAT_ROUGH: (f32, f32) = (0.4, 0.05);
const METAL_ROUGH: (f32, f32) = (0.6, 0.15);
const RELIEF_ROUGH: (f32, f32) = (0.8, 0.35);
const FOIL_ROUGH: f32 = 0.45;
/// How much light a surface with a sheen reflects head-on, as Bevy's surfaces measure it.
const SHEEN: f32 = 0.5;
/// The share of the even light a metal returns: away from its lamp it falls dark.
const METAL_EVEN: f32 = 0.6;
/// How far apart a card's runs of triangles are sorted, so that each is drawn over the one
/// painted before it. Bevy builds a pipeline for every whole step of this, so the runs of a card
/// share less than one between them, and one pipeline.
const OVER: f32 = 1.0 / 1024.0;
/// How much farther than its flat reach a turned card may be drawn, by its near edge growing.
const SLACK: f32 = 1.1;
/// The step a card's picture grows by, in pixels, so it keeps its size as the card's own
/// changes a little.
const GRAIN: f32 = 32.0;
/// How many seconds a card's picture is kept once the card is off the stage.
const LINGER: f64 = 1.0;
/// A card's own shader spreads its even light, and its edges are the interface's, smoothed
/// already.
const PICTURE_LOOK: PictureLook = PictureLook {
    lens: Lens::FromTheEye,
    smoothing: Msaa::Off,
    even: 0.0,
};

fn lerp((from, to): (f32, f32), share: f32) -> f32 {
    from + (to - from) * share.clamp(0.0, 1.0)
}

/// The image of a relief or a foil's pattern read as numbers, not colours, and tiling.
fn measure(world: &mut World, image: &ImageRef) -> Handle<Image> {
    let path = image.path();
    if let Some(kept) = world.resource::<CardStage>().reliefs.get(&path) {
        return kept.clone();
    }
    let read = world
        .resource::<AssetServer>()
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.is_srgb = false;
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load(path.clone());
    world
        .resource_mut::<CardStage>()
        .reliefs
        .insert(path, read.clone());
    read
}

/// How far from the middle of `flat` a card reaches, aside and above, that lies there and is
/// painted in `painted` besides.
fn reach_of(painted: impl Iterator<Item = Rect>, flat: Rect) -> egui::Vec2 {
    let middle = flat.center();
    painted
        .chain([flat])
        .flat_map(|drawn| [drawn.min, drawn.max])
        .filter(|corner| corner.is_finite())
        .map(|corner| (corner - middle).abs())
        .fold(egui::Vec2::ZERO, egui::Vec2::max)
}

/// Whether the renderer draws the card: its images are loaded, and the renderer has drawn a
/// card of its kind of material before, or has had the frames a change takes to reach the
/// screen and is compiling no pipeline.
fn drawn(
    world: &mut World,
    staged: &mut Staged,
    material: &CardMaterial,
    look: &SurfaceLook,
) -> bool {
    let kind = std::mem::discriminant(material);
    let assets = world.resource::<AssetServer>();
    let at_hand = [
        &look.relief,
        &look.foil.as_ref().map(|(pattern, _)| pattern.clone()),
    ]
    .into_iter()
    .flatten()
    .all(|image| assets.is_loaded_with_dependencies(image));
    if !at_hand {
        staged.at_hand = None;
        return false;
    }
    if world.resource::<CardStage>().drawn.contains(&kind) {
        return true;
    }
    let drawn = rendered(world, &mut staged.at_hand);
    if drawn {
        world.resource_mut::<CardStage>().drawn.insert(kind);
    }
    drawn
}

/// Makes the card's runs of triangles its meshes, each drawn over the one before it, and lays
/// them where the card is. A run painted as it was last pass keeps its mesh. Whether any run is
/// drawn otherwise than it was.
fn lay_runs(world: &mut World, staged: &mut Staged, runs: Vec<LiftedRun>, laid: &Laid) -> bool {
    let mut relaid = false;
    for gone in staged.runs.drain(runs.len().min(staged.runs.len())..) {
        world.despawn(gone.entity);
        relaid = true;
    }
    for (over, lifted) in runs.into_iter().enumerate() {
        let Some(print) = gui::texture_image(world, lifted.mesh.texture_id) else {
            continue;
        };
        let surface = |lifted: &LiftedRun| {
            laid.look.surface(
                print.clone(),
                clip_of(lifted.clip, laid.flat, laid.pixels),
                laid.lamp,
                over,
            )
        };
        match staged.runs.get_mut(over) {
            Some(run) => {
                let moved = run.flat != laid.flat || run.reach != laid.reach;
                if moved || run.lifted != lifted {
                    let mesh = mesh_of(&lifted, laid.flat, laid.reach);
                    world
                        .resource_mut::<Assets<Mesh>>()
                        .insert(run.mesh.id(), mesh)
                        .ok();
                    relaid = true;
                }
                if moved
                    || run.lifted.clip != lifted.clip
                    || run.print != print
                    || run.look != laid.look
                    || run.lamp != laid.lamp
                {
                    world
                        .resource_mut::<Assets<CardSurface>>()
                        .insert(run.material.id(), surface(&lifted))
                        .ok();
                    relaid = true;
                }
                run.lifted = lifted;
                run.flat = laid.flat;
                run.reach = laid.reach;
                run.print = print;
                run.look = laid.look.clone();
                run.lamp = laid.lamp;
                if laid.moved {
                    world
                        .entity_mut(run.entity)
                        .insert((laid.placed, GlobalTransform::from(laid.placed)));
                }
            }
            None => {
                let mesh = world
                    .resource_mut::<Assets<Mesh>>()
                    .add(mesh_of(&lifted, laid.flat, laid.reach));
                let material = world
                    .resource_mut::<Assets<CardSurface>>()
                    .add(surface(&lifted));
                let entity = world
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        laid.placed,
                        GlobalTransform::from(laid.placed),
                        RenderLayers::layer(laid.layer),
                        NoFrustumCulling,
                        NotShadowCaster,
                    ))
                    .id();
                staged.runs.push(StagedRun {
                    entity,
                    mesh,
                    material,
                    lifted,
                    flat: laid.flat,
                    reach: laid.reach,
                    print,
                    look: laid.look.clone(),
                    lamp: laid.lamp,
                });
                relaid = true;
            }
        }
    }
    relaid
}

/// A run's triangles as a mesh about the middle of `flat`, facing the eye. Its first uv is the
/// interface's, into the run's texture; its second measures the card from its top left corner in
/// its own widths; its colours are the interface's as they are, for the surface to blend the way
/// the interface does.
///
/// Bevy draws see-through meshes in the order of how far off the middle of each one's bounds
/// is, so every run of a card is given the bounds of the whole card, `reach` to each side of its
/// middle, by two corners no triangle uses: turned, the runs stay in the order they were
/// painted in, whatever part of the card each covers.
fn mesh_of(run: &LiftedRun, flat: Rect, reach: egui::Vec2) -> Mesh {
    let middle = flat.center();
    let corners = [middle - reach, middle + reach].map(|pos| egui::epaint::Vertex {
        pos,
        uv: egui::epaint::WHITE_UV,
        color: egui::Color32::TRANSPARENT,
    });
    let vertices: Vec<egui::epaint::Vertex> =
        run.mesh.vertices.iter().copied().chain(corners).collect();
    let positions: Vec<[f32; 3]> = vertices
        .iter()
        .map(|vertex| [vertex.pos.x - middle.x, middle.y - vertex.pos.y, 0.0])
        .collect();
    let uvs: Vec<[f32; 2]> = vertices
        .iter()
        .map(|vertex| [vertex.uv.x, vertex.uv.y])
        .collect();
    let measures: Vec<[f32; 2]> = vertices
        .iter()
        .map(|vertex| measured(vertex.pos, flat).to_array())
        .collect();
    let colors: Vec<[f32; 4]> = vertices
        .iter()
        .map(|vertex| vertex.color.to_array().map(|part| f32::from(part) / 255.0))
        .collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 0.0, 1.0]; vertices.len()],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_TANGENT,
        vec![[1.0, 0.0, 0.0, 1.0]; vertices.len()],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, measures)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(run.mesh.indices.clone()))
}

/// A point of the interface in the card's own measure.
fn measured(at: Pos2, flat: Rect) -> Vec2 {
    let from = (at - flat.min) / flat.width();
    Vec2::new(from.x, from.y)
}

/// A clip rectangle in the card's own measure, on the whole pixels the interface cuts at.
fn clip_of(clip: Rect, flat: Rect, pixels: f32) -> Vec4 {
    let whole = |at: Pos2| Pos2::new((at.x * pixels).round(), (at.y * pixels).round()) / pixels;
    let (min, max) = (
        measured(whole(clip.min), flat),
        measured(whole(clip.max), flat),
    );
    Vec4::new(min.x, min.y, max.x, max.y)
}

/// Takes every card off the stage that was not put on it this pass. Its picture is kept a while
/// longer, as it last was, for whatever of the interface still shows it: a sheet that closes
/// shows what it last held on its way out.
fn strike(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let mut gone: Vec<Entity> = Vec::new();
    let mut struck: Vec<egui::Id> = Vec::new();
    let mut forgotten: Vec<egui::Id> = Vec::new();
    {
        let mut stage = world.resource_mut::<CardStage>();
        for (key, staged) in &mut stage.cards {
            if !std::mem::take(&mut staged.kept) && staged.struck.is_none() {
                staged.struck = Some(now);
                gone.extend(staged.runs.drain(..).map(|run| run.entity));
                staged.shot = None;
                struck.push(*key);
            }
            if staged.struck.is_some_and(|at| now - at >= LINGER) {
                forgotten.push(*key);
            }
        }
    }
    for entity in gone {
        world.despawn(entity);
    }
    world.resource_scope(|world, mut stage: Mut<CardStage>| {
        for key in struck {
            if let Some(staged) = stage.cards.get_mut(&key) {
                staged.picture.strike(world);
            }
        }
        for key in forgotten {
            if let Some(mut staged) = stage.cards.remove(&key) {
                staged.picture.forget(world);
            }
        }
    });
}
