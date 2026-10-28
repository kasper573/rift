//! A skeleton as the game plays it: our rig put together from the joints of its files, every clip
//! it names found in its file and matched to the rig it is made on, and what each hand's clasp
//! turns.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::math::{Mat4, Quat};

use super::skeleton::{
    BuildName, Clasp, Clip, Cue, GroupName, JointName, Layer, MAX_CLIP, MAX_JOINTS, MIN_CLIP, Rig,
    SkeletonDef, SourceRig, check_cue_times,
};
use super::stance::StanceId;
use super::{Action, Motion, Pose};
use crate::core::content::RowCheck;
use crate::core::content::gltf::{GltfInfo, NodeTree, REST_SHIFT, REST_TURN, Rest, TreeNode};
use crate::core::content::retarget;
use crate::core::time::Seconds;
use crate::core::units::MetersPerSec;

/// A skeleton put together: our rig, every clip it names as it plays, and what each hand's clasp
/// turns.
pub struct BuiltSkeleton {
    pub def: SkeletonDef,
    /// Our rig: every joint of the rig's files, with each top but the body's hung where it hangs.
    pub rig: Arc<NodeTree>,
    /// Every clip the skeleton names, each once, by its place here.
    pub clips: Vec<PlayedClip>,
    index: HashMap<Clip, usize>,
    /// What each hand's clasp turns, by the hand's place among the hands: each joint of its fingers
    /// the clasp's clip turns, by the joint's index in the rig, with the turn it opens on. Empty for
    /// a hand without a clasp, or whose clasp turns none of its fingers.
    pub clasps: Vec<Vec<(usize, Quat)>>,
    gaits: HashMap<(Option<BuildName>, Option<StanceId>), Vec<Paced>>,
}

/// A clip as a skeleton plays it: which animation of its file it is, how long it plays, its cues
/// and pace, and the source rig it was made on when that is not ours.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayedClip {
    pub clip: Clip,
    pub animation: usize,
    pub length: Seconds,
    pub source: Option<usize>,
    pub cues: Vec<Cue>,
    pub pace: Option<MetersPerSec>,
}

/// A gait a body may cover ground in, with the ground its clip covers at scale 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paced {
    pub gait: Action,
    pub pace: MetersPerSec,
}

impl BuiltSkeleton {
    /// The skeleton `def` put together from the files `infos` holds, by their paths, none while it
    /// has no rig to put together, or every problem that keeps it from playing, at its place in
    /// the skeleton's file.
    pub fn build(
        def: &SkeletonDef,
        infos: &HashMap<String, GltfInfo>,
    ) -> Result<Option<BuiltSkeleton>, RowCheck> {
        let mut cx = RowCheck::default();
        check_apexes(def, &mut cx);
        let rig = read_rig(&def.rig, infos, &mut cx);
        if let Some(rig) = &rig {
            check_joints(def, rig, &mut cx);
        }
        let clips = rig
            .as_ref()
            .map(|rig| played_clips(def, rig, infos, &mut cx))
            .unwrap_or_default();
        if !cx.is_empty() {
            return Err(cx);
        }
        let Some(rig) = rig else {
            return Ok(None);
        };
        let index: HashMap<Clip, usize> = clips
            .iter()
            .enumerate()
            .map(|(at, played)| (played.clip.clone(), at))
            .collect();
        let mut built = BuiltSkeleton {
            def: def.clone(),
            rig: Arc::new(rig),
            clips,
            index,
            clasps: Vec::new(),
            gaits: HashMap::new(),
        };
        built.clasps = def
            .hands
            .iter()
            .map(|hand| {
                hand.clasp
                    .as_ref()
                    .map_or_else(Vec::new, |clasp| built.clasped(clasp, infos))
            })
            .collect();
        for (index, hand) in def.hands.iter().enumerate() {
            if let Some(clasp) = &hand.clasp
                && !clasp.fingers.is_empty()
                && built.clasps[index].is_empty()
            {
                cx.error(
                    format!("/hands/{index}/clasp/clip"),
                    "clasp turns no fingers",
                    format!(
                        "{} turns none of hand {:?}'s fingers",
                        clasp.clip, hand.name
                    ),
                    "a clip turning the joints of the clasp's fingers",
                );
            }
        }
        if !cx.is_empty() {
            return Err(cx);
        }
        let builds: Vec<Option<BuildName>> = if def.builds.is_empty() {
            vec![None]
        } else {
            def.builds.iter().cloned().map(Some).collect()
        };
        let stances: Vec<Option<StanceId>> = std::iter::once(None)
            .chain(def.stances.iter().map(|set| Some(set.stance)))
            .collect();
        for build in &builds {
            for &stance in &stances {
                let gaits = gaits_of(def, build.as_ref(), stance);
                built.gaits.insert((build.clone(), stance), gaits);
            }
        }
        Ok(Some(built))
    }

