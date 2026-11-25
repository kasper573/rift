//! A body drawn as skins over one rig: the rig's joints stand beneath the body, and every skinned
//! mesh of each skin's scene is bound to the rig's joints of the same names, so a clip played on the
//! rig moves every skin, however many files they come from.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::camera::primitives::Aabb;
use bevy::math::Affine3A;
use bevy::mesh::VertexAttributeValues;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldInstanceReady};

use super::material::WorldMaterial;
use super::paint::{Paint, Painter, Paints};
use super::rig_clip::{GltfClips, GltfClipsLoader, PendingClips, make_pending};
use crate::core::content::gltf::NodeTree;

/// What a body is drawn as: the joints of its rig at rest, and the skins bound to them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Figure {
    pub rig: Arc<NodeTree>,
    pub skins: Vec<Skin>,
}

/// A glTF file of skinned meshes, drawn in its own material or painted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Skin {
    pub mesh: String,
    pub paint: Option<Paint>,
}

/// A body standing under its entity: its rig, the joints its clips play on, the scenes of its
/// skins and the limbs they hang on the joints.
#[derive(Component)]
pub struct Body {
    figure: Figure,
    rig: Entity,
    skins: Vec<Entity>,
    joints: HashMap<String, Entity>,
    /// Where each joint's bone ends, in the joint's own space.
    tails: HashMap<Entity, Vec3>,
    worn: HashMap<Entity, Vec<Limb>>,
    limbs: Vec<Limb>,
}

/// Flesh a body's skins hang on one joint of its rig, as a bone with the skin round it: the bone
/// from `head` to `tail` in the joint's own space, and how far round it the skin lies at its head,
/// `girth`, and at its tail, `tail_girth`, in the same space. A joint another hangs on has the
/// rig's bone, from itself to that joint. Where the rig ends or branches a joint has none (a hull,
/// a head, a pelvis), and its flesh is told by bones laid end to end the way it is longest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limb {
    pub joint: Entity,
    pub head: Vec3,
    pub tail: Vec3,
    pub girth: f32,
    pub tail_girth: f32,
}

/// Triggered on a body once its rig stands, its joints known and its player ready to play.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct RigReady {
    pub entity: Entity,
}

/// A skin's scene, hidden until its file has loaded whole, its meshes are bound to the rig and
/// painted: at rest until then, it would stand apart from a rig already moving, and its textures
/// still loading would draw it bare.
#[derive(Component)]
pub(super) struct SkinOf {
    body: Entity,
    file: Handle<WorldAsset>,
    paint: Option<Paint>,
    spawned: bool,
}

impl Body {
    pub fn figure(&self) -> &Figure {
        &self.figure
    }

    /// The joint of the rig named `name`, once the rig stands.
    pub fn joint(&self, name: &str) -> Option<Entity> {
        self.joints.get(name).copied()
    }

    /// Every node of the rig by its name, once the rig stands.
    pub fn joints(&self) -> &HashMap<String, Entity> {
        &self.joints
    }

    /// Whether every skin of the body is drawn: loaded, bound to the rig and painted.
    pub fn drawn(&self) -> bool {
        self.skins.iter().all(|skin| self.worn.contains_key(skin))
    }

    /// Where everything the body's skins draw stands as they are posed now, as the corners of
    /// boxes in the world: one round all each joint moves of a skinned mesh, as the joint holds it,
    /// and one round each other mesh. Together they keep close to what the body draws, where a box
    /// round a whole skinned mesh would reach far past it.
    pub fn drawn_boxes(&self, world: &World) -> Vec<[Vec3; 8]> {
        let mut boxes = Vec::new();
        let mut left = self.skins.clone();
        while let Some(node) = left.pop() {
            match skinned_boxes(world, node) {
                Some(skinned) => boxes.extend(skinned),
                None => boxes.extend(mesh_box(world, node)),
            }
            if let Some(children) = world.get::<Children>(node) {
                left.extend(children.iter());
            }
        }
        boxes
    }

    /// The entity the rig's clips play on.
    pub fn player(&self) -> Option<Entity> {
        Some(self.rig)
    }

