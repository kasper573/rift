//! A body standing in one clip that loops, or held at one moment of it: how a model turns on a
//! turntable and is scrubbed through the clips made for its rig. It wears nothing of the game's
//! mechanics; the actors the game plays are puppets.

use bevy::animation::RepeatAnimation;
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::transition::AnimationTransitions;
use bevy::prelude::*;

use super::body::{Body, RigReady};
use crate::core::time::Seconds;

/// A clip by its number among the animations of the glTF at `file`, made on the rig.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigClip {
    pub file: String,
    pub index: usize,
}

/// The clip a body stands in, and the moment of it the body is held at when it is not left to loop.
#[derive(Component, Clone, Debug)]
pub struct Mannequin {
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
    hold: Option<Seconds>,
}

impl Mannequin {
    /// Holds the body at `at` into its clip, or lets it loop again with `None`.
    pub fn hold(&mut self, at: Option<Seconds>) {
        self.hold = at;
    }
}

/// A mannequin playing `clip` on a loop, to spawn on a body.
pub fn mannequin(world: &mut World, clip: &RigClip) -> Mannequin {
    let assets = world.resource::<AssetServer>().clone();
    let mut graph = AnimationGraph::new();
    let node = graph.add_clip(
        assets.load(GltfAssetLabel::Animation(clip.index).from_asset(clip.file.clone())),
        1.0,
        graph.root,
    );
    Mannequin {
        graph: world.resource_mut::<Assets<AnimationGraph>>().add(graph),
        node,
        hold: None,
    }
}

/// A mannequin's rig, once it stands, starts its clip.
pub(super) fn start(
    ready: On<RigReady>,
    mannequins: Query<(&Mannequin, &Body)>,
    mut commands: Commands,
) {
    let Ok((mannequin, body)) = mannequins.get(ready.entity) else {
        return;
    };
    let Some(player) = body.player() else {
        return;
    };
    let mut animation = AnimationPlayer::default();
    let mut transitions = AnimationTransitions::new();
    transitions
        .play(&mut animation, mannequin.node, std::time::Duration::ZERO)
        .set_repeat(RepeatAnimation::Forever);
    commands.entity(player).insert((
        animation,
        AnimationGraphHandle(mannequin.graph.clone()),
        transitions,
    ));
}

/// Every held mannequin stands at its moment; one let go loops on.
pub(super) fn hold(
    mannequins: Query<(&Mannequin, &Body)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (mannequin, body) in &mannequins {
        let Some(player) = body.player() else {
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
