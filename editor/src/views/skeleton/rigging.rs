//! What the skeleton's pages work from beyond its file: our rig as its files put it together, and
//! every clip of the game folder that plays on it, read from the folder as the editor holds it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use engine::core::assets::{AssetKind, ModelRef};
use engine::core::content::RowCheck;
use engine::core::content::check::RowError;
use engine::core::content::gltf::{self, GltfInfo, NodeTree};
use engine::core::time::Seconds;
use engine::systems::actor::rigging::{hang, plays_on, read_joints, read_rig};
use engine::systems::actor::skeleton::{
    Clip, JointName, MAX_CLIP, MIN_CLIP, Rig, SkeletonDef, SourceRig, TakeName,
};

use crate::project::ProjectFiles;
use crate::views::area::viewport::StageClip;

/// Every glTF binary of the game folder the editor has read, by its path, with the hash of the
/// bytes it was read from.
#[derive(Default)]
pub struct GltfCache(HashMap<String, (String, Option<Read>)>);

/// A glTF binary as the loader reads it, with its tree of nodes to stand a rig on.
#[derive(Clone)]
struct Read {
    info: Arc<GltfInfo>,
    tree: Arc<NodeTree>,
}

impl GltfCache {
    /// The file at `path` as the loader reads it, when it reads.
    pub fn info(&mut self, files: &ProjectFiles, path: &str) -> Option<Arc<GltfInfo>> {
        Some(self.read(files, path)?.info)
    }

    /// The tree of nodes of the file at `path`, when it reads.
    pub fn tree(&mut self, files: &ProjectFiles, path: &str) -> Option<Arc<NodeTree>> {
        Some(self.read(files, path)?.tree)
    }

    fn read(&mut self, files: &ProjectFiles, path: &str) -> Option<Read> {
        let hash = files.hash_of(path)?;
        if let Some((held, read)) = self.0.get(path)
            && held == hash
        {
            return read.clone();
        }
        let read = files
            .get(path)
            .and_then(|bytes| gltf::inspect(bytes).ok())
            .map(|info| Read {
                tree: Arc::new(info.tree.clone()),
                info: Arc::new(info),
            });
        self.0
            .insert(path.to_owned(), (hash.to_owned(), read.clone()));
        read
    }

    /// Every glTF binary of the game folder that reads, by its path.
    pub fn every(&mut self, files: &ProjectFiles) -> Vec<(String, Arc<GltfInfo>)> {
        let index = files.index();
        index
            .media
            .iter()
            .filter(|path| AssetKind::of(path) == Some(AssetKind::Model))
            .filter_map(|path| Some((path.clone(), self.info(files, path)?)))
            .collect()
    }
}

/// A clip of the game folder that plays on a skeleton: an animation of a file, how long it plays,
/// and the source rig it is made on when it is not ours.
#[derive(Clone, Debug, PartialEq)]
pub struct Playable {
    pub clip: Clip,
    pub animation: usize,
    pub length: Seconds,
    pub source: Option<usize>,
}

impl Playable {
    /// The clip as a stage plays it on our rig.
    pub fn stage_clip(&self, def: &SkeletonDef) -> StageClip {
        StageClip {
            file: self.clip.file().as_str().to_owned(),
            animation: self.animation,
            pairs: self
                .source
                .and_then(|source| def.sources.get(source))
                .map(SourceRig::joint_pairs),
        }
    }
}

/// A glTF binary of the game folder holding joints: nodes that draw no mesh.
#[derive(Clone, Debug)]
pub struct JointFile {
    pub path: String,
    pub joints: Vec<String>,
}

/// Our rig as the skeleton's files put it together, or why they don't, and the clips that play on
/// it.
pub struct Rigging {
    pub rig: Option<Arc<NodeTree>>,
    /// Every joint the rig's files hold, each top hanging under nothing, where they agree.
    pub joints: Option<NodeTree>,
    pub problems: Vec<RowError>,
    pub clips: Vec<Playable>,
    /// Clips that move joints of ours or of a source rig, but also one no rig has: the clip and
    /// that joint.
    pub strays: Vec<(Clip, String)>,
}

impl Rigging {
    pub fn playable(&self, clip: &Clip) -> Option<&Playable> {
        self.clips.iter().find(|playable| playable.clip == *clip)
    }
}

/// What a skeleton's rigging was worked out from, so it is worked out again only when that changes.
#[derive(Clone, PartialEq)]
pub struct RiggedFrom {
    pub revision: u64,
    pub rig: Rig,
    pub sources: Vec<SourceRig>,
}

