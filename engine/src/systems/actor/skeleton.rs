//! A rig and everything the models that wear it share: the joint tree at rest, the libraries of
//! clips made on it, the hands an item hangs from, the groups a clip may be kept to, what plays for
//! each action, pose and move in each stance, the cues on those clips, and the layers that move a
//! body where keyframes are not enough.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::stance::StanceId;
use super::{Action, Motion, Pose};
use crate::core::assets::{FileSize, ModelRef};
use crate::core::audio::playback::SfxId;
use crate::core::content::gltf::{Fit, GltfInfo, Misfit, NodeTree};
use crate::core::content::{
    Bundle, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, ModuleRule, ModuleSet,
    NamedContent, Nearby, PartSpec, Parts, RowCheck, Shape, serial,
};
use crate::core::geometry::{Degrees, Radians};
use crate::core::render::world::layers::Gait;
use crate::core::time::{Hertz, Millis, Seconds};
use crate::core::units::{Meters, MetersPerSec};

pub type SkeletonId = ContentId<SkeletonDef>;

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident, $marker:literal) => {
        $(#[$doc])*
        #[derive(
            JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord,
            Hash,
        )]
        #[serde(transparent)]
        #[schemars(extend($marker = true))]
        pub struct $name(pub String);

        impl $name {
            pub fn new(name: impl Into<String>) -> $name {
                $name(name.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(
    /// An animation of a skeleton's libraries or a model's own glTF, named as it is there.
    ClipName,
    "x-clip"
);
name_type!(
    /// A swing a skill asks for by name, which a skeleton's sets turn into a clip per stance.
    MoveName,
    "x-move"
);
name_type!(
    /// A named part of a body a clip may be kept to: the joints under the group's roots.
    GroupName,
    "x-group"
);
name_type!(
    /// A node of a rig, by its name in the rig's glTF.
    JointName,
    "x-joint"
);
name_type!(
    /// A body type a skeleton's models are cut for, such as male or female: a part may carry a
    /// mesh for each.
    BuildName,
    "x-build"
);
name_type!(
    /// A place on a body a part goes: a part covers some, and may hide others.
    BodyRegion,
    "x-region"
);
name_type!(
    /// A place on a body an item is worn besides its hands, such as its chest or its head.
    EquipmentSlot,
    "x-slot"
);

/// A rig and everything shared by the models that wear it: where an item is held, how the body is
/// grouped, what plays for each action, pose and move in each stance, the cues on those clips and
/// the layers that move it beyond its clips.
#[derive(JsonSchema, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonDef {
    pub name: String,
    #[serde(skip)]
    pub rig: ModelRef,
    #[serde(skip)]
    pub libraries: Vec<Library>,
    /// The only places an item hangs; the first is the main hand, the second the off hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hands: Vec<Hand>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<GroupName, Vec<JointName>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<BuildName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<BodyRegion>,
    /// The places items are worn besides the hands, in the order their parts are laid over the
    /// body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<EquipmentSlot>,
    pub base: ClipSet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stances: Vec<StanceSet>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cues: BTreeMap<ClipName, Vec<Cue>>,
    /// How fast each clip a set plays to walk, run or sprint carries a body over the ground at scale
    /// 1: the speed its planted feet slide back under it, or for a clip whose feet a layer plants, or
    /// that has none, the speed it looks right at. A body plays the gait whose clip covers nearest its
    /// speed, as fast as covers it exactly.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paces: BTreeMap<ClipName, MetersPerSec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<SkeletonModule>,
}

/// A glTF of clips made on the rig, kept in the bundle at `clips/<key>.glb`.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub key: String,
    pub file: ModelRef,
}

/// A joint an item hangs from, turned and moved so +Y runs along its palm.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hand {
    pub name: String,
    pub joint: JointName,
    #[serde(default, skip_serializing_if = "LocalOffset::is_zero")]
    pub offset: LocalOffset,
    #[serde(default, skip_serializing_if = "LocalTurn::is_zero")]
    pub turn: LocalTurn,
}

