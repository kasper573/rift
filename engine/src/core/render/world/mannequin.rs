//! A rig standing in one clip that loops, or held at one moment of it: how a resident is shown
//! where it lives, a model turns on a turntable and is scrubbed through its clips, and a prop
//! poses for its thumbnail. It wears nothing of the game's mechanics; the actors the game plays
//! are puppets.

use bevy::animation::RepeatAnimation;
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::transition::AnimationTransitions;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

use crate::core::time::Seconds;

/// The clip a rig stands in, by its number among the model's clips, and the moment of it the rig
/// is held at when it is not left to loop.
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

/// The scene of the model at `path` with its clip number `clip` looping, to spawn with a
/// transform.
pub fn mannequin(world: &mut World, path: &str, clip: usize) -> (WorldAssetRoot, Mannequin) {
    let assets = world.resource::<AssetServer>().clone();
    let mut graph = AnimationGraph::new();
    let node = graph.add_clip(
        assets.load(GltfAssetLabel::Animation(clip).from_asset(path.to_owned())),
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

/// A mannequin's scene, once spawned, starts its clip on the rig's player.
pub(super) fn start(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    mut mannequins: Query<&mut Mannequin>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
) {
    let Ok(mut mannequin) = mannequins.get_mut(ready.entity) else {
        return;
    };
    let Some(player) = children
        .iter_descendants(ready.entity)
        .find(|entity| players.contains(*entity))
    else {
        return;
    };
    let mut transitions = AnimationTransitions::new();
    if let Ok(mut playing) = players.get_mut(player) {
        transitions
            .play(&mut playing, mannequin.node, std::time::Duration::ZERO)
            .set_repeat(RepeatAnimation::Forever);
    }
    mannequin.player = Some(player);
    commands
        .entity(player)
        .insert((AnimationGraphHandle(mannequin.graph.clone()), transitions));
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
