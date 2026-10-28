//! A rig and everything the models that wear it share: our rig, read from the joints of glTF files,
//! the source rigs whose clips play on it through pairs of their joints and ours, the hands an item
//! hangs from, the groups a clip may be kept to, what plays for each action, pose and move in each
//! stance and build, the cues and paces of those clips, and the layers that move a body where
//! keyframes are not enough.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::stance::StanceId;
use super::{Action, Motion, Pose};
use crate::core::assets::ModelRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::gltf::{Fit, Misfit};
use crate::core::content::{
    ContentFile, ContentId, ContentProblem, ModuleRule, ModuleSet, NamedContent, Nearby, RowCheck,
    serial,
};
use crate::core::geometry::{Degrees, Radians};
use crate::core::math::{Rgb, Rng};
use crate::core::render::world::layers::Gait;
use crate::core::time::{Hertz, Millis, Seconds};
use crate::core::units::{Meters, MetersPerSec};

pub type SkeletonId = ContentId<SkeletonDef>;

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident, $marker:literal) => {
        name_type!(@named [$(#[$doc])*] [#[schemars(extend($marker = true))]] $name);
    };
    ($(#[$doc:meta])* $name:ident) => {
        name_type!(@named [$(#[$doc])*] [] $name);
    };
    (@named [$(#[$doc:meta])*] [$(#[$marked:meta])*] $name:ident) => {
        $(#[$doc])*
        #[derive(
            JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord,
            Hash,
        )]
        #[serde(transparent)]
        $(#[$marked])*
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
    /// An animation of a glTF file, by its name there: what tells apart the clips of a file that
    /// holds several.
    TakeName
);
name_type!(
    /// A swing a skill asks for by name, which a skeleton's sets turn into a clip per stance.
    MoveName,
    "x-move"
);
name_type!(
    /// A gesture a body makes by name: a line said with an expression of the same name, or an emote
    /// a script plays, such as a wave.
    GestureName,
    "x-gesture"
);
name_type!(
    /// A named part of a body a clip may be kept to: the joints under the group's roots.
    GroupName,
    "x-group"
);
name_type!(
    /// A joint of a rig, by its name.
    JointName,
    "x-joint"
);
name_type!(
    /// A body type a skeleton's models are cut for, such as male or female: a part may carry a
    /// mesh for each, and a build may play clips of its own.
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
name_type!(
    /// A colour a body is painted in wherever a material's mask marks it, such as its skin, its hair
    /// or its metal.
    TintName,
    "x-tint"
);

/// A tint of a skeleton, and the colour a body is painted in unless its model, its appearance or
/// what it wears says otherwise.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonTint {
    pub name: TintName,
    pub colour: Rgb,
}

/// Colours by the tint each paints.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[schemars(extend("propertyNames" = { "type": "string", "x-tint": true }))]
pub struct Colours(pub BTreeMap<TintName, Rgb>);

impl Colours {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// These colours laid over `beneath`: a tint these name takes their colour, any other keeps
    /// the one beneath.
    pub fn over(&self, beneath: &Colours) -> Colours {
        let mut colours = beneath.clone();
        colours
            .0
            .extend(self.0.iter().map(|(tint, colour)| (tint.clone(), *colour)));
        colours
    }
}

/// A rig and everything shared by the models that wear it: the rigs its clips are made on, where an
/// item is held, how the body is grouped, what plays for each action, pose and move in each stance
/// and build, the cues and paces of those clips and the layers that move it beyond its clips.
#[derive(JsonSchema, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonDef {
    pub name: String,
    pub rig: Rig,
    /// Rigs other than ours whose clips play on ours, each through pairs of its joints and ours.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceRig>,
    /// The only places an item hangs; the first is the main hand, the second the off hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hands: Vec<Hand>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<GroupName, Vec<JointName>>,
    /// The body types its models are cut for; the first is the one every other plays the clips of
    /// where its own sets name none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<BuildName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<BodyRegion>,
    /// The places items are worn besides the hands, in the order their parts are laid over the
    /// body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<EquipmentSlot>,
    /// The colours a body's parts are painted in, in the order they are painted where their masks
    /// overlap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tints: Vec<SkeletonTint>,
    /// What every body plays, and with builds, what the first build plays.
    #[serde(default)]
    pub base: ClipSet,
    /// What a body plays in each stance over the base set: the columns every build has.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stances: Vec<StanceSet>,
    /// What each build but the first plays over the first's, by the build.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub build_sets: BTreeMap<BuildName, BuildSets>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<ClipCues>,
    /// How fast each clip a set plays to walk, run or sprint carries a body over the ground at scale
    /// 1: the speed its planted feet slide back under it, or for a clip whose feet a layer plants, or
    /// that has none, the speed it looks right at. A body plays the gait whose clip covers nearest its
    /// speed, as fast as covers it exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paces: Vec<ClipPace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<SkeletonModule>,
}

/// Our rig: every joint, every node that draws no mesh, of the glTF files it is read from, each
/// with its parent and its rest as those files place it, where they must all agree. A joint with no
/// parent there is a top: the body's top carries the body, and every other hangs on a joint of
/// ours.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    pub files: Vec<ModelRef>,
    /// The joint each top but the body's hangs on, by the top: it stands where that joint stands,
    /// turned as its files rest it, and moves with it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hangs: BTreeMap<JointName, JointName>,
}

/// A rig other than ours that clips are made on, such as a seller's: its joints, and the joint of
/// ours each of them moves. A joint of his paired with none moves nothing of ours.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRig {
    pub name: String,
    pub joints: Vec<JointName>,
    /// The joint of ours each of his moves, by his joint.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pairs: BTreeMap<JointName, JointName>,
}

impl SourceRig {
    /// Each joint of ours one of his moves, with that joint of his, by name.
    pub fn joint_pairs(&self) -> Vec<(String, String)> {
        self.pairs
            .iter()
            .map(|(his, ours)| (ours.0.clone(), his.0.clone()))
            .collect()
    }
}

/// A clip: the one animation of a glTF file in the game folder, or one of the several a file holds,
/// by its name there.
#[derive(
    JsonSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(untagged)]
pub enum Clip {
    File(ModelRef),
    Take { file: ModelRef, take: TakeName },
}

impl Clip {
    pub fn file(&self) -> &ModelRef {
        match self {
            Clip::File(file) | Clip::Take { file, .. } => file,
        }
    }

    pub fn take(&self) -> Option<&TakeName> {
        match self {
            Clip::File(_) => None,
            Clip::Take { take, .. } => Some(take),
        }
    }

    /// What a designer calls the clip: its file's name, and the take's after it when there is one.
    pub fn shown(&self) -> String {
        let file = self.file().as_str();
        let name = file.rsplit('/').next().unwrap_or(file);
        let name = name.rsplit_once('.').map_or(name, |(name, _)| name);
        match self.take() {
            Some(take) => format!("{name} · {take}"),
            None => name.to_owned(),
        }
    }
}

impl std::fmt::Display for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.take() {
            Some(take) => write!(f, "{} ({take})", self.file().as_str()),
            None => f.write_str(self.file().as_str()),
        }
    }
}

/// The moments that matter to the game on one clip, wherever it plays.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipCues {
    pub clip: Clip,
    pub cues: Vec<Cue>,
}

/// The ground one clip covers at scale 1, wherever it plays to walk, run or sprint.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipPace {
    pub clip: Clip,
    pub pace: MetersPerSec,
}

/// What a build plays where it plays its own: over its base and stance columns, the first build's.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSets {
    #[serde(default)]
    pub base: ClipSet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stances: Vec<StanceSet>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clasp: Option<Clasp>,
}

