//! Shot: a picture of a game's own area with its models standing in it, drawn off screen by the
//! world's own camera and lights, to compare how the game looks against a reference picture.
//!
//! `shot <game folder> <shot.json> <out.png>`. The shot names its area, the lens the picture is
//! taken through, its eye and target set above the ground beneath them, and each body standing on
//! the ground in it: its model, where it stands, the spot it faces, the items in its hands, and the
//! moment it holds of its stance's idle or of another clip its skeleton plays.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::window::ExitCondition;
use engine::core::content::{Content, FilesystemFolder};
use engine::core::geometry::{Degrees, Radians};
use engine::core::ground::Heightfield;
use engine::core::math::Pos;
use engine::core::render::camera::{self, world_point};
use engine::core::render::world::WorldScenePlugin;
use engine::core::render::world::body;
use engine::core::render::world::layers::Layered;
use engine::core::render::world::mannequin::mannequin;
use engine::core::render::world::rig_clip;
use engine::core::render::world::sky::Daylight;
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::systems::actor::layering::{self, Layering};
use engine::systems::actor::model::{ActorModel, ModelId};
use engine::systems::actor::skeleton::Clip;
use engine::systems::actor::stance::{self, HandIndex};
use engine::systems::actor::{Action, Appearance, ModelDef, Motion, Pose, appearance, render};
use engine::systems::area::scenery;
use engine::systems::area::{self, AreaDef, Kept, Pictured};
use engine::systems::item::{ItemDef, Wear};
use serde::Deserialize;

