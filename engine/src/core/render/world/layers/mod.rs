//! Motion drawn over a body's clips where keyframes are not enough: feet planted where they touched
//! the ground, chains springing or trailing behind, a wave down a spine, a joint squashed by speed
//! and landings, joints held in a pose of their own. Layers run after the animation graph and before transforms propagate, and write
//! joint transforms only. Nothing the game reasons about comes from them (a body's place, its
//! reach, the moment a hit lands), so they need not agree between clients and keep no seed.

mod chain;
mod feet;
mod held_pose;
mod squash;
mod wave;

pub use chain::Chain;
pub use feet::{Feet, Gait};
pub use held_pose::HeldPose;
pub use squash::Squash;
pub use wave::Wave;

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;
use bevy_skeleton_modifiers::core::{BonePose, SkeletonView};

use super::super::camera::{GroundHeights, WorldCamera, ground_pos};

/// Within this distance of the camera a body's layers run every frame, and up to `FAR` every other
/// frame; beyond it the body plays its clips alone.
const NEAR: f32 = 15.0;
const FAR: f32 = 30.0;
/// How far around its place a body is looked for in the camera's view.
const BODY_RADIUS: f32 = 2.0;

/// The joints of a body's rig as its scene spawned them, by their index in the rig: each one's
/// entity, its parent among them, and how it rests.
pub struct Rig {
    pub joints: Vec<Entity>,
    pub parents: Vec<Option<u16>>,
    pub rest: Vec<Transform>,
}

impl Rig {
    /// The joints under `from`, from it down, each the first child of the one before.
    pub fn line(&self, from: u16) -> Vec<u16> {
        let mut line = vec![from];
        while let Some(next) = (0..self.parents.len() as u16)
            .find(|&joint| self.parents[joint as usize] == line.last().copied())
        {
            line.push(next);
        }
        line
    }

    fn rest_poses(&self) -> Vec<BonePose> {
        self.rest.iter().map(|rest| pose(*rest)).collect()
    }
}

/// Runs every body's layers after its clips play and before transforms propagate.
pub struct JointLayersPlugin;

impl Plugin for JointLayersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            run.after(bevy::app::AnimationSystems)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// One layer over a body's clips.
pub enum JointLayer {
    Feet(Feet),
    Chain(Chain),
    Wave(Wave),
    Squash(Squash),
    HeldPose(HeldPose),
}

impl JointLayer {
    fn run(&mut self, frame: &mut Frame) {
        match self {
            JointLayer::Feet(feet) => feet.run(frame),
            JointLayer::Chain(chain) => chain.run(frame),
            JointLayer::Wave(wave) => wave.run(frame),
            JointLayer::Squash(squash) => squash.run(frame),
            JointLayer::HeldPose(held) => held.run(frame),
        }
    }

    fn reset(&mut self) {
        match self {
            JointLayer::Feet(feet) => feet.reset(),
            JointLayer::Chain(chain) => chain.reset(),
            JointLayer::Wave(_) => {}
            JointLayer::Squash(squash) => squash.reset(),
            JointLayer::HeldPose(held) => held.reset(),
        }
    }
}

/// A body whose joints its layers move after its clips play, in the order given. The game says
/// each frame which layers the clip playing lets run and whether a foot came down.
#[derive(Component)]
pub struct Layered {
    rig: Rig,
    layers: Vec<JointLayer>,
    pub on: Vec<bool>,
    pub stepped: bool,
    /// What each joint's clip gave it and what the layers made of it, the last time they wrote it.
    written: Vec<Option<(BonePose, BonePose)>>,
    last: Option<(f32, glam::Vec3)>,
    skipped: bool,
}

impl Layered {
    pub fn new(rig: Rig, layers: Vec<JointLayer>) -> Layered {
        Layered {
            written: vec![None; rig.joints.len()],
            on: vec![false; layers.len()],
            rig,
            layers,
            stepped: false,
            last: None,
            skipped: false,
        }
    }

    /// Whether any layer is moving joints.
    pub fn moving(&self) -> bool {
        self.written.iter().any(Option::is_some)
    }
}

/// What a layer reads and moves as it runs: the rig's poses and where it stands, the time and
/// ground, and how the body moves.
pub(super) struct Frame<'a> {
    pub view: SkeletonView<'a>,
    pub dt: f32,
    pub now: f32,
    pub ground: &'a dyn Fn(glam::Vec3) -> f32,
    /// How fast the body goes across the ground, in metres a second.
    pub speed: f32,
    pub stepped: bool,
}