/// How a hand's fingers close on an item it holds: each joint from the root of each finger down,
/// as `clip` turns it at its first moment, over whatever else the body plays.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clasp {
    pub clip: Clip,
    /// The joint at the root of each finger.
    pub fingers: Vec<JointName>,
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
    pub clips: BTreeMap<Action, ClipCell>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub poses: BTreeMap<Pose, ClipCell>,
    /// What plays once as a body takes a pose, before the pose's own clip: sitting down.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub begins: BTreeMap<Pose, ClipCell>,
    /// What plays once as a body leaves a pose for idling: standing up.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ends: BTreeMap<Pose, ClipCell>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub moves: BTreeMap<MoveName, ClipCell>,
    /// What plays once for a gesture: a line said with the expression it is named after, or an
    /// emote.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gestures: BTreeMap<GestureName, ClipCell>,
}

/// What a set plays for one action, pose or move: a clip, or several taking turns, a looping one
/// moving on each time it plays through and one that plays once each time it is played.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClipCell {
    One(ClipRef),
    Several {
        clips: Vec<ClipRef>,
        #[serde(default, skip_serializing_if = "ClipOrder::is_random")]
        order: ClipOrder,
    },
}

/// How a cell's clips take turns: any but the one just played, or each in the order listed.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipOrder {
    #[default]
    Random,
    InTurn,
}