/// Frames the picture settles for once every body stands dressed, while what it shows loads and
/// the frames it is smoothed over fill.
const SETTLE: u32 = 180;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shot {
    area: String,
    /// A multiple of 64, so each row read back fills whole blocks of 256 bytes and comes unpadded.
    width: u32,
    height: u32,
    lens: Lens,
    bodies: Vec<Standing>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct Lens {
    eye: Spot,
    target: Spot,
    fov: Degrees,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Standing {
    model: String,
    at: Place,
    faces: Place,
    #[serde(default)]
    held: Vec<Option<String>>,
    #[serde(default)]
    clip: Option<Clip>,
    moment: Seconds,
}

/// A point `height` above the ground at `x`, `y`, so a lens frames alike on any area's terrain.
#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct Spot {
    x: Meters,
    y: Meters,
    height: Meters,
}

impl Spot {
    fn point(self, ground: &Heightfield) -> Vec3 {
        let at = Pos::new(self.x.0, self.y.0);
        world_point(at, Meters(ground.height_at(at).0 + self.height.0))
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct Place {
    x: Meters,
    y: Meters,
}

impl Place {
    fn ground(self) -> Pos<Meters> {
        Pos::new(self.x.0, self.y.0)
    }
}

#[derive(Resource)]
struct Taking {
    shot: Shot,
    out: PathBuf,
    image: Handle<Image>,
    stage: Stage,
}

enum Stage {
    Empty,
    Standing(Vec<Body>),
    Settling(u32),
    Reading(Entity),
}

struct Body {
    rig: Entity,
    model: ModelId,
    props: Vec<Prop>,
    hung: bool,
}

struct Prop {
    hand: HandIndex,
    joint: String,
    scene: String,
    transform: Transform,
}

#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, folder, shot, out] = args.as_slice() else {
        eprintln!("shot <game folder> <shot.json> <out.png>");
        std::process::exit(2);
    };
    let folder = std::fs::canonicalize(folder).expect("a game folder");
    let shot: Shot =
        serde_json::from_slice(&std::fs::read(shot).expect("a shot file")).expect("a shot");
    assert!(
        shot.width.is_multiple_of(64),
        "a shot's width is a multiple of 64"
    );
    let content = engine::systems::load_content(
        Arc::new(FilesystemFolder(folder.clone())),
        "the game did not load",
    )
    .unwrap_or_else(|report| panic!("{report}"));
    let out = PathBuf::from(out);
    let mut shot = Some(shot);
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: folder.display().to_string(),
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_plugins((WorldScenePlugin, VfxMaterialsPlugin))
        .insert_resource(content)
        .init_resource::<Read>()
        .add_observer(|read: On<ReadbackComplete>, mut taken: ResMut<Read>| {
            taken.0.push((read.entity, read.data.clone()));
        })
        .add_systems(Startup, move |world: &mut World| {
            let shot = shot.take().expect("startup runs once");
            let mut image = Image::new_target_texture(
                shot.width,
                shot.height,
                TextureFormat::Rgba8UnormSrgb,
                None,
            );
            image.sampler = ImageSampler::linear();
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let image = world.resource_mut::<Assets<Image>>().add(image);
            world.insert_resource(Taking {
                shot,
                out: out.clone(),
                image,
                stage: Stage::Empty,
            });
        })
        .add_systems(Update, step)
        .run();
}

fn step(world: &mut World) {
    let content = world.resource::<Content>().clone();
    world.resource_scope(|world, mut taking: Mut<Taking>| {
        let taking = &mut *taking;
        match &mut taking.stage {
            Stage::Empty => {
                if let Some(bodies) = set_up(world, &content, &taking.shot, &taking.image) {
                    taking.stage = Stage::Standing(bodies);
                }
            }
            Stage::Standing(bodies) => {
                let mut all = true;
                for standing in bodies.iter_mut() {
                    let model = loaded(&content, standing.model);
                    if !standing.hung {
                        standing.hung = hang(world, model, standing);
                    }
                    all &= standing.hung;
                }
                drive(world, bodies);
                if all {
                    taking.stage = Stage::Settling(0);
                }
            }
            Stage::Settling(frames) => {
                *frames += 1;
                if *frames >= SETTLE {
                    let readback = world
                        .spawn(ReadbackOnce::texture(taking.image.clone()))
                        .id();
                    taking.stage = Stage::Reading(readback);
                }
            }
            Stage::Reading(readback) => {
                let mut read = world.resource_mut::<Read>();
                let Some(index) = read.0.iter().position(|(entity, _)| entity == readback) else {
                    return;
                };
                let pixels = read.0.remove(index).1;
                save(pixels, taking.shot.width, taking.shot.height, &taking.out);
                println!("{}", taking.out.display());
                world.write_message(AppExit::Success);
            }
        }
    });
}

/// Stands the shot's area and its bodies and points the camera, once the area's cover shaders are
/// compiled.
fn set_up(
    world: &mut World,
    content: &Content,
    shot: &Shot,
    image: &Handle<Image>,
) -> Option<Vec<Body>> {
    let area_id = content
        .by_path::<AreaDef>(&shot.area)
        .unwrap_or_else(|| panic!("no area {}", shot.area));
    let picture = area::picture(content, Pictured::Area(area_id), Kept::All)
        .unwrap_or_else(|| panic!("{} does not picture", shot.area));
    if !scenery::shaders_ready(world, content, &picture.area) {
        return None;
    }
    let camera = world
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            camera::lens(shot.lens.fov, shot.width, shot.height),
            camera::picture(),
            Transform::from_translation(shot.lens.eye.point(&picture.area.heights))
                .looking_at(shot.lens.target.point(&picture.area.heights), Vec3::Y),
        ))
        .id();
    let stood = scenery::spawn(world, content, &picture.area, default());
    scenery::light(world, &picture.area, Daylight::Fixed, stood, camera);
    world.insert_resource(scenery::climate(&picture.area));
    Some(
        shot.bodies
            .iter()
            .map(|standing| stand(world, content, &picture.area.heights, standing))
            .collect(),
    )
}

fn loaded(content: &Content, model: ModelId) -> &ActorModel {
    ActorModel::loaded(content, model)
        .unwrap_or_else(|| panic!("{} did not build", model.path(content)))
}

