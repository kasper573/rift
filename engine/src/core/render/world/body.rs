//! A body drawn as skins over one rig: the rig's scene stands beneath the body, and every skinned
//! mesh of each skin's scene is bound to the rig's joints of the same names, so a clip played on the
//! rig moves every skin, however many files they come from.

use std::collections::HashMap;

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

/// The glTF files a body is drawn from: the rig its joints stand in, and the skins bound to them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BodyFiles {
    pub rig: String,
    pub skins: Vec<String>,
}

/// A body standing under its entity: the scenes of its rig and skins, and once the rig stands, its
/// joints by name and the player its clips play on.
#[derive(Component)]
pub struct Body {
    files: BodyFiles,
    scenes: Vec<Entity>,
    joints: HashMap<String, Entity>,
    player: Option<Entity>,
}

/// Triggered on a body once its rig stands, its joints known and its player ready to play.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct RigReady {
    pub entity: Entity,
}

#[derive(Component)]
struct RigOf(Entity);

/// A skin's scene, hidden until its meshes are bound to the rig: at rest until then, it would
/// stand apart from a rig already moving.
#[derive(Component)]
struct SkinOf {
    body: Entity,
    spawned: bool,
}

impl Body {
    pub fn files(&self) -> &BodyFiles {
        &self.files
    }

    /// The joint of the rig named `name`, once the rig stands.
    pub fn joint(&self, name: &str) -> Option<Entity> {
        self.joints.get(name).copied()
    }

    /// Every node of the rig by its name, once the rig stands.
    pub fn joints(&self) -> &HashMap<String, Entity> {
        &self.joints
    }

    /// The entity the rig's clips play on, once the rig stands.
    pub fn player(&self) -> Option<Entity> {
        self.player
    }
}

/// Stands the body `files` describe under `entity`, in place of any body it stood before.
pub fn dress(world: &mut World, entity: Entity, files: &BodyFiles) {
    let old: Vec<Entity> = world
        .get::<Body>(entity)
        .map(|body| body.scenes.clone())
        .unwrap_or_default();
    for scene in old {
        if let Ok(scene) = world.get_entity_mut(scene) {
            scene.despawn();
        }
    }
    let assets = world.resource::<AssetServer>().clone();
    let load = |path: &str| {
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.to_owned())))
    };
    let mut scenes = vec![
        world
            .spawn((
                load(&files.rig),
                Transform::default(),
                Visibility::default(),
                RigOf(entity),
                ChildOf(entity),
            ))
            .id(),
    ];
    for skin in &files.skins {
        scenes.push(
            world
                .spawn((
                    load(skin),
                    Transform::default(),
                    Visibility::Hidden,
                    SkinOf {
                        body: entity,
                        spawned: false,
                    },
                    ChildOf(entity),
                ))
                .id(),
        );
    }
    world.entity_mut(entity).insert(Body {
        files: files.clone(),
        scenes,
        joints: HashMap::new(),
        player: None,
    });
}

pub struct BodyPlugin;

impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(stand).add_systems(Update, bind);
    }
}

/// A rig's scene, once spawned, is made playable and its body told so; a skin's is marked ready to
/// bind.
fn stand(
    ready: On<WorldInstanceReady>,
    rigs: Query<&RigOf>,
    mut skins: Query<&mut SkinOf>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut bodies: Query<&mut Body>,
    mut commands: Commands,
) {
    if let Ok(mut skin) = skins.get_mut(ready.entity) {
        skin.spawned = true;
        return;
    }
    let Ok(&RigOf(entity)) = rigs.get(ready.entity) else {
        return;
    };
    let Ok(mut body) = bodies.get_mut(entity) else {
        return;
    };
    let Some(player) = animate_rig(ready.entity, &children, &names, &mut commands) else {
        return;
    };
    body.joints = children
        .iter_descendants(ready.entity)
        .filter_map(|node| Some((names.get(node).ok()?.as_str().to_owned(), node)))
        .collect();
    body.player = Some(player);
    commands.trigger(RigReady { entity });
}

/// Binds the skinned meshes of every spawned skin to its body's rig once the rig stands, and shows
/// the skin.
fn bind(
    mut skins: Query<(Entity, &SkinOf, &mut Visibility)>,
    bodies: Query<&Body>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut meshes: Query<&mut SkinnedMesh>,
    mut commands: Commands,
) {
    for (scene, skin, mut visibility) in &mut skins {
        let Ok(body) = bodies.get(skin.body) else {
            continue;
        };
        if !skin.spawned || body.joints.is_empty() {
            continue;
        }
        for node in children.iter_descendants(scene) {
            let Ok(mut mesh) = meshes.get_mut(node) else {
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
        }
        *visibility = Visibility::Inherited;
        commands.entity(scene).remove::<SkinOf>();
    }
}

/// Gives a spawned glTF scene a player and names every node under it the way a clip made on the
/// same rig names its targets, then gives that player back. The loader names a clip's targets by
/// the path from its file's top node down, below the entity it spawns for the scene itself, and
/// names nodes only for the clips its own file holds, so a rig carrying none would otherwise play
/// none made for it.
fn animate_rig(
    instance: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    commands: &mut Commands,
) -> Option<Entity> {
    let size = |node: Entity| children.iter_descendants(node).count();
    let scene = children
        .get(instance)
        .ok()?
        .iter()
        .max_by_key(|&node| size(node))?;
    let mut stack: Vec<(Entity, Vec<Name>)> = children
        .get(scene)
        .ok()?
        .iter()
        .map(|top| (top, Vec::new()))
        .collect();
    while let Some((node, mut path)) = stack.pop() {
        let Ok(name) = names.get(node) else {
            continue;
        };
        path.push(name.clone());
        commands.entity(node).insert((
            AnimationTargetId::from_names(path.iter()),
            AnimatedBy(scene),
        ));
        if let Ok(below) = children.get(node) {
            stack.extend(below.iter().map(|child| (child, path.clone())));
        }
    }
    commands.entity(scene).insert(AnimationPlayer::default());
    Some(scene)
}