/// A displacement in a joint's own frame: metres along its x, y and z.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalOffset(pub [Meters; 3]);

/// A turn in a joint's own frame: degrees about its x, y and z, applied in that order.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalTurn(pub [Degrees; 3]);

impl LocalOffset {
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|axis| axis.0 == 0.0)
    }

    pub fn array(self) -> [f32; 3] {
        self.0.map(|axis| axis.0)
    }
}

impl LocalTurn {
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(Degrees::is_zero)
    }

    pub fn array(self) -> [f32; 3] {
        self.0.map(|axis| axis.0)
    }
}

/// What plays for each engine action, each pose and each move: a set only lists what differs from
/// the set beneath it.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSet {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub clips: BTreeMap<Action, ClipRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub poses: BTreeMap<Pose, ClipRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub moves: BTreeMap<MoveName, ClipRef>,
}

/// The set a stance plays, over the base set.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StanceSet {
    pub stance: StanceId,
    pub set: ClipSet,
}

/// A clip of the skeleton's libraries or the model's own glTF by name, or kept to one group, played
/// over the body's resting clip.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClipRef {
    Whole(ClipName),
    Grouped { clip: ClipName, only: GroupName },
}

impl ClipRef {
    pub fn clip(&self) -> &ClipName {
        match self {
            ClipRef::Whole(clip) | ClipRef::Grouped { clip, .. } => clip,
        }
    }

    pub fn only(&self) -> Option<&GroupName> {
        match self {
            ClipRef::Whole(_) => None,
            ClipRef::Grouped { only, .. } => Some(only),
        }
    }
}

/// A moment on a clip that matters to the game: a foot coming down, the hit landing, a sound.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub at: Millis,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub step: bool,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub apex: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SfxId>,
}

/// Motion a client draws over a body's clips where keyframes are not enough, while the actions it
/// lists play: never a hit, a reach or a position the game reasons about.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layer {
    /// Plants each leg's foot where it touched the ground and steps it in an arc once the body has
    /// moved a stride on; the body rides the planted feet.
    Feet {
        legs: Vec<Leg>,
        gait: Gait,
        stride: Meters,
        lift: Meters,
        #[serde(default = "calm")]
        during: Vec<Action>,
    },
    /// Springs the joints under `from` behind the body; with `follow`, each trails the one before
    /// along the path the body went.
    Chain {
        from: JointName,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        follow: bool,
        #[serde(default = "Layer::stiffness")]
        stiffness: f32,
        #[serde(default = "Layer::drag")]
        drag: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        gravity: f32,
        #[serde(default = "calm")]
        during: Vec<Action>,
    },
    /// Sways the joints under `from` in a wave travelling down the chain, growing toward its end.
    Wave {
        from: JointName,
        amplitude: Radians,
        hz: Hertz,
        lag: f32,
        #[serde(default = "calm")]
        during: Vec<Action>,
    },
    /// Squashes a joint by the body's speed and on each step, keeping its volume.
    Squash {
        joint: JointName,
        #[serde(default, skip_serializing_if = "is_zero")]
        by_speed: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        on_step: f32,
        ease: Seconds,
        #[serde(default = "calm")]
        during: Vec<Action>,
    },
}

/// A leg of a feet layer: its joints from the hip down to the foot, a chain down the tree.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub name: String,
    pub joints: Vec<JointName>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkeletonModule {
    /// Walks ignore blocked cells; wild ones keep out of safe zones.
    Airborne,
}

impl Layer {
    fn stiffness() -> f32 {
        0.5
    }

    fn drag() -> f32 {
        0.4
    }

    pub fn during(&self) -> &[Action] {
        match self {
            Layer::Feet { during, .. }
            | Layer::Chain { during, .. }
            | Layer::Wave { during, .. }
            | Layer::Squash { during, .. } => during,
        }
    }