    /// The limbs of every skin bound to the rig: for each joint some skin hangs flesh on, the
    /// flesh of the skin that hangs the thickest there.
    pub fn limbs(&self) -> &[Limb] {
        &self.limbs
    }

    fn wear(&mut self, skin: Entity, limbs: Vec<Limb>) {
        self.worn.insert(skin, limbs);
        self.gather_limbs();
    }

    fn gather_limbs(&mut self) {
        let mut thickest: HashMap<Entity, (f32, Vec<Limb>)> = HashMap::new();
        for worn in self.worn.values() {
            let mut hung: HashMap<Entity, Vec<Limb>> = HashMap::new();
            for limb in worn {
                hung.entry(limb.joint).or_default().push(*limb);
            }
            for (joint, limbs) in hung {
                let girth = limbs
                    .iter()
                    .map(|limb| limb.girth.max(limb.tail_girth))
                    .fold(0.0, f32::max);
                if thickest.get(&joint).is_none_or(|(kept, _)| *kept < girth) {
                    thickest.insert(joint, (girth, limbs));
                }
            }
        }
        self.limbs = thickest
            .into_values()
            .flat_map(|(_, limbs)| limbs)
            .collect();
    }
}

/// Stands the body `figure` describes under `entity`, in place of any body it stood before: its
/// rig's joints at rest, ready to play, and its skins bound to them once they load.
pub fn dress(world: &mut World, entity: Entity, figure: &Figure) {
    let old: Vec<Entity> = world
        .get::<Body>(entity)
        .map(|body| {
            std::iter::once(body.rig)
                .chain(body.skins.iter().copied())
                .collect()
        })
        .unwrap_or_default();
    for scene in old {
        if let Ok(scene) = world.get_entity_mut(scene) {
            scene.despawn();
        }
    }
    let rig = world
        .spawn((
            Name::new("rig"),
            Transform::default(),
            Visibility::default(),
            AnimationPlayer::default(),
            ChildOf(entity),
        ))
        .id();
    let tree = &figure.rig;
    let mut placed = vec![Entity::PLACEHOLDER; tree.nodes.len()];
    let mut joints = HashMap::new();
    let mut tails = HashMap::new();
    for node in tree.downward() {
        let joint = &tree.nodes[node];
        let parent = joint.parent.map_or(rig, |parent| placed[parent]);
        placed[node] = world
            .spawn((
                Name::new(joint.name.clone()),
                Transform {
                    translation: joint.rest.translation,
                    rotation: joint.rest.rotation,
                    scale: joint.rest.scale,
                },
                Visibility::default(),
                AnimationTargetId::from_iter(tree.path(node)),
                AnimatedBy(rig),
                ChildOf(parent),
            ))
            .id();
        joints.insert(joint.name.clone(), placed[node]);
        tails.extend(tail(tree, node).map(|tail| (placed[node], tail)));
    }
    let skins = figure
        .skins
        .iter()
        .map(|skin| spawn_skin(world, entity, skin))
        .collect();
    world.entity_mut(entity).insert(Body {
        figure: figure.clone(),
        rig,
        skins,
        joints,
        tails,
        worn: HashMap::new(),
        limbs: Vec::new(),
    });
    world.trigger(RigReady { entity });
}

/// Swaps the skins of the body standing under `entity` for `skins`, keeping its rig and what plays
/// on it: a skin it already wears stays as it is.
pub fn reskin(world: &mut World, entity: Entity, skins: &[Skin]) {
    let Some(body) = world.get::<Body>(entity) else {
        return;
    };
    let kept: Vec<(Skin, Entity)> = body
        .figure
        .skins
        .iter()
        .cloned()
        .zip(body.skins.iter().copied())
        .collect();
    let mut scenes: Vec<Entity> = Vec::new();
    for (file, scene) in &kept {
        if !skins.contains(file)
            && let Ok(scene) = world.get_entity_mut(*scene)
        {
            scene.despawn();
        }
    }
    for skin in skins {
        match kept.iter().find(|(file, _)| file == skin) {
            Some((_, scene)) => scenes.push(*scene),
            None => scenes.push(spawn_skin(world, entity, skin)),
        }
    }
    if let Some(mut body) = world.get_mut::<Body>(entity) {
        body.figure.skins = skins.to_vec();
        body.worn.retain(|skin, _| scenes.contains(skin));
        body.gather_limbs();
        body.skins = scenes;
    }
}