    /// The gaits a body of `build` in `stance` may cover ground in.
    pub fn gaits(&self, build: Option<&BuildName>, stance: Option<StanceId>) -> &[Paced] {
        let build = build.cloned().or_else(|| self.def.builds.first().cloned());
        self.gaits
            .get(&(build.clone(), stance))
            .or_else(|| self.gaits.get(&(build, None)))
            .map_or(&[], Vec::as_slice)
    }

    /// Where among the skeleton's clips `clip` is, when the skeleton names it.
    pub fn played(&self, clip: &Clip) -> Option<usize> {
        self.index.get(clip).copied()
    }

    /// The joints of ours each joint of source rig `source` moves, as pairs of a joint of our rig
    /// and a node of `file`, a file made on that rig, both by index.
    pub fn pairs(&self, source: usize, file: &NodeTree) -> Vec<(usize, usize)> {
        self.def.sources[source]
            .pairs
            .iter()
            .filter_map(|(his, ours)| Some((self.rig.named(&ours.0)?, file.named(&his.0)?)))
            .collect()
    }

    /// Each joint of the clasp's fingers its clip turns at its first moment, with that turn.
    fn clasped(&self, clasp: &Clasp, infos: &HashMap<String, GltfInfo>) -> Vec<(usize, Quat)> {
        let Some(played) = self.played(&clasp.clip).map(|at| &self.clips[at]) else {
            return Vec::new();
        };
        let Some(info) = infos.get(played.clip.file().as_str()) else {
            return Vec::new();
        };
        let opening = &info.clips[played.animation].opening;
        let roots: Vec<usize> = clasp
            .fingers
            .iter()
            .filter_map(|finger| self.rig.named(&finger.0))
            .collect();
        let fingers = self
            .rig
            .joints()
            .filter(|&joint| roots.iter().any(|&root| self.rig.descends(joint, root)));
        match played.source {
            None => fingers
                .filter_map(|joint| {
                    let name = &self.rig.nodes[joint].name;
                    opening
                        .iter()
                        .find(|(node, _)| info.tree.nodes[*node].name == *name)
                        .map(|(_, turn)| (joint, *turn))
                })
                .collect(),
            Some(source) => {
                let pairs = self.pairs(source, &info.tree);
                let mut pose: Vec<Rest> = info.tree.nodes.iter().map(|node| node.rest).collect();
                for (node, turn) in opening {
                    pose[*node].rotation = *turn;
                }
                let ours = retarget::retargeted_pose(&self.rig, &info.tree, &pairs, &pose);
                fingers
                    .filter(|joint| {
                        pairs.iter().any(|(ours, his)| {
                            ours == joint && opening.iter().any(|(node, _)| node == his)
                        })
                    })
                    .map(|joint| (joint, ours[joint].rotation))
                    .collect()
            }
        }
    }
}

