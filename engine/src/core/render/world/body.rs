//! A body drawn as skins over one rig: the rig's joints stand beneath the body, and every skinned
//! mesh of each skin's scene is bound to the rig's joints of the same names, so a clip played on the
//! rig moves every skin, however many files they come from.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::mesh::VertexAttributeValues;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

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

/// The flesh a body's skins hang on one joint of its rig: the joint's bone, from the joint to
/// `tail` in the joint's own space, and how far round it the skin lies on average, `girth`, in the
/// same space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limb {
    pub joint: Entity,
    pub tail: Vec3,
    pub girth: f32,
}

/// Triggered on a body once its rig stands, its joints known and its player ready to play.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct RigReady {
    pub entity: Entity,
}

/// A skin's scene, hidden until its meshes are bound to the rig and painted: at rest until then,
/// it would stand apart from a rig already moving.
#[derive(Component)]
pub(super) struct SkinOf {
    body: Entity,
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

    /// The entity the rig's clips play on.
    pub fn player(&self) -> Option<Entity> {
        Some(self.rig)
    }

    /// The limbs of every skin bound to the rig, one for each joint some skin hangs flesh on.
    pub fn limbs(&self) -> &[Limb] {
        &self.limbs
    }

    fn wear(&mut self, skin: Entity, limbs: Vec<Limb>) {
        self.worn.insert(skin, limbs);
        self.gather_limbs();
    }

    fn gather_limbs(&mut self) {
        let mut thickest: HashMap<Entity, Limb> = HashMap::new();
        for limb in self.worn.values().flatten() {
            thickest
                .entry(limb.joint)
                .and_modify(|kept| kept.girth = kept.girth.max(limb.girth))
                .or_insert(*limb);
        }
        self.limbs = thickest.into_values().collect();
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
        tails.insert(placed[node], tail(tree, node));
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
            WorldAssetRoot(scene),
            Transform::default(),
            Visibility::Hidden,
            SkinOf {
                body,
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

/// The meshes a skin's scene draws, and what binds them to joints.
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
        if !skin.spawned || body.joints.is_empty() {
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

/// Where the bone of the rig's `node` ends in its own space: at its one child joint, or, where it
/// branches or ends, at the joint itself.
fn tail(tree: &NodeTree, node: usize) -> Vec3 {
    let mut children = tree
        .nodes
        .iter()
        .filter(|child| child.parent == Some(node) && !child.mesh);
    match (children.next(), children.next()) {
        (Some(child), None) => child.rest.translation,
        _ => Vec3::ZERO,
    }
}

/// The limbs a skinned `mesh` hangs on the `joints` it is bound to, by the inverse `binds` of its
/// skin: for each joint that weighs some vertex most, how far from the joint's bone those vertices
/// lie on average.
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
    let tail_of = |slot: usize| tails.get(&joints[slot]).copied().unwrap_or(Vec3::ZERO);
    let mut around = vec![(0.0f32, 0u32); joints.len().min(binds.len())];
    for ((position, index), weight) in positions.iter().zip(indices).zip(weights) {
        let heaviest = (0..4)
            .max_by(|&a, &b| weight[a].total_cmp(&weight[b]))
            .map_or(0, |at| index[at] as usize);
        let Some((sum, count)) = around.get_mut(heaviest) else {
            continue;
        };
        let at = binds[heaviest].transform_point3(Vec3::from_array(*position));
        *sum += from_bone(at, tail_of(heaviest));
        *count += 1;
    }
    around
        .into_iter()
        .enumerate()
        .filter(|(_, (_, count))| *count > 0)
        .map(|(slot, (sum, count))| Limb {
            joint: joints[slot],
            tail: tail_of(slot),
            girth: sum / count as f32,
        })
        .collect()
}

/// How far `at` lies from the bone running from its joint to `tail`, all in the joint's space.
fn from_bone(at: Vec3, tail: Vec3) -> f32 {
    let along = tail.length_squared();
    let share = if along > 0.0 {
        (at.dot(tail) / along).clamp(0.0, 1.0)
    } else {
        0.0
    };
    at.distance(tail * share)
}