/// Stands a body where the shot says, facing its spot, holding its items in its stance's idle.
fn stand(world: &mut World, content: &Content, ground: &Heightfield, standing: &Standing) -> Body {
    let model_id = content
        .by_path::<ModelDef>(&standing.model)
        .unwrap_or_else(|| panic!("no model {}", standing.model));
    let model = loaded(content, model_id);
    let held: BTreeMap<HandIndex, _> = standing
        .held
        .iter()
        .enumerate()
        .filter_map(|(hand, item)| {
            let item = item.as_ref()?;
            let id = content
                .by_path::<ItemDef>(item)
                .unwrap_or_else(|| panic!("no item {item}"));
            Some((HandIndex(hand as u8), id))
        })
        .collect();
    let stance = stance::of_hands(content, &held);
    let built = model.skeleton();
    let played = match &standing.clip {
        Some(clip) => built
            .played(clip)
            .unwrap_or_else(|| panic!("{} plays no clip {clip}", standing.model)),
        None => model.standing_clip(stance, &Motion::Action(Action::Idle), Pose::Idle),
    };
    let clip = &built.clips[played];
    let pairs = clip
        .source
        .map(|source| built.def.sources[source].joint_pairs());
    let made = rig_clip::rig_clip(
        world,
        built.rig.clone(),
        clip.clip.file().as_str(),
        clip.animation,
        rig_clip::maker(built.rig.clone(), pairs),
    );
    let mut mannequin = mannequin(world, made);
    mannequin.hold(Some(standing.moment));
    let figure = model.dressed(
        content,
        &appearance::look(content, &Appearance::of(model_id), &BTreeMap::new()),
    );
    let at = standing.at.ground();
    let faces = standing.faces.ground();
    let rig = world
        .spawn((
            Transform::from_translation(world_point(at, ground.height_at(at))).with_rotation(
                camera::facing_rotation(Radians((faces.y - at.y).atan2(faces.x - at.x))),
            ),
            Visibility::default(),
            mannequin,
        ))
        .id();
    body::dress(world, rig, &figure);
    let props = render::hung(content, model, &held)
        .into_iter()
        .filter_map(|(hand, item)| {
            let Some(Wear::Held { grip, .. }) = item.get(content).wear() else {
                return None;
            };
            let at = model.hands().get(hand.index())?;
            Some(Prop {
                hand,
                joint: at.joint.0.clone(),
                scene: item.get(content).look.get(content).model.path(),
                transform: render::held_at(model, at, grip),
            })
        })
        .collect();
    Body {
        rig,
        model: model_id,
        props,
        hung: false,
    }
}

/// Hangs the body's props under their joints and puts its layers on, once its scene has spawned
/// the joints and every part of it shows.
fn hang(world: &mut World, model: &ActorModel, standing: &Body) -> bool {
    let Some(joints) = world
        .get::<body::Body>(standing.rig)
        .map(|body| body.joints().clone())
    else {
        return false;
    };
    let rigged = model
        .skeleton()
        .rig
        .joints()
        .all(|joint| joints.contains_key(&model.skeleton().rig.nodes[joint].name));
    let dressed = world
        .get::<Children>(standing.rig)
        .into_iter()
        .flatten()
        .filter(|child| world.get::<WorldAssetRoot>(**child).is_some())
        .all(|skin| world.get::<Visibility>(*skin) != Some(&Visibility::Hidden));
    if !rigged || !dressed {
        return false;
    }
    for prop in &standing.props {
        let Some(&joint) = joints.get(&prop.joint) else {
            continue;
        };
        let scene = world
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(prop.scene.clone()));
        world.spawn((
            WorldAssetRoot(scene),
            prop.transform,
            Visibility::default(),
            ChildOf(joint),
        ));
    }
    if let Some((layered, layering)) = layering::layered(model, &joints) {
        world.entity_mut(standing.rig).insert((layered, layering));
    }
    true
}

/// Drives each body's layers as the game drives an idle body holding its props.
fn drive(world: &mut World, bodies: &[Body]) {
    for standing in bodies {
        let holding: Vec<HandIndex> = standing.props.iter().map(|prop| prop.hand).collect();
        if let Ok((mut layered, layering)) = world
            .query::<(&mut Layered, &Layering)>()
            .get_mut(world, standing.rig)
        {
            layering::drive(&mut layered, layering, Action::Idle, &holding, false);
        }
    }
}

fn save(pixels: Vec<u8>, width: u32, height: u32, path: &std::path::Path) {
    let picture = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );
    picture
        .try_into_dynamic()
        .expect("an rgba picture")
        .to_rgba8()
        .save(path)
        .expect("a written picture");
}