/// Our rig as `rig` reads it from its files: every joint any of them holds, where they all agree
/// on its parent and its rest, with each top but the body's hung where `rig` hangs it.
pub fn read_rig(
    rig: &Rig,
    infos: &HashMap<String, GltfInfo>,
    cx: &mut RowCheck,
) -> Option<NodeTree> {
    let mut tree = read_joints(rig, infos, cx)?;
    hang(&mut tree, &rig.hangs, cx);
    let tops: Vec<&str> = tree
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .map(|node| node.name.as_str())
        .collect();
    if tops.len() > 1 {
        cx.error(
            "/rig/hangs",
            "tops unhung",
            format!("{} have no parent", tops.join(", ")),
            "one top joint carrying the body, every other hung on a joint of ours",
        );
    }
    if tree.nodes.len() > MAX_JOINTS {
        cx.error(
            "/rig/files",
            "too many joints",
            format!("{} joints", tree.nodes.len()),
            format!("at most {MAX_JOINTS}"),
        );
    }
    cx.is_empty().then_some(tree)
}

/// Every joint the files of `rig` hold, where they all agree on its parent and its rest, each top
/// hanging under nothing.
pub fn read_joints(
    rig: &Rig,
    infos: &HashMap<String, GltfInfo>,
    cx: &mut RowCheck,
) -> Option<NodeTree> {
    if rig.files.is_empty() {
        return None;
    }
    let mut nodes: Vec<TreeNode> = Vec::new();
    let mut parents: Vec<Option<String>> = Vec::new();
    let mut read_from: Vec<usize> = Vec::new();
    let mut named: HashMap<String, usize> = HashMap::new();
    let mut disagreed = false;
    for (at, file) in rig.files.iter().enumerate() {
        let Some(info) = infos.get(file.as_str()) else {
            continue;
        };
        let tree = &info.tree;
        let joints: Vec<usize> = tree.joints().collect();
        if joints.is_empty() {
            cx.error(
                format!("/rig/files/{at}"),
                "no joints",
                format!("{:?} holds no node without a mesh", file.as_str()),
                "a file holding joints of the rig",
            );
            continue;
        }
        let worlds = tree.worlds();
        for joint in joints {
            let node = &tree.nodes[joint];
            let parent = node
                .parent
                .filter(|&parent| !tree.nodes[parent].mesh)
                .map(|parent| tree.nodes[parent].name.clone());
            let rest = match node.parent {
                Some(parent) if tree.nodes[parent].mesh => {
                    let (scale, rotation, translation) =
                        worlds[joint].to_scale_rotation_translation();
                    Rest {
                        translation,
                        rotation,
                        scale,
                    }
                }
                _ => node.rest,
            };
            match named.get(&node.name) {
                None => {
                    named.insert(node.name.clone(), nodes.len());
                    nodes.push(TreeNode {
                        name: node.name.clone(),
                        parent: None,
                        rest,
                        mesh: false,
                    });
                    parents.push(parent);
                    read_from.push(at);
                }
                Some(&known) if !disagreed => {
                    let first = rig.files[read_from[known]].as_str();
                    let file = file.as_str();
                    let under = |parent: &Option<String>| {
                        parent.clone().unwrap_or_else(|| "nothing".to_owned())
                    };
                    let found = if parents[known] != parent {
                        Some(format!(
                            "{} hangs under {} in {file:?}, under {} in {first:?}",
                            node.name,
                            under(&parent),
                            under(&parents[known]),
                        ))
                    } else {
                        rests_apart(&nodes[known].rest, &rest).map(|(turn, shift)| {
                            format!(
                                "{} rests {turn:.1}° and {:.1} mm apart in {file:?} and {first:?}",
                                node.name,
                                shift * 1000.0
                            )
                        })
                    };
                    if let Some(found) = found {
                        disagreed = true;
                        cx.error(
                            format!("/rig/files/{at}"),
                            "files disagree",
                            found,
                            "every file placing each joint under the same parent, resting alike \
                             within 2° and 2 mm",
                        );
                    }
                }
                Some(_) => {}
            }
        }
    }
    for (node, parent) in parents.iter().enumerate() {
        nodes[node].parent = parent
            .as_ref()
            .and_then(|parent| named.get(parent).copied());
    }
    (!disagreed && !nodes.is_empty()).then_some(NodeTree { nodes })
}