    /// Every joint the layer names, with where in its entry it names it.
    pub fn joints(&self) -> Vec<(String, &JointName)> {
        match self {
            Layer::Feet { legs, .. } => legs
                .iter()
                .enumerate()
                .flat_map(|(leg, entry)| {
                    entry
                        .joints
                        .iter()
                        .enumerate()
                        .map(move |(at, joint)| (format!("/legs/{leg}/joints/{at}"), joint))
                })
                .collect(),
            Layer::Chain { from, .. } | Layer::Wave { from, .. } => {
                vec![("/from".to_owned(), from)]
            }
            Layer::Squash { joint, .. } => vec![("/joint".to_owned(), joint)],
        }
    }
}

/// The actions a layer runs during when its entry names none: all but a hit and a death, which a
/// keyed clip always plays alone.
fn calm() -> Vec<Action> {
    vec![
        Action::Idle,
        Action::Walk,
        Action::Run,
        Action::Sprint,
        Action::Talk,
    ]
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

pub const RIG_GLB: PartSpec = PartSpec::File {
    name: "rig",
    exts: &["glb"],
};

pub const CLIP_LIBRARIES: PartSpec = PartSpec::Keyed {
    folder: "clips",
    exts: &["glb"],
};

/// A rig is drawn, and every clip of its libraries posed, for every body on it.
pub const MAX_JOINTS: usize = 64;
pub const MAX_HANDS: usize = 9;
/// Every client that sees one body on a skeleton downloads each of its libraries.
pub const MAX_LIBRARY_BYTES: FileSize = FileSize::kib(4 * 1024);
pub const MAX_LIBRARIES_BYTES: FileSize = FileSize::kib(8 * 1024);
pub const MIN_CLIP: Millis = Millis(100.0);
pub const MAX_CLIP: Millis = Millis(60_000.0);
pub const MAX_CUES: usize = 32;

impl Bundle for SkeletonDef {
    const PARTS: &'static [PartSpec] = &[RIG_GLB, CLIP_LIBRARIES];

    fn read_parts(&mut self, parts: &mut Parts) {
        let mut cx = RowCheck::default();
        let rig = parts.required(RIG_GLB).and_then(|glb| {
            self.rig = ModelRef::new(parts.path(glb));
            parts.gltf_info(glb)
        });
        if let Some(rig) = &rig {
            check_rig(self, rig, &mut cx);
        }
        let mut carried: BTreeMap<&str, &str> = BTreeMap::new();
        let mut total = 0;
        let keyed = parts.keyed(CLIP_LIBRARIES);
        let mut infos = Vec::new();
        for &(key, file) in &keyed {
            self.libraries.push(Library {
                key: key.to_owned(),
                file: ModelRef::new(parts.path(file)),
            });
            infos.push((file, parts.weight(file), parts.gltf_info(file)));
        }
        for (file, weight, info) in &infos {
            let weight = weight.unwrap_or(FileSize(0));
            total += weight.0;
            if weight > MAX_LIBRARY_BYTES {
                parts.error(
                    file,
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        "library too heavy",
                        format!("{file} weighs {weight}"),
                        format!("at most {MAX_LIBRARY_BYTES}"),
                    ),
                );
            }
            let Some(info) = info else {
                continue;
            };
            if let Some(rig) = &rig {
                for misfit in &crate::core::content::gltf::fit(&rig.tree, &info.tree).misfits {
                    parts.error(
                        file,
                        ErrorPlace::WholeFile,
                        misfit_problem(file, "rig.glb", misfit),
                    );
                }
            }
            for clip in &info.clips {
                if clip.name.is_empty() {
                    parts.error(
                        file,
                        ErrorPlace::WholeFile,
                        ContentProblem::new(
                            "unnamed clip",
                            format!("{file} carries a clip without a name"),
                            "every clip named, as sets name it",
                        ),
                    );
                    continue;
                }
                let length = clip.length.millis();
                if length < MIN_CLIP || length > MAX_CLIP {
                    parts.error(
                        file,
                        ErrorPlace::WholeFile,
                        ContentProblem::new(
                            "clip length",
                            format!("{} plays {} ms", clip.name, length.0),
                            format!("{} ms to {} ms", MIN_CLIP.0, MAX_CLIP.0),
                        ),
                    );
                }
                if let Some(earlier) = carried.insert(&clip.name, file) {
                    parts.error(
                        file,
                        ErrorPlace::WholeFile,
                        ContentProblem::new(
                            "clip twice",
                            format!("clip {} is in {earlier} and {file}", clip.name),
                            "one library per name",
                        ),
                    );
                }
            }
        }
        let lengths: BTreeMap<&str, Seconds> = infos
            .iter()
            .filter_map(|(_, _, info)| info.as_ref())
            .flat_map(|info| {
                info.clips
                    .iter()
                    .map(|clip| (clip.name.as_str(), clip.length))
            })
            .collect();
        for (clip, cues) in &self.cues {
            let pointer = format!("/cues/{}", clip.0);
            match lengths.get(clip.as_str()) {
                Some(&length) => check_cue_times(&pointer, cues, length, &mut cx),
                None => cx.error(
                    pointer,
                    "unknown clip",
                    format!("cues for {clip}, which no library carries"),
                    "a clip of clips/*.glb",
                ),
            }
        }
        let total = FileSize(total);
        if total > MAX_LIBRARIES_BYTES {
            parts.error(
                "",
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "libraries too heavy",
                    format!("libraries weigh {total}"),
                    format!("at most {MAX_LIBRARIES_BYTES}"),
                ),
            );
        }
        parts.config_errors(cx);
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn check_cue_times(pointer: &str, cues: &[Cue], length: Seconds, cx: &mut RowCheck) {
    let length = length.millis();
    for (index, cue) in cues.iter().enumerate() {
        if cue.at > length {
            cx.error(
                format!("{pointer}/{index}/at"),
                "cue past the clip",
                format!("{} ms into a clip of {} ms", cue.at.0, length.0),
                "a moment within the clip",
            );
        }
    }
}

/// What a skeleton's config must be to its rig: within the joint budget, every joint it names
/// there, every leg a chain down the tree, and no joint in two groups.
fn check_rig(def: &SkeletonDef, rig: &GltfInfo, cx: &mut RowCheck) {
    let tree = &rig.tree;
    let joints = rig.joints();
    if joints > MAX_JOINTS {
        cx.error(
            "",
            "too many joints",
            format!("rig.glb has {joints} joints"),
            format!("at most {MAX_JOINTS}"),
        );
    }
    let joint = |name: &JointName| {
        tree.joints()
            .find(|&index| tree.nodes[index].name == name.0)
    };
    for (index, hand) in def.hands.iter().enumerate() {
        if joint(&hand.joint).is_none() {
            cx.error(
                format!("/hands/{index}/joint"),
                "unknown joint",
                format!(
                    "hand {:?} names joint {}, not in rig.glb",
                    hand.name, hand.joint
                ),
                "a joint of rig.glb",
            );
        }
    }
    let mut grouped: BTreeMap<usize, &GroupName> = BTreeMap::new();
    for (group, roots) in &def.groups {
        for (at, root) in roots.iter().enumerate() {
            let pointer = format!("/groups/{}/{at}", group.0);
            let Some(root) = joint(root) else {
                cx.error(
                    pointer,
                    "unknown joint",
                    format!("group {group} names joint {root}, not in rig.glb"),
                    "a joint of rig.glb",
                );
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
            if joint(name).is_none() {
                cx.error(
                    format!("/layers/{index}{pointer}"),
                    "unknown joint",
                    format!("{name} is not in rig.glb"),
                    "a joint of rig.glb",
                );
            }
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

/// The problem a misfit is, in `file` against the rig of `rig`.
pub fn misfit_problem(file: &str, rig: &str, misfit: &Misfit) -> ContentProblem {
    match misfit {
        Misfit::Missing { joint } => ContentProblem::new(
            "joint missing",
            format!("{file}: no joint {joint}"),
            format!("every joint of {rig}, at its path"),
        ),
        Misfit::Misparented {
            joint,
            parent,
            expected,
        } => ContentProblem::new(
            "joint misparented",
            format!("{file}: joint {joint} has parent {parent}"),
            format!("{rig} expects {expected}"),
        ),
        Misfit::RestsOff { joint, turn, shift } => ContentProblem::new(
            "joint rests off",
            format!(
                "{file}: joint {joint} rests {:.1}° and {:.1} mm from {rig}",
                turn.0,
                shift.0 * 1000.0
            ),
            "at most 2° and 2 mm, so library clips play true",
        ),
    }
}

impl ContentFile for SkeletonDef {
    const TABLE: &'static str = "skeleton";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<SkeletonDef, String> {
        Ok(SkeletonDef {
            name: near.name(),
            rig: ModelRef::default(),
            libraries: Vec::new(),
            hands: Vec::new(),
            groups: BTreeMap::new(),
            builds: Vec::new(),
            regions: Vec::new(),
            slots: Vec::new(),
            base: ClipSet {
                clips: <Action as strum::VariantArray>::VARIANTS
                    .iter()
                    .filter(|action| action.required())
                    .map(|&action| (action, ClipRef::Whole(ClipName::new(action.as_ref()))))
                    .collect(),
                ..ClipSet::default()
            },
            stances: Vec::new(),
            cues: BTreeMap::from([(
                ClipName::new(Action::Attack.as_ref()),
                vec![Cue {
                    at: Millis(400.0),
                    step: false,
                    apex: true,
                    sound: None,
                }],
            )]),
            paces: BTreeMap::from([
                (
                    ClipName::new(Action::Walk.as_ref()),
                    MetersPerSec(Meters(1.0)),
                ),
                (
                    ClipName::new(Action::Run.as_ref()),
                    MetersPerSec(Meters(3.0)),
                ),
            ]),
            layers: Vec::new(),
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 64);
        if self.hands.len() > MAX_HANDS {
            cx.error(
                "/hands",
                "too many hands",
                format!("{} hands", self.hands.len()),
                format!("at most {MAX_HANDS}"),
            );
        }
        let mut names = BTreeSet::new();
        for (index, hand) in self.hands.iter().enumerate() {
            cx.text(format!("/hands/{index}/name"), &hand.name, 1, 32);
            if !names.insert(hand.name.as_str()) {
                cx.error(
                    format!("/hands/{index}/name"),
                    "hand twice",
                    format!("{:?}", hand.name),
                    "every hand named once",
                );
            }
        }
        check_names(cx, "builds", "build twice", &self.builds);
        check_names(cx, "regions", "region twice", &self.regions);
        check_names(cx, "slots", "slot twice", &self.slots);
        for &action in <Action as strum::VariantArray>::VARIANTS {
            if action.required() && !self.base.clips.contains_key(&action) {
                cx.error(
                    "/base/clips",
                    "missing clip",
                    format!("base set has no {} clip", action.as_ref()),
                    format!("every engine action needs one: {}", listed_actions()),
                );
            }
        }
        let mut stances = BTreeSet::new();
        for (index, set) in self.stances.iter().enumerate() {
            if !stances.insert(set.stance) {
                cx.error(
                    format!("/stances/{index}/stance"),
                    "stance twice",
                    format!("a second set for {}", set.stance.raw()),
                    "one set per stance",
                );
            }
        }
        for (pointer, set) in self.sets() {
            if set.poses.contains_key(&Pose::Idle) {
                cx.error(
                    format!("{pointer}/poses/idle"),
                    "pose is an action",
                    "a clip for the idle pose",
                    "the idle action's clip, which idling plays already",
                );
            }
            for (entry, clip) in set.entries() {
                cx.text(format!("{pointer}{entry}"), clip.clip().as_str(), 1, 64);
                if let Some(group) = clip.only()
                    && !self.groups.contains_key(group)
                {
                    cx.error(
                        format!("{pointer}{entry}/only"),
                        "unknown group",
                        format!("{group}"),
                        format!("one of the skeleton's groups: {}", self.group_names()),
                    );
                }
            }
        }
        for (clip, cues) in &self.cues {
            check_cues(&format!("/cues/{}", clip.0), cues, cx);
        }
        for (pointer, set) in self.sets() {
            for gait in Action::GAITS {
                let Some(clip) = set.clips.get(&gait) else {
                    continue;
                };
                if !self.paces.contains_key(clip.clip()) {
                    cx.error(
                        format!("{pointer}/clips/{}", gait.as_ref()),
                        "no pace",
                        format!("{} has no pace", clip.clip()),
                        "a pace for every clip played to walk, run or sprint",
                    );
                }
            }
        }
        for (clip, pace) in &self.paces {
            if pace.0.0 <= 0.0 {
                cx.error(
                    format!("/paces/{}", clip.0),
                    "no ground covered",
                    format!("{} m/s", pace.0.0),
                    "a speed above 0",
                );
            }
        }
        for (index, layer) in self.layers.iter().enumerate() {
            check_layer(&format!("/layers/{index}"), layer, cx);
        }
    }
}

fn listed_actions() -> String {
    <Action as strum::VariantArray>::VARIANTS
        .iter()
        .map(|action| action.as_ref())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a clip's cues must be on their own: a few, none before the clip begins.
fn check_names<N: AsRef<str> + Ord>(
    cx: &mut RowCheck,
    list: &str,
    twice: &'static str,
    names: &[N],
) {
    let mut seen = BTreeSet::new();
    for (index, name) in names.iter().enumerate() {
        cx.text(format!("/{list}/{index}"), name.as_ref(), 1, 32);
        if !seen.insert(name) {
            cx.error(
                format!("/{list}/{index}"),
                twice,
                name.as_ref(),
                "every name listed once",
            );
        }
    }
}

pub fn check_cues(pointer: &str, cues: &[Cue], cx: &mut RowCheck) {
    cx.count(pointer, cues.len(), 0, MAX_CUES, "cues");
    for (index, cue) in cues.iter().enumerate() {
        if cue.at.0 < 0.0 {
            cx.error(
                format!("{pointer}/{index}/at"),
                "cue before the clip",
                format!("{} ms", cue.at.0),
                "a moment from 0 ms",
            );
        }
    }
}

fn check_layer(pointer: &str, layer: &Layer, cx: &mut RowCheck) {
    for (index, action) in layer.during().iter().enumerate() {
        if matches!(action, Action::Attack | Action::Dead) {
            cx.error(
                format!("{pointer}/during/{index}"),
                "keyed clip only",
                format!("during lists {}", action.as_ref()),
                "a hit and a death always play their clip alone",
            );
        }
    }
    match layer {
        Layer::Feet {
            legs, stride, lift, ..
        } => {
            cx.count(format!("{pointer}/legs"), legs.len(), 1, 8, "legs");
            for (index, leg) in legs.iter().enumerate() {
                cx.count(
                    format!("{pointer}/legs/{index}/joints"),
                    leg.joints.len(),
                    2,
                    6,
                    "joints",
                );
            }
            cx.range(format!("{pointer}/stride"), stride.0, 0.05, 4.0, "m");
            cx.range(format!("{pointer}/lift"), lift.0, 0.0, 1.0, "m");
        }
        Layer::Chain {
            stiffness,
            drag,
            gravity,
            ..
        } => {
            cx.range(
                format!("{pointer}/stiffness"),
                *stiffness,
                0.0,
                4.0,
                "a stiffness",
            );
            cx.range(format!("{pointer}/drag"), *drag, 0.0, 1.0, "a drag");
            cx.range(
                format!("{pointer}/gravity"),
                *gravity,
                0.0,
                10.0,
                "a gravity",
            );
        }
        Layer::Wave {
            amplitude, hz, lag, ..
        } => {
            cx.range(
                format!("{pointer}/amplitude"),
                amplitude.0,
                0.0,
                1.5,
                "radians",
            );
            cx.range(format!("{pointer}/hz"), hz.0, 0.05, 10.0, "Hz");
            cx.range(format!("{pointer}/lag"), *lag, 0.0, 4.0, "a lag");
        }
        Layer::Squash {
            by_speed,
            on_step,
            ease,
            ..
        } => {
            cx.range(
                format!("{pointer}/by_speed"),
                *by_speed,
                0.0,
                1.0,
                "a squash",
            );
            cx.range(format!("{pointer}/on_step"), *on_step, 0.0, 1.0, "a squash");
            cx.range(format!("{pointer}/ease"), ease.0, 0.0, 2.0, "s");
        }
    }
}

impl NamedContent for SkeletonDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for SkeletonDef {
    type Module = SkeletonModule;

    fn modules(&self) -> &[SkeletonModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<SkeletonModule> {
        &mut self.modules
    }

    fn rule(_: &SkeletonModule) -> ModuleRule {
        ModuleRule::AtMostOne
    }
}

impl ClipSet {
    /// Every clip the set names, with where in the set it names it.
    pub fn entries(&self) -> impl Iterator<Item = (String, &ClipRef)> {
        let clips = self
            .clips
            .iter()
            .map(|(action, clip)| (format!("/clips/{}", action.as_ref()), clip));
        let poses = self
            .poses
            .iter()
            .map(|(pose, clip)| (format!("/poses/{}", pose.as_ref()), clip));
        let moves = self
            .moves
            .iter()
            .map(|(name, clip)| (format!("/moves/{}", name.0), clip));
        clips.chain(poses).chain(moves)
    }

    /// The clips that play as a hit: the attack and every move.
    pub fn attacks(&self) -> impl Iterator<Item = (String, &ClipRef)> {
        let attack = self
            .clips
            .get(&Action::Attack)
            .map(|clip| ("/clips/attack".to_owned(), clip));
        let moves = self
            .moves
            .iter()
            .map(|(name, clip)| (format!("/moves/{}", name.0), clip));
        attack.into_iter().chain(moves)
    }
}

/// Which rung of the ladder named a clip: the stance's set, the base set, or a fallback to the
/// attack for a move neither names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    Stance,
    Base,
    Fallback,
}

impl SkeletonDef {
    /// The clip the sets name for `motion` in `stance`, holding `pose` while idle, and where it was
    /// named: the stance's set names it, else the base set; a move neither names plays the stance's
    /// attack, else the base's; a sprint neither names plays the run. The base set names every other
    /// action, as the loader holds it to.
    pub fn named(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> (&ClipRef, Rung) {
        let base = &self.base;
        let set = stance.and_then(|stance| self.set(stance));
        let action = |action: Action| -> (&ClipRef, Rung) {
            let action = if action.required() || self.names(stance, action) {
                action
            } else {
                Action::Run
            };
            match set.and_then(|set| set.clips.get(&action)) {
                Some(clip) => (clip, Rung::Stance),
                None => (&base.clips[&action], Rung::Base),
            }
        };
        match motion {
            Motion::Action(Action::Idle) => {
                match (
                    set.and_then(|set| set.poses.get(&pose)),
                    base.poses.get(&pose),
                ) {
                    (Some(clip), _) => (clip, Rung::Stance),
                    (None, Some(clip)) => (clip, Rung::Base),
                    (None, None) => action(Action::Idle),
                }
            }
            Motion::Action(acted) => action(*acted),
            Motion::Move(name) => {
                match (
                    set.and_then(|set| set.moves.get(name)),
                    base.moves.get(name),
                ) {
                    (Some(clip), _) => (clip, Rung::Stance),
                    (None, Some(clip)) => (clip, Rung::Base),
                    (None, None) => (action(Action::Attack).0, Rung::Fallback),
                }
            }
        }
    }

    /// The gaits a body in `stance` may cover ground in, each with its clip's pace.
    fn gaits(&self, stance: Option<StanceId>) -> Vec<Paced> {
        Action::GAITS
            .into_iter()
            .filter(|&gait| gait.required() || self.names(stance, gait))
            .filter_map(|gait| {
                let (clip, _) = self.named(stance, &Motion::Action(gait), Pose::Idle);
                let pace = *self.paces.get(clip.clip())?;
                Some(Paced { gait, pace })
            })
            .collect()
    }

    /// Whether the stance's set or the base set names a clip for the action.
    pub fn names(&self, stance: Option<StanceId>, action: Action) -> bool {
        stance
            .and_then(|stance| self.set(stance))
            .is_some_and(|set| set.clips.contains_key(&action))
            || self.base.clips.contains_key(&action)
    }

    /// The base set and every stance's set, each with where it stands in the config.
    pub fn sets(&self) -> impl Iterator<Item = (String, &ClipSet)> {
        std::iter::once(("/base".to_owned(), &self.base)).chain(
            self.stances
                .iter()
                .enumerate()
                .map(|(index, stance)| (format!("/stances/{index}/set"), &stance.set)),
        )
    }

    pub fn set(&self, stance: StanceId) -> Option<&ClipSet> {
        self.stances
            .iter()
            .find(|set| set.stance == stance)
            .map(|set| &set.set)
    }

    pub fn airborne(&self) -> bool {
        self.modules
            .iter()
            .any(|module| matches!(module, SkeletonModule::Airborne))
    }

    /// Every move any of the skeleton's sets declares.
    pub fn moves(&self) -> impl Iterator<Item = &MoveName> {
        self.sets().flat_map(|(_, set)| set.moves.keys())
    }

    fn group_names(&self) -> String {
        if self.groups.is_empty() {
            return "none".to_owned();
        }
        self.groups
            .keys()
            .map(|group| group.0.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The fit of a candidate's tree to a skeleton's rig, as one line per joint that does not fit.
pub fn fit_problems(fit: &Fit, file: &str, skeleton: &str) -> Vec<ContentProblem> {
    fit.misfits
        .iter()
        .map(|misfit| misfit_problem(file, skeleton, misfit))
        .collect()
}

/// Every move some skeleton's set declares: the only moves a script may ask for.
pub struct DeclaredMoves(pub BTreeSet<MoveName>);

/// The rig a skeleton's bodies are fitted against, and where each clip of its libraries is.
pub struct BuiltSkeleton {
    pub def: SkeletonDef,
    pub rig: NodeTree,
    /// Every clip of the libraries by name: which library carries it and how long it plays.
    pub clips: BTreeMap<ClipName, LibraryClip>,
    gaits: BTreeMap<Option<StanceId>, Vec<Paced>>,
}

/// A gait a body may cover ground in, with the ground its clip covers at scale 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paced {
    pub gait: Action,
    pub pace: MetersPerSec,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LibraryClip {
    /// The library's place among the skeleton's libraries.
    pub library: usize,
    /// The clip's place among the library's animations.
    pub index: usize,
    pub length: Seconds,
}

impl BuiltSkeleton {
    pub fn new(def: &SkeletonDef, rig: &GltfInfo, libraries: &[Option<&GltfInfo>]) -> Self {
        let clips = libraries
            .iter()
            .enumerate()
            .filter_map(|(library, info)| Some((library, (*info)?)))
            .flat_map(|(library, info)| {
                info.clips.iter().enumerate().map(move |(index, clip)| {
                    (
                        ClipName::new(clip.name.clone()),
                        LibraryClip {
                            library,
                            index,
                            length: clip.length,
                        },
                    )
                })
            })
            .collect();
        let gaits = std::iter::once(None)
            .chain(def.stances.iter().map(|set| Some(set.stance)))
            .map(|stance| (stance, def.gaits(stance)))
            .collect();
        BuiltSkeleton {
            def: def.clone(),
            rig: rig.tree.clone(),
            clips,
            gaits,
        }
    }

    /// The gaits a body in `stance` may cover ground in.
    pub fn gaits(&self, stance: Option<StanceId>) -> &[Paced] {
        self.gaits
            .get(&stance)
            .or_else(|| self.gaits.get(&None))
            .map_or(&[], Vec::as_slice)
    }
}