/// The rigging of `def` from the files as they stand.
pub fn rig_up(def: &SkeletonDef, files: &ProjectFiles, cache: &mut GltfCache) -> Rigging {
    let infos = infos_of(&def.rig.files, files, cache);
    let mut cx = RowCheck::default();
    let joints = read_joints(&def.rig, &infos, &mut RowCheck::default());
    let rig = read_rig(&def.rig, &infos, &mut cx).map(Arc::new);
    let mut clips = Vec::new();
    let mut strays = Vec::new();
    if let Some(rig) = &rig {
        for (path, info) in cache.every(files) {
            let file = ModelRef::new(path);
            for (animation, take) in info.clips.iter().enumerate() {
                let length = take.length.millis();
                if length < MIN_CLIP || length > MAX_CLIP {
                    continue;
                }
                let clip = match info.clips.len() {
                    1 => Clip::File(file.clone()),
                    _ => Clip::Take {
                        file: file.clone(),
                        take: TakeName::new(take.name.clone()),
                    },
                };
                let moved: Vec<&str> = take
                    .targets
                    .iter()
                    .map(|&node| info.tree.nodes[node].name.as_str())
                    .collect();
                match plays_on(def, rig, &moved) {
                    Ok(source) => clips.push(Playable {
                        clip,
                        animation,
                        length: take.length,
                        source,
                    }),
                    Err(stray) => {
                        let known = |name: &&str| {
                            rig.named(name).is_some()
                                || def.sources.iter().any(|source| {
                                    source.joints.iter().any(|joint| joint.0 == *name)
                                })
                        };
                        if moved.iter().any(known) {
                            strays.push((clip, stray.to_owned()));
                        }
                    }
                }
            }
        }
    }
    Rigging {
        rig,
        joints,
        problems: cx.into_errors(),
        clips,
        strays,
    }
}

/// Every glTF binary of the game folder that holds joints, with their names.
pub fn joint_files(files: &ProjectFiles, cache: &mut GltfCache) -> Vec<JointFile> {
    cache
        .every(files)
        .into_iter()
        .filter_map(|(path, info)| {
            let joints: Vec<String> = info
                .tree
                .joints()
                .map(|joint| info.tree.nodes[joint].name.clone())
                .collect();
            (!joints.is_empty()).then_some(JointFile { path, joints })
        })
        .collect()
}

/// Where each top of the rig `files` put together, but the body's, hangs: on the joint that stands
/// the joints under it closest to where the skins of those files bind them.
pub fn guess_hangs(
    files: &[ModelRef],
    project: &ProjectFiles,
    cache: &mut GltfCache,
) -> BTreeMap<JointName, JointName> {
    let infos = infos_of(files, project, cache);
    let rig = Rig {
        files: files.to_vec(),
        hangs: BTreeMap::new(),
    };
    let Some(tree) = read_joints(&rig, &infos, &mut RowCheck::default()) else {
        return BTreeMap::new();
    };
    let tops: Vec<usize> = (0..tree.nodes.len())
        .filter(|&node| tree.nodes[node].parent.is_none())
        .collect();
    let Some((&body, others)) = tops
        .iter()
        .max_by_key(|&&top| {
            tree.joints()
                .filter(|&joint| tree.descends(joint, top))
                .count()
        })
        .map(|body| (body, tops.iter().filter(|&&top| top != *body)))
    else {
        return BTreeMap::new();
    };
    let mut hangs = BTreeMap::new();
    for &top in others {
        let under: Vec<&str> = tree
            .joints()
            .filter(|&joint| tree.descends(joint, top))
            .map(|joint| tree.nodes[joint].name.as_str())
            .collect();
        let binding: Vec<&GltfInfo> = infos
            .values()
            .filter(|info| {
                info.binds
                    .iter()
                    .any(|(node, _)| under.contains(&info.tree.nodes[*node].name.as_str()))
            })
            .collect();
        let candidates = tree.joints().filter(|&joint| tree.descends(joint, body));
        let best = candidates
            .map(|on| {
                let mut hung = tree.clone();
                let hangs = BTreeMap::from([(
                    JointName::new(tree.nodes[top].name.clone()),
                    JointName::new(tree.nodes[on].name.clone()),
                )]);
                hang(&mut hung, &hangs, &mut RowCheck::default());
                let off: f32 = binding
                    .iter()
                    .map(|info| off_binds(&hung, info, &under))
                    .sum();
                (on, off)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((on, _)) = best {
            hangs.insert(
                JointName::new(tree.nodes[top].name.clone()),
                JointName::new(tree.nodes[on].name.clone()),
            );
        }
    }
    hangs
}

/// How far, in metres all told, the joints `under` stand from where `info`'s skin binds them on
/// `rig`, measured in the scene's frame.
fn off_binds(rig: &NodeTree, info: &GltfInfo, under: &[&str]) -> f32 {
    let worlds = rig.worlds();
    info.binds
        .iter()
        .filter_map(|(node, inverse_bind)| {
            let name = info.tree.nodes[*node].name.as_str();
            if !under.contains(&name) {
                return None;
            }
            let joint = rig.named(name)?;
            let bound = inverse_bind.inverse().w_axis.truncate();
            Some(worlds[joint].w_axis.truncate().distance(bound))
        })
        .sum()
}

fn infos_of(
    files: &[ModelRef],
    project: &ProjectFiles,
    cache: &mut GltfCache,
) -> HashMap<String, GltfInfo> {
    files
        .iter()
        .filter_map(|file| {
            let info = cache.info(project, file.as_str())?;
            Some((file.as_str().to_owned(), (*info).clone()))
        })
        .collect()
}