/// Hangs each top `hangs` names on its joint, standing where that joint stands, turned as it rests,
/// a top hung on a joint under another hung top after that one.
pub fn hang(tree: &mut NodeTree, hangs: &BTreeMap<JointName, JointName>, cx: &mut RowCheck) {
    let mut waiting: Vec<(&JointName, &JointName)> = Vec::new();
    for (top, onto) in hangs {
        let pointer = format!("/rig/hangs/{}", top.0);
        match (tree.named(&top.0), tree.named(&onto.0)) {
            (None, _) => cx.error(
                pointer,
                "unknown joint",
                format!("{top} is not a joint of the rig"),
                "a top joint of the rig's files",
            ),
            (Some(at), _) if tree.nodes[at].parent.is_some() => cx.error(
                pointer,
                "not a top",
                format!("{top} hangs under {} already", tree.parent_name(at)),
                "a joint with no parent in the rig's files",
            ),
            (_, None) => cx.error(
                pointer,
                "unknown joint",
                format!("{top} hangs on {onto}, not a joint of the rig"),
                "a joint of the rig",
            ),
            (Some(at), Some(on)) if tree.descends(on, at) => cx.error(
                pointer,
                "hung on itself",
                format!("{onto} lies under {top}"),
                "a joint outside what hangs",
            ),
            _ => waiting.push((top, onto)),
        }
    }
    while !waiting.is_empty() {
        let ready = waiting.iter().position(|(_, onto)| {
            let on = tree.named(&onto.0).expect("checked above");
            !waiting.iter().any(|(top, _)| {
                let top = tree.named(&top.0).expect("checked above");
                tree.descends(on, top)
            })
        });
        let Some(ready) = ready else {
            cx.error(
                "/rig/hangs",
                "hung in a ring",
                "tops each hung under another",
                "every top hung, in the end, under the body's",
            );
            return;
        };
        let (top, onto) = waiting.remove(ready);
        let at = tree.named(&top.0).expect("checked above");
        let on = tree.named(&onto.0).expect("checked above");
        let held = tree.worlds()[on];
        let rest = tree.nodes[at].rest;
        let standing = Mat4::from_scale_rotation_translation(
            rest.scale,
            rest.rotation,
            held.w_axis.truncate(),
        );
        let (scale, rotation, translation) =
            (held.inverse() * standing).to_scale_rotation_translation();
        tree.nodes[at].parent = Some(on);
        tree.nodes[at].rest = Rest {
            translation,
            rotation,
            scale,
        };
    }
}

/// How far apart two rests turn and stand, when further than the rig allows.
fn rests_apart(a: &Rest, b: &Rest) -> Option<(f32, f32)> {
    let turn = a.rotation.angle_between(b.rotation).to_degrees();
    let shift = a.translation.distance(b.translation);
    let scaled = !a.scale.abs_diff_eq(b.scale, 1.0e-3);
    (turn > REST_TURN.0 || shift > REST_SHIFT.0 || scaled).then_some((turn, shift))
}