fn spawn_skin(world: &mut World, body: Entity, skin: &Skin) -> Entity {
    let scene = world
        .resource::<AssetServer>()
        .load(GltfAssetLabel::Scene(0).from_asset(skin.mesh.clone()));
    world
        .spawn((
            WorldAssetRoot(scene.clone()),
            Transform::default(),
            Visibility::Hidden,
            SkinOf {
                body,
                file: scene,
                paint: skin.paint.clone(),
                spawned: false,
            },
            ChildOf(body),
        ))
        .id()
}

pub struct BodyPlugin;

impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Paints>()
            .init_resource::<PendingClips>()
            .init_asset::<GltfClips>()
            .register_asset_loader(GltfClipsLoader)
            .add_observer(stand)
            .add_systems(
                Update,
                (
                    super::paint::advance_bakes.before(bind),
                    bind,
                    make_pending,
                    super::paint::forget_dropped,
                ),
            );
    }
}

/// A skin's scene, once spawned, is marked ready to bind.
fn stand(ready: On<WorldInstanceReady>, mut skins: Query<&mut SkinOf>) {
    if let Ok(mut skin) = skins.get_mut(ready.entity) {
        skin.spawned = true;
    }
}

/// The meshes a skin's scene draws, what loads them, and what binds them to joints.
#[derive(bevy::ecs::system::SystemParam)]
struct SkinMeshes<'w, 's> {
    skinned: Query<
        'w,
        's,
        (
            &'static mut SkinnedMesh,
            &'static Mesh3d,
            Option<&'static mut MeshMaterial3d<WorldMaterial>>,
        ),
    >,
    meshes: Res<'w, Assets<Mesh>>,
    binds: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
    assets: Res<'w, AssetServer>,
}

/// Binds the skinned meshes of every spawned skin to its body's rig once the rig stands, learns
/// the limbs they hang on its joints, paints them once their paint is baked, and shows the skin.
fn bind(
    mut skins: Query<(Entity, &SkinOf, &mut Visibility)>,
    mut bodies: Query<&mut Body>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut meshes: SkinMeshes,
    mut painter: Painter,
    mut commands: Commands,
) {
    for (scene, skin, mut visibility) in &mut skins {
        let Ok(mut body) = bodies.get_mut(skin.body) else {
            continue;
        };
        if !skin.spawned
            || body.joints.is_empty()
            || !meshes.assets.is_loaded_with_dependencies(&skin.file)
        {
            continue;
        }
        let texture = match &skin.paint {
            Some(paint) => match painter.texture(paint) {
                Some(texture) => Some(texture),
                None if painter.failed(paint) => None,
                None => continue,
            },
            None => None,
        };
        let mut limbs = Vec::new();
        for node in children.iter_descendants(scene) {
            let Ok((mut mesh, drawn, material)) = meshes.skinned.get_mut(node) else {
                continue;
            };
            let bound: Vec<Entity> = mesh
                .joints
                .iter()
                .map(|&joint| {
                    names
                        .get(joint)
                        .ok()
                        .and_then(|name| body.joint(name.as_str()))
                        .unwrap_or(joint)
                })
                .collect();
            mesh.joints = bound;
            if let (Some(drawn), Some(binds)) = (
                meshes.meshes.get(&drawn.0),
                meshes.binds.get(&mesh.inverse_bindposes),
            ) {
                limbs.extend(limbs_of(drawn, binds, &mesh.joints, &body.tails));
            }
            if let (Some(texture), Some(mut material)) = (&texture, material)
                && let Some(painted) = painter.painted(&material.0, texture)
            {
                material.0 = painted;
            }
        }
        body.wear(scene, limbs);
        *visibility = Visibility::Inherited;
        commands.entity(scene).remove::<SkinOf>();
    }
}