/// The cells a stance plays, over the base column.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StanceSet {
    pub stance: StanceId,
    pub set: ClipSet,
}

/// A clip played on the whole body, or kept to one group, played over the body's resting clip.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClipRef {
    Whole(Clip),
    Grouped { clip: Clip, only: GroupName },
}

impl ClipCell {
    pub fn clips(&self) -> &[ClipRef] {
        match self {
            ClipCell::One(clip) => std::slice::from_ref(clip),
            ClipCell::Several { clips, .. } => clips,
        }
    }

    pub fn order(&self) -> ClipOrder {
        match self {
            ClipCell::One(_) => ClipOrder::Random,
            ClipCell::Several { order, .. } => *order,
        }
    }

    /// The clip picked at `pick`, or the first when there is none there.
    pub fn nth(&self, pick: usize) -> &ClipRef {
        let clips = self.clips();
        clips.get(pick).unwrap_or(&clips[0])
    }

    /// The clip to pick after `last`, the one picked before it, if any.
    pub fn after(&self, last: Option<usize>, rng: &mut Rng) -> usize {
        let count = self.clips().len() as u32;
        match (self.order(), last) {
            _ if count <= 1 => 0,
            (ClipOrder::InTurn, None) => 0,
            (ClipOrder::InTurn, Some(last)) => (last + 1) % count as usize,
            (ClipOrder::Random, None) => rng.rand_range(0..count) as usize,
            (ClipOrder::Random, Some(last)) => {
                let other = rng.rand_range(0..count - 1) as usize;
                if other >= last { other + 1 } else { other }
            }
        }
    }

    /// Each clip the cell names, with where in the cell it names it, below `pointer`.
    fn entries(&self, pointer: String) -> Vec<(String, &ClipRef)> {
        match self {
            ClipCell::One(clip) => vec![(pointer, clip)],
            ClipCell::Several { clips, .. } => clips
                .iter()
                .enumerate()
                .map(|(index, clip)| (format!("{pointer}/clips/{index}"), clip))
                .collect(),
        }
    }
}

impl ClipOrder {
    fn is_random(&self) -> bool {
        *self == ClipOrder::Random
    }
}