/// Every joint the skeleton names is one of our rig's, every leg a chain down it, and no joint in
/// two groups.
fn check_joints(def: &SkeletonDef, tree: &NodeTree, cx: &mut RowCheck) {
    let joint = |name: &JointName| tree.named(&name.0);
    let unknown = |cx: &mut RowCheck, pointer: String, name: &JointName| {
        if joint(name).is_none() {
            cx.error(
                pointer,
                "unknown joint",
                format!("{name} is not a joint of our rig"),
                "a joint of our rig",
            );
        }
    };
    for (index, hand) in def.hands.iter().enumerate() {
        unknown(cx, format!("/hands/{index}/joint"), &hand.joint);
        let fingers = hand.clasp.iter().flat_map(|clasp| &clasp.fingers);
        for (at, finger) in fingers.enumerate() {
            unknown(cx, format!("/hands/{index}/clasp/fingers/{at}"), finger);
        }
    }
    for (index, source) in def.sources.iter().enumerate() {
        for (his, ours) in &source.pairs {
            unknown(cx, format!("/sources/{index}/pairs/{}", his.0), ours);
        }
    }
    let mut grouped: BTreeMap<usize, &GroupName> = BTreeMap::new();
    for (group, roots) in &def.groups {
        for (at, root) in roots.iter().enumerate() {
            let pointer = format!("/groups/{}/{at}", group.0);
            let Some(root) = joint(root) else {
                unknown(cx, pointer, root);
                continue;
            };
            for under in tree.joints().filter(|&node| tree.descends(node, root)) {
                if let Some(other) = grouped.insert(under, group)
                    && other != group
                {
                    cx.error(
                        pointer.clone(),
                        "groups overlap",
                        format!(
                            "joint {} is in groups {other} and {group}",
                            tree.nodes[under].name
                        ),
                        "every joint in one group at most",
                    );
                    break;
                }
            }
        }
    }
    for (index, layer) in def.layers.iter().enumerate() {
        for (pointer, name) in layer.joints() {
            unknown(cx, format!("/layers/{index}{pointer}"), name);
        }
        if let Layer::Feet { legs, .. } = layer {
            for (at, leg) in legs.iter().enumerate() {
                for pair in leg.joints.windows(2) {
                    let (Some(upper), Some(lower)) = (joint(&pair[0]), joint(&pair[1])) else {
                        continue;
                    };
                    if !tree.descends(lower, upper) || lower == upper {
                        cx.error(
                            format!("/layers/{index}/legs/{at}/joints"),
                            "not a chain",
                            format!(
                                "leg {:?}: {} is not a descendant of {}",
                                leg.name, pair[1], pair[0]
                            ),
                            "a leg's joints each beneath the one before",
                        );
                        break;
                    }
                }
            }
        }
    }
}

/// Every clip the skeleton names, each once, as it plays: found in its file, within the length
/// allowed, and moving only joints of ours or only joints of one source rig.
fn played_clips(
    def: &SkeletonDef,
    rig: &NodeTree,
    infos: &HashMap<String, GltfInfo>,
    cx: &mut RowCheck,
) -> Vec<PlayedClip> {
    let mut played: Vec<PlayedClip> = Vec::new();
    let mut seen: HashMap<&Clip, bool> = HashMap::new();
    for (pointer, clip) in def.clips() {
        if seen.contains_key(clip) {
            continue;
        }
        let found = play(def, rig, infos, clip);
        seen.insert(clip, found.is_ok());
        match found {
            Ok(Some(clip)) => {
                for (index, cues) in def.cues.iter().enumerate() {
                    if cues.clip == clip.clip {
                        check_cue_times(
                            &format!("/cues/{index}/cues"),
                            &cues.cues,
                            clip.length,
                            cx,
                        );
                    }
                }
                played.push(clip);
            }
            Ok(None) => {}
            Err((rule, found, expected)) => cx.error(pointer, rule, found, expected),
        }
    }
    played
}

