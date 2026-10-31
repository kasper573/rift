//! Clips made for a rig from the keyframes of glTF files: a file's clips read whole, turned into
//! keys for the rig's joints by whatever made the rig play them, and each joint's keys as curves
//! the body's player plays on the joint of that name, at its path of names down the rig.

use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};

use bevy::animation::AnimationTargetId;
use bevy::animation::animatable::Animatable;
use bevy::animation::animated_field;
use bevy::animation::animation_curves::{
    AnimatableCurve, AnimatableKeyframeCurve, AnimatableProperty,
};
use bevy::animation::gltf_curves::SteppedKeyframeCurve;
use bevy::prelude::*;
use bevy::reflect::{FromReflect, GetTypeRegistration, Typed};
use rayon::prelude::*;

use crate::core::content::gltf::{self, ClipKeys, Keys, NodeTree};
use crate::core::content::retarget;

/// A glTF file's tree of nodes and every clip it holds, read whole.
#[derive(Asset, TypePath)]
pub struct GltfClips {
    pub tree: NodeTree,
    pub clips: Vec<ClipKeys>,
}

/// Reads a glTF binary's clips whole when they are asked for as such: it claims no extension, so a
/// glTF loaded as anything else is read as the renderer reads it.
#[derive(TypePath)]
pub struct GltfClipsLoader;

impl AssetLoader for GltfClipsLoader {
    type Asset = GltfClips;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<GltfClips, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let unreadable =
            |error: String| std::io::Error::new(std::io::ErrorKind::InvalidData, error);
        let library = gltf::ClipLibrary::read(&bytes).map_err(unreadable)?;
        let clips = (0..library.clips())
            .map(|index| library.keys(index))
            .collect::<Result<_, _>>()
            .map_err(unreadable)?;
        Ok(GltfClips {
            tree: library.tree(),
            clips,
        })
    }

    fn extensions(&self) -> &[&str] {
        &[]
    }
}

/// What turns a clip as its file holds it, with the file's tree, into keys for a rig's joints.
pub type ClipMaker = Arc<dyn Fn(&NodeTree, &ClipKeys) -> ClipKeys + Send + Sync>;

/// What plays a clip on `rig`: one made on a rig whose joints are ours by name plays as it is, one
/// made on a source rig through `pairs`, each a joint of ours and the joint of his it copies, by
/// name.
pub fn maker(rig: Arc<NodeTree>, pairs: Option<Vec<(String, String)>>) -> ClipMaker {
    Arc::new(move |tree: &NodeTree, keys: &ClipKeys| match &pairs {
        None => retarget::own(&rig, tree, keys),
        Some(pairs) => {
            let pairs: Vec<(usize, usize)> = pairs
                .iter()
                .filter_map(|(ours, his)| Some((rig.named(ours)?, tree.named(his)?)))
                .collect();
            retarget::retarget(&rig, tree, &pairs, keys)
        }
    })
}

/// Clips asked for and not yet made: each made once its file is read.
#[derive(Resource, Default)]
pub struct PendingClips(Vec<Pending>);

struct Pending {
    clip: Handle<AnimationClip>,
    file: Handle<GltfClips>,
    animation: usize,
    rig: Arc<NodeTree>,
    make: ClipMaker,
}

/// A clip for `rig` made by `make` from the clip at `animation` among those of the glTF at `file`:
/// empty until the file is read, then made in place.
pub fn rig_clip(
    world: &mut World,
    rig: Arc<NodeTree>,
    file: &str,
    animation: usize,
    make: ClipMaker,
) -> Handle<AnimationClip> {
    let clip = world
        .resource_mut::<Assets<AnimationClip>>()
        .add(AnimationClip::default());
    let file = world
        .resource::<AssetServer>()
        .load::<GltfClips>(file.to_owned());
    world
        .get_resource_or_init::<PendingClips>()
        .0
        .push(Pending {
            clip: clip.clone(),
            file,
            animation,
            rig,
            make,
        });
    clip
}

/// Makes the pending clips whose files have been read, across the threads, and gives up on one
/// whose file can't be.
pub fn make_pending(
    mut pending: ResMut<PendingClips>,
    files: Res<Assets<GltfClips>>,
    assets: Res<AssetServer>,
    mut clips: ResMut<Assets<AnimationClip>>,
) {
    let (read, waiting): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut pending.0)
        .into_iter()
        .filter(|waiting| {
            let failed = assets.load_state(&waiting.file).is_failed();
            if failed {
                warn!("could not read the clips of {:?}", waiting.file.path());
            }
            !failed
        })
        .partition(|waiting| files.contains(&waiting.file));
    pending.0 = waiting;
    let made: Vec<(Handle<AnimationClip>, AnimationClip)> = read
        .into_par_iter()
        .filter_map(|read| {
            let file = files.get(&read.file)?;
            let keys = file.clips.get(read.animation)?;
            Some((
                read.clip,
                animation_clip(&read.rig, &(read.make)(&file.tree, keys)),
            ))
        })
        .collect();
    for (clip, made) in made {
        let _ = clips.insert(clip.id(), made);
    }
}

/// `keys`, keyframes for the joints of `rig` by index, as a clip moving those joints of a body
/// standing on `rig`.
pub fn animation_clip(rig: &NodeTree, keys: &ClipKeys) -> AnimationClip {
    let mut clip = AnimationClip::default();
    let length = keys.length.0.max(f32::EPSILON);
    for track in &keys.tracks {
        let target = AnimationTargetId::from_iter(rig.path(track.node));
        if let Some(keys) = &track.translation {
            add(
                &mut clip,
                target,
                animated_field!(Transform::translation),
                keys,
                length,
            );
        }
        if let Some(keys) = &track.rotation {
            add(
                &mut clip,
                target,
                animated_field!(Transform::rotation),
                keys,
                length,
            );
        }
        if let Some(keys) = &track.scale {
            add(
                &mut clip,
                target,
                animated_field!(Transform::scale),
                keys,
                length,
            );
        }
    }
    clip.set_duration(length);
    clip
}

fn add<P, T>(
    clip: &mut AnimationClip,
    target: AnimationTargetId,
    field: P,
    keys: &Keys<T>,
    length: f32,
) where
    P: AnimatableProperty<Property = T> + Clone,
    T: Animatable + Clone + std::fmt::Debug + FromReflect + GetTypeRegistration + Typed + TypePath,
{
    let samples: Vec<(f32, T)> = match (keys.times.len(), keys.values.first()) {
        (1, Some(value)) => vec![(0.0, value.clone()), (length, value.clone())],
        _ => keys
            .times
            .iter()
            .copied()
            .zip(keys.values.iter().cloned())
            .collect(),
    };
    if keys.stepped {
        if let Ok(curve) = SteppedKeyframeCurve::new(samples) {
            clip.add_curve_to_target(target, AnimatableCurve::new(field, curve));
        }
    } else if let Ok(curve) = AnimatableKeyframeCurve::new(samples) {
        clip.add_curve_to_target(target, AnimatableCurve::new(field, curve));
    }
}