impl ClipRef {
    pub fn clip(&self) -> &Clip {
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

/// A rig is drawn, and every clip played on it posed, for every body on it.
pub const MAX_JOINTS: usize = 64;
pub const MAX_HANDS: usize = 9;
pub const MAX_SOURCES: usize = 8;
pub const MIN_CLIP: Millis = Millis(100.0);
pub const MAX_CLIP: Millis = Millis(60_000.0);
pub const MAX_CUES: usize = 32;

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

/// The problem a misfit is, in `file` against the rig of `rig`.
pub fn misfit_problem(file: &str, rig: &str, misfit: &Misfit) -> ContentProblem {
    ContentProblem::new(
        "joint rests off",
        format!(
            "{file} binds {} {:.1}° and {:.1} mm from where {rig} rests it",
            misfit.joint,
            misfit.turn.0,
            misfit.shift.0 * 1000.0
        ),
        "at most 2° and 2 mm, so the skin moves with the rig",
    )
}

impl ContentFile for SkeletonDef {
    const TABLE: &'static str = "skeleton";

    fn fresh(near: &Nearby) -> Result<SkeletonDef, String> {
        Ok(SkeletonDef {
            name: near.name(),
            rig: Rig::default(),
            sources: Vec::new(),
            hands: Vec::new(),
            groups: BTreeMap::new(),
            builds: Vec::new(),
            regions: Vec::new(),
            slots: Vec::new(),
            tints: Vec::new(),
            base: ClipSet::default(),
            stances: Vec::new(),
            build_sets: BTreeMap::new(),
            cues: Vec::new(),
            paces: Vec::new(),
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
        check_sources(&self.sources, cx);
        check_names(cx, "builds", "build twice", &self.builds);
        check_names(cx, "regions", "region twice", &self.regions);
        check_names(cx, "slots", "slot twice", &self.slots);
        let mut tints = BTreeSet::new();
        for (index, tint) in self.tints.iter().enumerate() {
            cx.text(format!("/tints/{index}/name"), tint.name.as_str(), 1, 32);
            if !tints.insert(&tint.name) {
                cx.error(
                    format!("/tints/{index}/name"),
                    "tint twice",
                    tint.name.as_str(),
                    "every name listed once",
                );
            }
        }
        check_stances("/stances", &self.stances, cx);
        for (build, sets) in &self.build_sets {
            let pointer = format!("/build_sets/{}", build.0);
            if self.builds.first() == Some(build) {
                cx.error(
                    pointer.clone(),
                    "first build's own sets",
                    format!("sets for {build}, the first build"),
                    "the first build's cells in base and stances, which every other plays over",
                );
            } else if !self.builds.contains(build) {
                cx.error(
                    pointer.clone(),
                    "unknown build",
                    build.as_str(),
                    format!("one of the builds: {}", listed_names(&self.builds)),
                );
            }
            check_stances(&format!("{pointer}/stances"), &sets.stances, cx);
            for (index, set) in sets.stances.iter().enumerate() {
                if !self
                    .stances
                    .iter()
                    .any(|column| column.stance == set.stance)
                {
                    cx.error(
                        format!("{pointer}/stances/{index}/stance"),
                        "stance with no column",
                        format!("{build}'s cells for stance {}", set.stance.raw()),
                        "a stance the skeleton has a column for in stances",
                    );
                }
            }
        }
        for (pointer, set) in self.sets() {
            for (map, poses) in [
                ("poses", &set.poses),
                ("begins", &set.begins),
                ("ends", &set.ends),
            ] {
                if poses.contains_key(&Pose::Idle) {
                    cx.error(
                        format!("{pointer}/{map}/idle"),
                        "pose is an action",
                        "a clip for the idle pose",
                        "the idle action's clip, which idling plays already",
                    );
                }
            }
            for (entry, cell) in set.cells() {
                if cell.clips().is_empty() {
                    cx.error(
                        format!("{pointer}{entry}/clips"),
                        "no clips",
                        "an empty list",
                        "at least one clip to play",
                    );
                }
            }
            for (entry, clip) in set.entries() {
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
        let mut cued = BTreeSet::new();
        for (index, cues) in self.cues.iter().enumerate() {
            let pointer = format!("/cues/{index}");
            if !cued.insert(&cues.clip) {
                cx.error(
                    format!("{pointer}/clip"),
                    "clip twice",
                    cues.clip.to_string(),
                    "every clip's cues listed once",
                );
            }
            check_cues(&format!("{pointer}/cues"), &cues.cues, cx);
        }
        let mut paced = BTreeSet::new();
        for (index, paced_clip) in self.paces.iter().enumerate() {
            let pointer = format!("/paces/{index}");
            if !paced.insert(&paced_clip.clip) {
                cx.error(
                    format!("{pointer}/clip"),
                    "clip twice",
                    paced_clip.clip.to_string(),
                    "every clip's pace listed once",
                );
            }
            if paced_clip.pace.0.0 <= 0.0 {
                cx.error(
                    format!("{pointer}/pace"),
                    "no ground covered",
                    format!("{} m/s", paced_clip.pace.0.0),
                    "a speed above 0",
                );
            }
        }
        for (pointer, set) in self.sets() {
            for gait in Action::GAITS {
                let Some(cell) = set.clips.get(&gait) else {
                    continue;
                };
                for (entry, clip) in cell.entries(format!("{pointer}/clips/{}", gait.as_ref())) {
                    if self.pace(clip.clip()).is_none() {
                        cx.error(
                            entry,
                            "no pace",
                            format!("{} has no pace", clip.clip()),
                            "a pace for every clip played to walk, run or sprint",
                        );
                    }
                }
            }
        }
        for (index, layer) in self.layers.iter().enumerate() {
            check_layer(&format!("/layers/{index}"), layer, cx);
        }
    }
}

/// Every source rig named once, within the budget, each of its joints its own, every pair from one
/// of its joints.
fn check_sources(sources: &[SourceRig], cx: &mut RowCheck) {
    cx.count("/sources", sources.len(), 0, MAX_SOURCES, "source rigs");
    let mut names = BTreeSet::new();
    let mut owners: BTreeMap<&JointName, &str> = BTreeMap::new();
    for (index, source) in sources.iter().enumerate() {
        let pointer = format!("/sources/{index}");
        cx.text(format!("{pointer}/name"), &source.name, 1, 64);
        if !names.insert(source.name.as_str()) {
            cx.error(
                format!("{pointer}/name"),
                "source rig twice",
                format!("{:?}", source.name),
                "every source rig named once",
            );
        }
        for (at, joint) in source.joints.iter().enumerate() {
            if let Some(owner) = owners.insert(joint, &source.name) {
                cx.error(
                    format!("{pointer}/joints/{at}"),
                    "joint of two rigs",
                    format!("{joint} is a joint of {owner:?} and {:?}", source.name),
                    "every joint of a source rig its alone, so each clip plays on one rig",
                );
            }
        }
        for his in source.pairs.keys() {
            if !source.joints.contains(his) {
                cx.error(
                    format!("{pointer}/pairs/{}", his.0),
                    "unknown joint",
                    format!("a pair for {his}, not a joint of {:?}", source.name),
                    "a pair for each of the rig's own joints",
                );
            }
        }
    }
}

/// Every stance with one set of cells in `stances`.
fn check_stances(pointer: &str, stances: &[StanceSet], cx: &mut RowCheck) {
    let mut seen = BTreeSet::new();
    for (index, set) in stances.iter().enumerate() {
        if !seen.insert(set.stance) {
            cx.error(
                format!("{pointer}/{index}/stance"),
                "stance twice",
                format!("a second set for {}", set.stance.raw()),
                "one set per stance",
            );
        }
    }
}

fn listed_names<N: AsRef<str>>(names: &[N]) -> String {
    if names.is_empty() {
        return "none".to_owned();
    }
    names
        .iter()
        .map(AsRef::as_ref)
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
    /// Every cell of the set, with where in the set it is.
    pub fn cells(&self) -> impl Iterator<Item = (String, &ClipCell)> {
        let clips = self
            .clips
            .iter()
            .map(|(action, cell)| (format!("/clips/{}", action.as_ref()), cell));
        let poses = self
            .poses
            .iter()
            .map(|(pose, cell)| (format!("/poses/{}", pose.as_ref()), cell));
        let begins = self
            .begins
            .iter()
            .map(|(pose, cell)| (format!("/begins/{}", pose.as_ref()), cell));
        let ends = self
            .ends
            .iter()
            .map(|(pose, cell)| (format!("/ends/{}", pose.as_ref()), cell));
        let moves = self
            .moves
            .iter()
            .map(|(name, cell)| (format!("/moves/{}", name.0), cell));
        let gestures = self
            .gestures
            .iter()
            .map(|(name, cell)| (format!("/gestures/{}", name.0), cell));
        clips
            .chain(poses)
            .chain(begins)
            .chain(ends)
            .chain(moves)
            .chain(gestures)
    }

    /// Every clip the set names, each of a cell's clips apart, with where in the set it names it.
    pub fn entries(&self) -> impl Iterator<Item = (String, &ClipRef)> {
        self.cells()
            .flat_map(|(pointer, cell)| cell.entries(pointer))
    }

    /// The clips that play as a hit, each of a cell's clips apart: the attack and every move.
    pub fn attacks(&self) -> impl Iterator<Item = (String, &ClipRef)> {
        let attack = self
            .clips
            .get(&Action::Attack)
            .into_iter()
            .flat_map(|cell| cell.entries("/clips/attack".to_owned()));
        let moves = self
            .moves
            .iter()
            .flat_map(|(name, cell)| cell.entries(format!("/moves/{}", name.0)));
        attack.chain(moves)
    }
}

/// Which rung of the ladder named a clip: a stance's column, the base column, or a fallback for
/// what neither names: the attack for a move, idling for a gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    Stance,
    Base,
    Fallback,
}

impl SkeletonDef {
    /// The colour of each of its tints, where nothing laid over a body says otherwise.
    pub fn tint_colours(&self) -> Colours {
        Colours(
            self.tints
                .iter()
                .map(|tint| (tint.name.clone(), tint.colour))
                .collect(),
        )
    }

    /// The sets a body of `build` in `stance` looks through for a cell, in order, each with its
    /// rung: the build's own stance cells, the first build's, the build's own base cells, the first
    /// build's. A build without sets of its own, or none at all, plays the first build's.
    fn ladder(&self, build: Option<&BuildName>, stance: Option<StanceId>) -> Vec<(&ClipSet, Rung)> {
        let own = build
            .filter(|build| self.builds.first() != Some(*build))
            .and_then(|build| self.build_sets.get(build));
        [
            (
                own.and_then(|own| column(&own.stances, stance)),
                Rung::Stance,
            ),
            (column(&self.stances, stance), Rung::Stance),
            (own.map(|own| &own.base), Rung::Base),
            (Some(&self.base), Rung::Base),
        ]
        .into_iter()
        .filter_map(|(set, rung)| Some((set?, rung)))
        .collect()
    }

    /// The cell the sets name for `motion` in `stance` on a body of `build`, holding `pose` while
    /// idle, and where it was named: the first set of the ladder that names it; a move none names
    /// plays the attack, a sprint none names the run, a pose's beginning or end none names its
    /// idle. Every other action the base column names, as the loader holds it to; until it does,
    /// none is named.
    pub fn named(
        &self,
        build: Option<&BuildName>,
        stance: Option<StanceId>,
        motion: &Motion,
        pose: Pose,
    ) -> Option<(&ClipCell, Rung)> {
        let ladder = self.ladder(build, stance);
        let first = |cell: &dyn Fn(&ClipSet) -> Option<&ClipCell>| {
            ladder
                .iter()
                .find_map(|(set, rung)| Some((cell(set)?, *rung)))
        };
        let action = |action: Action| {
            let action = if action.required() || self.names(build, stance, action) {
                action
            } else {
                Action::Run
            };
            first(&|set| set.clips.get(&action))
        };
        match motion {
            Motion::Action(Action::Idle) => {
                first(&|set| set.poses.get(&pose)).or_else(|| action(Action::Idle))
            }
            Motion::Action(acted) => action(*acted),
            Motion::Begin(posed) => first(&|set| set.begins.get(posed))
                .or_else(|| self.named(build, stance, &Motion::Action(Action::Idle), *posed)),
            Motion::End(posed) => {
                first(&|set| set.ends.get(posed)).or_else(|| action(Action::Idle))
            }
            Motion::Gesture(name) => first(&|set| set.gestures.get(name))
                .or_else(|| Some((action(Action::Idle)?.0, Rung::Fallback))),
            Motion::Move(name) => first(&|set| set.moves.get(name))
                .or_else(|| Some((action(Action::Attack)?.0, Rung::Fallback))),
        }
    }

    /// Whether a set names the clip of a motion that plays only where one is named: taking or
    /// leaving a pose, which a body without one does at once, and a gesture, which it never makes.
    /// Any other motion always has a clip.
    pub fn plays(
        &self,
        build: Option<&BuildName>,
        stance: Option<StanceId>,
        motion: &Motion,
    ) -> bool {
        let names = |set: &ClipSet| match motion {
            Motion::Begin(pose) => set.begins.contains_key(pose),
            Motion::End(pose) => set.ends.contains_key(pose),
            Motion::Gesture(name) => set.gestures.contains_key(name),
            Motion::Action(_) | Motion::Move(_) => true,
        };
        self.ladder(build, stance).iter().any(|(set, _)| names(set))
    }

    /// Whether any set of the ladder names a clip for the action.
    pub fn names(
        &self,
        build: Option<&BuildName>,
        stance: Option<StanceId>,
        action: Action,
    ) -> bool {
        self.ladder(build, stance)
            .iter()
            .any(|(set, _)| set.clips.contains_key(&action))
    }

    /// Every set of cells, each with where it stands in the file: the base column, every stance
    /// column, and every build's own.
    pub fn sets(&self) -> impl Iterator<Item = (String, &ClipSet)> {
        let first = std::iter::once(("/base".to_owned(), &self.base)).chain(
            self.stances
                .iter()
                .enumerate()
                .map(|(index, stance)| (format!("/stances/{index}/set"), &stance.set)),
        );
        let builds = self.build_sets.iter().flat_map(|(build, sets)| {
            let pointer = format!("/build_sets/{}", build.0);
            std::iter::once((format!("{pointer}/base"), &sets.base)).chain(
                sets.stances.iter().enumerate().map(move |(index, stance)| {
                    (format!("{pointer}/stances/{index}/set"), &stance.set)
                }),
            )
        });
        first.chain(builds)
    }

    /// The first build's cells for `stance`, the column every build has.
    pub fn set(&self, stance: StanceId) -> Option<&ClipSet> {
        set_of(&self.stances, stance)
    }

    /// Every clip the skeleton names, with where it names it: in its sets, its hands' clasps, its
    /// cues and its paces.
    pub fn clips(&self) -> Vec<(String, &Clip)> {
        let mut clips: Vec<(String, &Clip)> = self
            .sets()
            .flat_map(|(pointer, set)| {
                set.entries().map(move |(entry, clip)| {
                    (
                        clip_pointer(&format!("{pointer}{entry}"), clip),
                        clip.clip(),
                    )
                })
            })
            .collect();
        for (index, hand) in self.hands.iter().enumerate() {
            if let Some(clasp) = &hand.clasp {
                clips.push((format!("/hands/{index}/clasp/clip"), &clasp.clip));
            }
        }
        for (index, cues) in self.cues.iter().enumerate() {
            clips.push((format!("/cues/{index}/clip"), &cues.clip));
        }
        for (index, paced) in self.paces.iter().enumerate() {
            clips.push((format!("/paces/{index}/clip"), &paced.clip));
        }
        clips
    }

    /// The cues the skeleton marks on `clip`.
    pub fn cues_of(&self, clip: &Clip) -> &[Cue] {
        self.cues
            .iter()
            .find(|cues| cues.clip == *clip)
            .map_or(&[], |cues| cues.cues.as_slice())
    }

    /// The ground `clip` covers at scale 1, when the skeleton says.
    pub fn pace(&self, clip: &Clip) -> Option<MetersPerSec> {
        self.paces
            .iter()
            .find(|paced| paced.clip == *clip)
            .map(|paced| paced.pace)
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

    pub fn gestures(&self) -> impl Iterator<Item = &GestureName> {
        self.sets().flat_map(|(_, set)| set.gestures.keys())
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

fn column(stances: &[StanceSet], stance: Option<StanceId>) -> Option<&ClipSet> {
    set_of(stances, stance?)
}

fn set_of(stances: &[StanceSet], stance: StanceId) -> Option<&ClipSet> {
    stances
        .iter()
        .find(|set| set.stance == stance)
        .map(|set| &set.set)
}

/// Where a cell's entry names its clip: the entry itself, or its `clip` when it keeps it to a
/// group.
fn clip_pointer(entry: &str, clip: &ClipRef) -> String {
    match clip {
        ClipRef::Whole(_) => entry.to_owned(),
        ClipRef::Grouped { .. } => format!("{entry}/clip"),
    }
}

/// The fit of a skin to a skeleton's rig, as one line per joint that does not fit.
pub fn fit_problems(fit: &Fit, file: &str, skeleton: &str) -> Vec<ContentProblem> {
    fit.misfits
        .iter()
        .map(|misfit| misfit_problem(file, skeleton, misfit))
        .collect()
}

/// Every move and gesture some skeleton's set declares: the only ones a script may ask for.
pub struct Declared {
    pub moves: BTreeSet<MoveName>,
    pub gestures: BTreeSet<GestureName>,
}