/// Runs every body's layers over the pose its clips gave it this frame.
#[allow(clippy::type_complexity)]
fn run(
    time: Res<Time>,
    ground: Option<Res<GroundHeights>>,
    cameras: Query<(&GlobalTransform, &Frustum), With<WorldCamera>>,
    mut bodies: Query<&mut Layered>,
    mut transforms: Query<(&mut Transform, Option<&ChildOf>)>,
) {
    let now = time.elapsed_secs();
    let camera = cameras.iter().next();
    let height = |at: glam::Vec3| {
        ground.as_ref().map_or(0.0, |ground| {
            ground.height_at(ground_pos(Vec3::new(at.x, at.y, at.z))).0
        })
    };
    for mut body in &mut bodies {
        let body = &mut *body;
        let Some(root_global) = body
            .rig
            .joints
            .first()
            .and_then(|&root| transforms.get(root).ok())
            .and_then(|(_, parent)| parent.map(ChildOf::parent))
            .map(|parent| global_of(&transforms, parent))
        else {
            continue;
        };
        let mut local: Vec<BonePose> = Vec::with_capacity(body.rig.joints.len());
        for (joint, written) in body.rig.joints.iter().zip(&body.written) {
            let Ok((transform, _)) = transforms.get(*joint) else {
                break;
            };
            local.push(given(pose(*transform), *written));
        }
        if local.len() != body.rig.joints.len() {
            continue;
        }
        let at = glam::Vec3::from_array(root_global.translation.to_array());
        let seen = camera.is_none_or(|(_, frustum)| {
            frustum.intersects_sphere(
                &Sphere {
                    center: root_global.translation.into(),
                    radius: BODY_RADIUS,
                },
                true,
            )
        });
        let distance = camera.map_or(0.0, |(eye, _)| {
            eye.translation().distance(Vec3::new(at.x, at.y, at.z))
        });
        let stepped = std::mem::take(&mut body.stepped);
        if !seen || distance > FAR || !body.on.iter().any(|on| *on) {
            for layer in &mut body.layers {
                layer.reset();
            }
            body.last = None;
            write(
                &mut transforms,
                &body.rig,
                &local,
                &local,
                &mut body.written,
            );
            continue;
        }
        if distance > NEAR && !body.skipped {
            body.skipped = true;
            let held: Vec<BonePose> = local
                .iter()
                .zip(&body.written)
                .map(|(given, written)| written.map_or(*given, |(_, shown)| shown))
                .collect();
            write(&mut transforms, &body.rig, &local, &held, &mut body.written);
            continue;
        }
        body.skipped = false;
        let (dt, speed) = match body.last {
            Some((then, was)) if now > then => {
                let gone = glam::Vec2::new(at.x - was.x, at.z - was.z).length();
                (now - then, gone / (now - then))
            }
            _ => (time.delta_secs(), 0.0),
        };
        body.last = Some((now, at));
        let input = local.clone();
        let rest = body.rig.rest_poses();
        let root = affine(root_global);
        for (layer, on) in body.layers.iter_mut().zip(&body.on) {
            if !*on {
                layer.reset();
                continue;
            }
            let mut frame = Frame {
                view: SkeletonView {
                    parents: &body.rig.parents,
                    rest: &rest,
                    local: &mut local,
                    root_global: root,
                },
                dt,
                now,
                ground: &height,
                speed,
                stepped,
            };
            layer.run(&mut frame);
        }
        write(
            &mut transforms,
            &body.rig,
            &input,
            &local,
            &mut body.written,
        );
    }
}

/// Writes the layered pose `shown` to the rig's joints, and what the clip gave each, so a field the
/// clip does not key comes back to the clip's next frame rather than to the layer's last.
fn write(
    transforms: &mut Query<(&mut Transform, Option<&ChildOf>)>,
    rig: &Rig,
    given: &[BonePose],
    shown: &[BonePose],
    written: &mut [Option<(BonePose, BonePose)>],
) {
    for (index, joint) in rig.joints.iter().enumerate() {
        let moved = shown[index] != given[index];
        if !moved && written[index].is_none() {
            continue;
        }
        if let Ok((mut transform, _)) = transforms.get_mut(*joint) {
            *transform = transform_of(shown[index]);
        }
        written[index] = moved.then_some((given[index], shown[index]));
    }
}

/// The pose a clip gave a joint this frame: a field still as the layers left it is one the clip
/// does not key, so it is what the clip gave it before.
fn given(now: BonePose, written: Option<(BonePose, BonePose)>) -> BonePose {
    let Some((given, shown)) = written else {
        return now;
    };
    BonePose {
        translation: if now.translation == shown.translation {
            given.translation
        } else {
            now.translation
        },
        rotation: if now.rotation == shown.rotation {
            given.rotation
        } else {
            now.rotation
        },
        scale: if now.scale == shown.scale {
            given.scale
        } else {
            now.scale
        },
    }
}

/// Where `entity` stands in the world as its own and its ancestors' transforms stand now, before
/// they propagate.
fn global_of(transforms: &Query<(&mut Transform, Option<&ChildOf>)>, entity: Entity) -> Transform {
    let mut global = Transform::IDENTITY;
    let mut at = Some(entity);
    while let Some(node) = at {
        let Ok((transform, parent)) = transforms.get(node) else {
            break;
        };
        global = transform.mul_transform(global);
        at = parent.map(ChildOf::parent);
    }
    global
}

fn pose(transform: Transform) -> BonePose {
    BonePose {
        translation: glam::Vec3::from_array(transform.translation.to_array()),
        rotation: glam::Quat::from_array(transform.rotation.to_array()),
        scale: glam::Vec3::from_array(transform.scale.to_array()),
    }
}

fn transform_of(pose: BonePose) -> Transform {
    Transform {
        translation: Vec3::from_array(pose.translation.to_array()),
        rotation: Quat::from_array(pose.rotation.to_array()),
        scale: Vec3::from_array(pose.scale.to_array()),
    }
}

fn affine(transform: Transform) -> glam::Affine3A {
    glam::Affine3A::from_scale_rotation_translation(
        glam::Vec3::from_array(transform.scale.to_array()),
        glam::Quat::from_array(transform.rotation.to_array()),
        glam::Vec3::from_array(transform.translation.to_array()),
    )
}