/// Where the bone of the rig's `node` ends in its own space: at its one child joint, where that
/// lies off the joint itself. A joint that branches or ends has no bone.
fn tail(tree: &NodeTree, node: usize) -> Option<Vec3> {
    let mut children = tree
        .nodes
        .iter()
        .filter(|child| child.parent == Some(node) && !child.mesh);
    match (children.next(), children.next()) {
        (Some(child), None) => Some(child.rest.translation).filter(|tail| *tail != Vec3::ZERO),
        _ => None,
    }
}

/// The limbs a skinned `mesh` hangs on the `joints` it is bound to, by the inverse `binds` of its
/// skin: the flesh of each joint that weighs some vertex most, told by where those vertices lie
/// round it.
fn limbs_of(
    mesh: &Mesh,
    binds: &[Mat4],
    joints: &[Entity],
    tails: &HashMap<Entity, Vec3>,
) -> Vec<Limb> {
    let (
        Some(VertexAttributeValues::Float32x3(positions)),
        Some(VertexAttributeValues::Uint16x4(indices)),
        Some(VertexAttributeValues::Float32x4(weights)),
    ) = (
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX),
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT),
    )
    else {
        return Vec::new();
    };
    let mut hung = vec![Vec::new(); joints.len().min(binds.len())];
    for ((position, index), weight) in positions.iter().zip(indices).zip(weights) {
        let heaviest = (0..4)
            .max_by(|&a, &b| weight[a].total_cmp(&weight[b]))
            .map_or(0, |at| index[at] as usize);
        if let Some(flesh) = hung.get_mut(heaviest) {
            flesh.push(binds[heaviest].transform_point3(Vec3::from_array(*position)));
        }
    }
    hung.into_iter()
        .enumerate()
        .filter(|(_, flesh)| !flesh.is_empty())
        .flat_map(|(slot, flesh)| {
            let joint = joints[slot];
            match tails.get(&joint) {
                Some(&tail) => vec![on_bone(joint, tail, &flesh)],
                None => along_flesh(joint, &flesh),
            }
        })
        .collect()
}

/// The `flesh` of a joint whose bone runs to `tail`: how far from the bone it lies on average.
fn on_bone(joint: Entity, tail: Vec3, flesh: &[Vec3]) -> Limb {
    let girth = flesh.iter().map(|&at| from_bone(at, tail)).sum::<f32>() / flesh.len() as f32;
    Limb {
        joint,
        head: Vec3::ZERO,
        tail,
        girth,
        tail_girth: girth,
    }
}

/// The `flesh` of a joint with no bone of its own, as bones laid end to end the way the flesh is
/// longest, each as thick at its ends as the flesh there: a hull comes out long, full amidships
/// and narrow at its bow, a head or a slime as one short bone in a ball of flesh.
fn along_flesh(joint: Entity, flesh: &[Vec3]) -> Vec<Limb> {
    let middle = flesh.iter().sum::<Vec3>() / flesh.len() as f32;
    let way = longest_way(flesh, middle);
    let laid: Vec<(f32, f32)> = flesh
        .iter()
        .map(|&at| {
            let along = (at - middle).dot(way);
            (along, (at - middle - way * along).length())
        })
        .collect();
    let (from, to) = laid
        .iter()
        .fold((f32::MAX, f32::MIN), |(from, to), (along, _)| {
            (from.min(*along), to.max(*along))
        });
    let whole = girth_between(&laid, from, to).unwrap_or(0.0);
    let length = to - from;
    let bones = if whole > 0.0 {
        ((length / (whole * 2.0)).ceil() as usize).clamp(1, FLESH_BONES)
    } else {
        1
    };
    let span = length / bones as f32;
    let mut ends: Vec<(f32, f32)> = (0..=bones)
        .map(|end| {
            let along = from + span * end as f32;
            let girth = girth_between(&laid, along - span / 2.0, along + span / 2.0);
            (along, girth.unwrap_or(whole))
        })
        .collect();
    // The flesh ends where the round end of its outermost bone does.
    ends[0].0 += ends[0].1.min(span / 2.0);
    ends[bones].0 -= ends[bones].1.min(span / 2.0);
    ends.windows(2)
        .map(|bone| Limb {
            joint,
            head: middle + way * bone[0].0,
            tail: middle + way * bone[1].0,
            girth: bone[0].1,
            tail_girth: bone[1].1,
        })
        .collect()
}

