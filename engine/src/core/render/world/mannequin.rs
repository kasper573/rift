//! A rig standing in one clip that loops, or held at one moment of it: how a model turns on a
//! turntable and is scrubbed through the clips made for its rig. It wears nothing of the game's
//! mechanics; the actors the game plays are puppets.

use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::transition::AnimationTransitions;
use bevy::animation::{AnimatedBy, AnimationTargetId, RepeatAnimation};
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

use crate::core::time::Seconds;

/// A clip by its number among the animations of the glTF at `file`: the model's own, or one made
/// on the same rig.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigClip {
    pub file: String,
    pub index: usize,
}

/// The clip a rig stands in, and the moment of it the rig is held at when it is not left to loop.
#[derive(Component, Clone, Debug)]
pub struct Mannequin {
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
    hold: Option<Seconds>,
    player: Option<Entity>,
}

impl Mannequin {
    /// Holds the rig at `at` into its clip, or lets it loop again with `None`.
    pub fn hold(&mut self, at: Option<Seconds>) {
        self.hold = at;
    }
}

/// The scene of the model at `path` with `clip` looping, to spawn with a transform.
pub fn mannequin(world: &mut World, path: &str, clip: &RigClip) -> (WorldAssetRoot, Mannequin) {
    let assets = world.resource::<AssetServer>().clone();
    let mut graph = AnimationGraph::new();
    let node = graph.add_clip(
        assets.load(GltfAssetLabel::Animation(clip.index).from_asset(clip.file.clone())),
        1.0,
        graph.root,
    );
    let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
    (
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.to_owned()))),
        Mannequin {
            graph,
            node,
            hold: None,
            player: None,
        },
    )
}

/// Gives a spawned glTF scene a player and names every node under it the way a clip made on the
/// same rig names its targets, then gives that player back. The loader names a clip's targets by
/// the path from its file's top node down, below the entity it spawns for the scene itself, and
/// names nodes only for the clips its own file holds, so a skin carrying none would otherwise play
/// none made for its rig.
pub fn animate_rig(
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

/// A mannequin's scene, once spawned, starts its clip on the rig's player.
pub(super) fn start(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut mannequins: Query<&mut Mannequin>,
    mut commands: Commands,
) {
    let Ok(mut mannequin) = mannequins.get_mut(ready.entity) else {
        return;
    };
    let Some(player) = animate_rig(ready.entity, &children, &names, &mut commands) else {
        return;
    };
    let mut animation = AnimationPlayer::default();
    let mut transitions = AnimationTransitions::new();
    transitions
        .play(&mut animation, mannequin.node, std::time::Duration::ZERO)
        .set_repeat(RepeatAnimation::Forever);
    mannequin.player = Some(player);
    commands.entity(player).insert((
        animation,
        AnimationGraphHandle(mannequin.graph.clone()),
        transitions,
    ));
}

/// Every held mannequin stands at its moment; one let go loops on.
pub(super) fn hold(mannequins: Query<&Mannequin>, mut players: Query<&mut AnimationPlayer>) {
    for mannequin in &mannequins {
        let Some(player) = mannequin.player else {
            continue;
        };
        let Ok(mut player) = players.get_mut(player) else {
            continue;
        };
        let Some(active) = player.animation_mut(mannequin.node) else {
            continue;
        };
        match mannequin.hold {
            Some(at) => {
                active.seek_to(at.0).pause();
            }
            None => {
                active.resume();
            }
        }
    }
}