type ClipProblem = (&'static str, String, String);

/// `clip` as the skeleton plays it, or why it can't; nothing for a file that didn't decode, which
/// its own error reports.
fn play(
    def: &SkeletonDef,
    rig: &NodeTree,
    infos: &HashMap<String, GltfInfo>,
    clip: &Clip,
) -> Result<Option<PlayedClip>, ClipProblem> {
    let file = clip.file().as_str();
    let Some(info) = infos.get(file) else {
        return Ok(None);
    };
    let names: Vec<&str> = info.clips.iter().map(|clip| clip.name.as_str()).collect();
    let animation = match clip.take() {
        Some(take) => names
            .iter()
            .position(|name| *name == take.as_str())
            .ok_or_else(|| {
                (
                    "unknown take",
                    format!("{file} holds no animation named {take}"),
                    format!("one of {}", names.join(", ")),
                )
            })?,
        None => match names.len() {
            1 => 0,
            0 => {
                return Err((
                    "no clip",
                    format!("{file} holds no animation"),
                    "a file holding a clip".to_owned(),
                ));
            }
            _ => {
                return Err((
                    "which take",
                    format!("{file} holds {} animations", names.len()),
                    format!("the take named, one of {}", names.join(", ")),
                ));
            }
        },
    };
    let animated = &info.clips[animation];
    let length = animated.length.millis();
    if length < MIN_CLIP || length > MAX_CLIP {
        return Err((
            "clip length",
            format!("{clip} plays {} ms", length.0),
            format!("{} ms to {} ms", MIN_CLIP.0, MAX_CLIP.0),
        ));
    }
    let moved: Vec<&str> = animated
        .targets
        .iter()
        .map(|&node| info.tree.nodes[node].name.as_str())
        .collect();
    let source = plays_on(def, rig, &moved).map_err(|stray| {
        (
            "plays on no rig",
            format!("{clip} moves {stray}"),
            "a clip moving only joints of our rig, or only joints of one source rig".to_owned(),
        )
    })?;
    Ok(Some(PlayedClip {
        clip: clip.clone(),
        animation,
        length: animated.length,
        source,
        cues: def.cues_of(clip).to_vec(),
        pace: def.pace(clip),
    }))
}

/// The rig a clip moving the joints named `moved` plays on: ours, or the source rig at the index
/// given, when every joint it moves is that rig's; otherwise a joint it moves that keeps it from
/// playing on any.
pub fn plays_on<'m>(
    def: &SkeletonDef,
    rig: &NodeTree,
    moved: &[&'m str],
) -> Result<Option<usize>, &'m str> {
    let ours = |name: &&str| rig.named(name).is_some();
    if moved.iter().all(ours) {
        return Ok(None);
    }
    let his = |source: &SourceRig, name: &str| source.joints.iter().any(|joint| joint.0 == name);
    if let Some(source) = def
        .sources
        .iter()
        .position(|source| moved.iter().all(|name| his(source, name)))
    {
        return Ok(Some(source));
    }
    let stray = moved
        .iter()
        .find(|name| !ours(name) && !def.sources.iter().any(|source| his(source, name)))
        .or_else(|| moved.iter().find(|name| !ours(name)))
        .copied()
        .unwrap_or_default();
    Err(stray)
}

/// No clip that plays as a hit lands more than once; one that marks no apex yet leaves the skeleton
/// unfinished.
fn check_apexes(def: &SkeletonDef, cx: &mut RowCheck) {
    for (pointer, set) in def.sets() {
        for (entry, clip) in set.attacks() {
            let clip = clip.clip();
            let apexes = def.cues_of(clip).iter().filter(|cue| cue.apex).count();
            if apexes > 1 {
                cx.error(
                    format!("{pointer}{entry}"),
                    "apex out of place",
                    format!("{clip} plays as an attack but has {apexes} apex cues"),
                    "one, where the hit lands",
                );
            }
        }
    }
}

/// The gaits a body of `build` in `stance` may cover ground in, each with its clip's pace.
fn gaits_of(def: &SkeletonDef, build: Option<&BuildName>, stance: Option<StanceId>) -> Vec<Paced> {
    Action::GAITS
        .into_iter()
        .filter(|&gait| gait.required() || def.names(build, stance, gait))
        .filter_map(|gait| {
            let (cell, _) = def.named(build, stance, &Motion::Action(gait), Pose::Idle)?;
            let pace = def.pace(cell.nth(0).clip())?;
            Some(Paced { gait, pace })
        })
        .collect()
}