/// How many bones the flesh of a joint with none of its own is told by at most.
const FLESH_BONES: usize = 4;

/// The way `flesh` lying round `middle` is longest.
fn longest_way(flesh: &[Vec3], middle: Vec3) -> Vec3 {
    let spread = flesh.iter().fold(Mat3::ZERO, |spread, &at| {
        let off = at - middle;
        spread + Mat3::from_cols(off * off.x, off * off.y, off * off.z)
    });
    let widest = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .max_by(|a, b| a.dot(spread * *a).total_cmp(&b.dot(spread * *b)))
        .unwrap_or(Vec3::X);
    (0..WAY_STEPS).fold(widest, |way, _| (spread * way).normalize_or(way))
}

/// How many times a way is turned toward the longest of its flesh: each turn takes it most of
/// what is left, so this many leave it within what a mesh's vertices can tell.
const WAY_STEPS: usize = 16;

/// How far round its way the flesh `laid` along it lies between `from` and `to`, each point told
/// by how far along the way and how far from it: the mean of its outer half, so what a hull
/// holds within it (thwarts, a mast's foot) leaves it no thinner. None where no flesh lies.
fn girth_between(laid: &[(f32, f32)], from: f32, to: f32) -> Option<f32> {
    let mut around: Vec<f32> = laid
        .iter()
        .filter(|(along, _)| (from..=to).contains(along))
        .map(|(_, around)| *around)
        .collect();
    around.sort_by(f32::total_cmp);
    let outer = &around[around.len() / 2..];
    (!outer.is_empty()).then(|| outer.iter().sum::<f32>() / outer.len() as f32)
}

/// How far `at` lies from the bone running from its joint to `tail`, all in the joint's space.
fn from_bone(at: Vec3, tail: Vec3) -> f32 {
    let share = (at.dot(tail) / tail.length_squared()).clamp(0.0, 1.0);
    at.distance(tail * share)
}

fn skinned_boxes(world: &World, node: Entity) -> Option<Vec<[Vec3; 8]>> {
    let skinned = world.get::<SkinnedMesh>(node)?;
    let mesh = world
        .resource::<Assets<Mesh>>()
        .get(&world.get::<Mesh3d>(node)?.0)?;
    let bindposes = world
        .resource::<Assets<SkinnedMeshInverseBindposes>>()
        .get(&skinned.inverse_bindposes)?;
    let boxes = mesh
        .skinned_mesh_bounds()?
        .iter()
        .filter_map(|(joint, bounds)| {
            let index = usize::from(joint.0);
            let placed = world.get::<GlobalTransform>(*skinned.joints.get(index)?)?;
            let from_model = placed.affine() * Affine3A::from_mat4(*bindposes.get(index)?);
            Some(
                box_corners(bounds.center, bounds.half_size)
                    .map(|corner| from_model.transform_point3(corner)),
            )
        })
        .collect();
    Some(boxes)
}

fn mesh_box(world: &World, node: Entity) -> Option<[Vec3; 8]> {
    let (bounds, placed) = (
        world.get::<Aabb>(node)?,
        world.get::<GlobalTransform>(node)?,
    );
    Some(
        box_corners(bounds.center.into(), bounds.half_extents.into())
            .map(|corner| placed.transform_point(corner)),
    )
}

fn box_corners(center: Vec3, half: Vec3) -> [Vec3; 8] {
    std::array::from_fn(|corner| {
        let side = |bit: usize| if corner & bit == 0 { -1.0 } else { 1.0 };
        center + half * Vec3::new(side(1), side(2), side(4))
    })
}
