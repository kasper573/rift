//! A body drawn as skins over one rig: the rig's joints stand beneath the body, and every skinned
//! mesh of each skin's scene is bound to the rig's joints of the same names, so a clip played on the
//! rig moves every skin, however many files they come from.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

use super::material::WorldMaterial;
use super::paint::{Paint, Painter, Paints};
use super::rig_clip::{GltfClips, GltfClipsLoader, PendingClips, make_pending};
use crate::core::content::gltf::NodeTree;
use crate::core::pace::Pace;

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

/// A body standing under its entity: its rig, the joints its clips play on, and the scenes of its
/// skins.
#[derive(Component)]
pub struct Body {
    figure: Figure,
    rig: Entity,
    skins: Vec<Entity>,
    joints: HashMap<String, Entity>,
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
        let pace = app.world_mut().get_resource_or_init::<Pace>().clone();
        app.init_resource::<Paints>()
            .init_resource::<PendingClips>()
            .init_asset::<GltfClips>()
            .register_asset_loader(GltfClipsLoader { pace })
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

/// Binds the skinned meshes of every spawned skin to its body's rig once the rig stands, paints
/// them once their paint is baked, and shows the skin.
fn bind(
    mut skins: Query<(Entity, &SkinOf, &mut Visibility)>,
    bodies: Query<&Body>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut meshes: Query<(&mut SkinnedMesh, Option<&mut MeshMaterial3d<WorldMaterial>>)>,
    mut painter: Painter,
    mut commands: Commands,
) {
    for (scene, skin, mut visibility) in &mut skins {
        let Ok(body) = bodies.get(skin.body) else {
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
        for node in children.iter_descendants(scene) {
            let Ok((mut mesh, material)) = meshes.get_mut(node) else {
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
            if let (Some(texture), Some(mut material)) = (&texture, material)
                && let Some(painted) = painter.painted(&material.0, texture)
            {
                material.0 = painted;
            }
        }
        *visibility = Visibility::Inherited;
        commands.entity(scene).remove::<SkinOf>();
    }
}
