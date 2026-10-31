//! A model: a body put together from parts on one skeleton, in one of its builds, with the capsule
//! it is hit in. What it plays and how it times its hits are its skeleton's.

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::appearance::{self, BodyLook, LaidPart};
use super::material;
use super::part::{MAX_PART_BYTES, ModelPartDef, ModelPartId};
use super::playing::Motion;
use super::rigging::BuiltSkeleton;
use super::skeleton::{
    self, BuildName, Clip, ClipCell, Colours, Cue, Declared, GroupName, Hand, Naming, SkeletonDef,
    SkeletonId,
};
use super::stance::StanceId;
use super::{Action, Pose};
use crate::core::assets::FileSize;
use crate::core::content::gltf::{self, GltfInfo};
use crate::core::content::media::GltfInfos;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, FileHashes, Loading, ModuleRule, ModuleSet,
    Nearby, RowCheck,
};
use crate::core::geometry::Capsule;
use crate::core::render::world::body::{Figure, Skin};
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::core::units::{Meters, MetersPerSec};

pub type ModelId = ContentId<ModelDef>;

/// A body of `parts` on `skeleton`, each drawn as cut for `build` and painted in `colours` over the
/// skeleton's, hit in `hitbox`.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    pub skeleton: SkeletonId,
    /// One of the skeleton's builds, which a skeleton declaring any must be given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildName>,
    pub hitbox: Capsule,
    pub parts: Vec<ModelPartId>,
    #[serde(default, skip_serializing_if = "Colours::is_empty")]
    pub colours: Colours,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModelModule>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelModule {
    Scale { factor: f32 },
}

// A character is downloaded by every client that sees it, and drawn many times a frame.
pub const MAX_CHARACTER_TRIANGLES: usize = 8000;

/// The radii a model's hitbox may take.
pub const HITBOX_RADII: RangeInclusive<f32> = 0.1..=4.0;
/// The heights a model's hitbox may take.
pub const HITBOX_HEIGHTS: RangeInclusive<f32> = 0.2..=8.0;

impl ContentFile for ModelDef {
    const TABLE: &'static str = "model";

    fn fresh(near: &Nearby) -> Result<ModelDef, String> {
        let skeleton: SkeletonId = near
            .file_where("a finished skeleton", |skeleton: &SkeletonDef| {
                skeleton.unfinished().is_empty()
            })?;
        let part = near.file_where(
            "a part with a mesh on that skeleton",
            |part: &ModelPartDef| part.skeleton == skeleton && part.meshes().next().is_some(),
        )?;
        Ok(ModelDef {
            skeleton,
            build: near.read(skeleton)?.builds.first().cloned(),
            hitbox: Capsule {
                radius: Meters(0.4),
                height: Meters(1.8),
            },
            parts: vec![part],
            colours: Colours::default(),
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        let radius = &HITBOX_RADII;
        let height = &HITBOX_HEIGHTS;
        cx.range(
            "/hitbox/radius",
            self.hitbox.radius.0,
            *radius.start(),
            *radius.end(),
            "m",
        );
        cx.range(
            "/hitbox/height",
            self.hitbox.height.0,
            *height.start(),
            *height.end(),
            "m",
        );
        cx.count("/parts", self.parts.len(), 1, 64, "parts");
        let mut seen = HashSet::new();
        for (index, part) in self.parts.iter().enumerate() {
            if !seen.insert(part) {
                cx.error(
                    format!("/parts/{index}"),
                    "part twice",
                    part.raw(),
                    "every part listed once",
                );
            }
        }
        for (index, module) in self.modules.iter().enumerate() {
            let ModelModule::Scale { factor } = module;
            cx.range(
                format!("/modules/{index}/factor"),
                *factor,
                0.1,
                10.0,
                "a scale",
            );
        }
    }
}

impl ModuleSet for ModelDef {
    type Module = ModelModule;

    fn modules(&self) -> &[ModelModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<ModelModule> {
        &mut self.modules
    }

    fn rule(_: &ModelModule) -> ModuleRule {
        ModuleRule::AtMostOne
    }
}

impl ModelDef {
    /// Its own parts, each painted in its colours.
    pub fn laid_parts(&self) -> impl Iterator<Item = LaidPart> + '_ {
        self.parts.iter().map(|&part| LaidPart {
            part,
            colours: self.colours.clone(),
        })
    }

    pub fn scale(&self) -> f32 {
        self.modules
            .iter()
            .map(|module| match module {
                ModelModule::Scale { factor } => *factor,
            })
            .next()
            .unwrap_or(1.0)
    }
}

pub struct Models(pub Vec<Option<ActorModel>>);

/// Every skeleton that loaded, by its id: its rig and every clip it plays.
pub struct Skeletons(pub Vec<Option<Arc<BuiltSkeleton>>>);

impl Skeletons {
    /// The skeleton, when it loaded.
    pub fn loaded(content: &Content, skeleton: SkeletonId) -> Option<&Arc<BuiltSkeleton>> {
        content.has_built::<Skeletons>().then_some(())?;
        content
            .built::<Skeletons>()
            .0
            .get(skeleton.index())
            .and_then(Option::as_ref)
    }
}

/// Builds every skeleton, holds each part's meshes to its skeleton's rig and builds, and builds
/// every model from parts of its skeleton.
pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let infos = content.built::<GltfInfos>();
    let hashes = content.built::<FileHashes>();
    let skeletons = build_skeletons(loading, &content, infos);
    let loaded_skeletons: HashSet<SkeletonId> = loading
        .rows::<SkeletonDef>(&content)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for (id, part) in loading.rows::<ModelPartDef>(&content) {
        if !loaded_skeletons.contains(&part.skeleton) {
            loading.fail(id);
            continue;
        }
        let mut check = RowCheck::default();
        let skeleton_def = part.skeleton.get(&content);
        if let Some(skeleton) = skeletons.get(part.skeleton.index()).cloned().flatten() {
            let skeleton_path = part.skeleton.path(&content);
            check_meshes(part, &skeleton, skeleton_path, infos, hashes, &mut check);
        }
        check_part(&content, part, skeleton_def, &mut check);
        if !check.is_empty() {
            loading.row_errors(id, check);
            loading.fail(id);
        }
    }
    let built_parts: HashSet<ModelPartId> = loading
        .rows::<ModelPartDef>(&content)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut models: Vec<Option<ActorModel>> = content.ids::<ModelDef>().map(|_| None).collect();
    for (id, def) in loading.rows::<ModelDef>(&content) {
        // A skeleton or part that did not load was refused at its own file already.
        if !loaded_skeletons.contains(&def.skeleton)
            || !def.parts.iter().all(|part| built_parts.contains(part))
        {
            loading.fail(id);
            continue;
        }
        let mut check = RowCheck::default();
        check_finished(&content, def.skeleton, &mut check);
        check_meshed(&content, &def.parts, "/parts", &mut check);
        let skeleton = skeletons.get(def.skeleton.index()).cloned().flatten();
        if let Some(skeleton) = &skeleton {
            check_model(&content, def, skeleton, infos, &mut check);
        }
        match skeleton {
            Some(skeleton) if check.is_empty() => {
                models[id.index()] = Some(ActorModel::new(&content, def, skeleton));
            }
            _ => {
                loading.row_errors(id, check);
                loading.fail(id);
            }
        }
    }
    let skeleton_defs = loading.rows::<SkeletonDef>(&content);
    loading.built(Declared {
        moves: skeleton_defs
            .iter()
            .flat_map(|(_, def)| def.moves().cloned())
            .collect(),
        gestures: skeleton_defs
            .iter()
            .flat_map(|(_, def)| def.gestures().cloned())
            .collect(),
    });
    loading.built(Skeletons(skeletons));
    loading.built(Models(models));
}

fn build_skeletons(
    loading: &mut Loading,
    content: &Content,
    infos: &GltfInfos,
) -> Vec<Option<Arc<BuiltSkeleton>>> {
    let mut built: Vec<Option<Arc<BuiltSkeleton>>> =
        content.ids::<SkeletonDef>().map(|_| None).collect();
    for (id, def) in loading.rows::<SkeletonDef>(content) {
        match BuiltSkeleton::build(def, &infos.0) {
            Ok(skeleton) => built[id.index()] = skeleton.map(Arc::new),
            Err(check) => {
                loading.row_errors(id, check);
                loading.fail(id);
            }
        }
    }
    built
}

/// What a part's meshes must be to its skeleton: each cut for one of its builds, light enough to
/// download, within the joint budget, and binding only joints of its rig where the rig rests them.
fn check_meshes(
    part: &ModelPartDef,
    skeleton: &BuiltSkeleton,
    skeleton_path: &str,
    infos: &GltfInfos,
    hashes: &FileHashes,
    cx: &mut RowCheck,
) {
    for (build, mesh) in part.meshes() {
        let pointer = match build {
            Some(build) => format!("/builds/{}", build.0),
            None => "/mesh".to_owned(),
        };
        if let Some(build) = build
            && !skeleton.def.builds.contains(build)
        {
            cx.error(
                pointer.clone(),
                "unknown build",
                format!("a mesh for build {build}"),
                expected_build(&skeleton.def.builds, skeleton_path),
            );
        }
        if let Some(hashed) = hashes.0.get(mesh.as_str())
            && FileSize(hashed.size as usize) > MAX_PART_BYTES
        {
            cx.error(
                pointer.clone(),
                "mesh too heavy",
                format!("{mesh:?} weighs {}", FileSize(hashed.size as usize)),
                format!("at most {MAX_PART_BYTES}"),
            );
        }
        let Some(info) = infos.0.get(mesh.as_str()) else {
            continue;
        };
        if info.joints() > skeleton::MAX_JOINTS {
            cx.error(
                pointer.clone(),
                "too many joints",
                format!("{mesh:?} has {} joints", info.joints()),
                format!("at most {}", skeleton::MAX_JOINTS),
            );
        }
        for problem in fit_report(skeleton, skeleton_path, mesh.as_str(), info).problems {
            cx.problem(pointer.clone(), problem);
        }
    }
}

/// What a body needs of the skeleton it is made on, which may be unfinished while it is made.
fn check_finished(content: &Content, skeleton: SkeletonId, cx: &mut RowCheck) {
    let skeleton_path = skeleton.path(content);
    for lacking in skeleton.get(content).unfinished() {
        cx.error(
            "/skeleton",
            "unfinished skeleton",
            format!("{skeleton_path} {lacking}"),
            "a skeleton a body can wear: a rig, a base clip for every action a body must play, an \
             apex on every clip played as a hit, and a pace for every gait",
        );
    }
}

/// Every part laid on a body has a mesh, which a part may lack while it is made. `at` is where the
/// parts are listed.
pub fn check_meshed(content: &Content, parts: &[ModelPartId], at: &str, cx: &mut RowCheck) {
    for (index, part) in parts.iter().enumerate() {
        if part.get(content).meshes().next().is_none() {
            cx.error(
                format!("{at}/{index}"),
                "unfinished part",
                format!("{} has no mesh", part.path(content)),
                "a part with a mesh for every build, or one for each build it is cut for",
            );
        }
    }
}

/// Every region a part covers or hides is one of its skeleton's, and its material is made on its
/// skeleton.
fn check_part(content: &Content, part: &ModelPartDef, skeleton: &SkeletonDef, cx: &mut RowCheck) {
    if let Some(material) = part.material {
        let made_on = material.get(content).skeleton;
        if made_on != part.skeleton {
            cx.error(
                "/material",
                "material of another skeleton",
                format!(
                    "{} is made on {}",
                    material.path(content),
                    made_on.path(content)
                ),
                format!("a material made on {}", part.skeleton.path(content)),
            );
        }
    }
    for (field, regions) in [("covers", &part.covers), ("hides", &part.hides)] {
        for (index, region) in regions.iter().enumerate() {
            if !skeleton.regions.contains(region) {
                cx.error(
                    format!("/{field}/{index}"),
                    "unknown region",
                    region.as_str(),
                    expected_name("regions", &skeleton.regions, part.skeleton.path(content)),
                );
            }
        }
    }
}

/// What a model must be to its skeleton and parts: every part made on its skeleton, a build of the
/// skeleton when it declares any, and the meshes of the parts it draws within the triangle budget.
fn check_model(
    content: &Content,
    def: &ModelDef,
    skeleton: &BuiltSkeleton,
    infos: &GltfInfos,
    cx: &mut RowCheck,
) {
    let skeleton_path = def.skeleton.path(content);
    let builds = &skeleton.def.builds;
    match &def.build {
        Some(build) if !builds.contains(build) => cx.error(
            "/build",
            "unknown build",
            build.as_str(),
            expected_build(builds, skeleton_path),
        ),
        None if !builds.is_empty() => cx.error(
            "/build",
            "missing build",
            "no build",
            format!("one of the builds of {skeleton_path}: {}", listed(builds)),
        ),
        _ => {}
    }
    material::check_colours(content, def.skeleton, &def.colours, "/colours", cx);
    for (index, &part) in def.parts.iter().enumerate() {
        let part_def = part.get(content);
        if part_def.skeleton != def.skeleton {
            cx.error(
                format!("/parts/{index}"),
                "part of another skeleton",
                format!(
                    "{} is made on {}",
                    part.path(content),
                    part_def.skeleton.path(content)
                ),
                format!("a part made on {skeleton_path}"),
            );
        }
    }
    let triangles = appearance::laid(content, def.laid_parts())
        .parts
        .iter()
        .filter_map(|laid| laid.part.get(content).mesh_for(def.build.as_ref()))
        .filter_map(|mesh| infos.0.get(mesh.as_str()))
        .map(|info| info.triangles)
        .sum();
    cx.range("/parts", triangles, 0, MAX_CHARACTER_TRIANGLES, "triangles");
}

fn expected_build(builds: &[BuildName], skeleton_path: &str) -> String {
    expected_name("builds", builds, skeleton_path)
}

/// What a name must be to a list of its skeleton's.
pub fn expected_name<N: AsRef<str>>(list: &str, names: &[N], skeleton_path: &str) -> String {
    if names.is_empty() {
        format!("none: {skeleton_path} declares no {list}")
    } else {
        format!("one of the {list} of {skeleton_path}: {}", listed(names))
    }
}

/// How a mesh's joints fit the skeleton at `skeleton_path`, with the loader's word on each joint
/// that does not.
pub struct FitReport {
    pub fit: gltf::Fit,
    pub problems: Vec<ContentProblem>,
}

/// How the skins of the mesh `file` carries as `info` fit the skeleton at `skeleton_path`.
pub fn fit_report(
    skeleton: &BuiltSkeleton,
    skeleton_path: &str,
    file: &str,
    info: &GltfInfo,
) -> FitReport {
    let fit = gltf::fit_skin(&skeleton.rig, info);
    let skeleton_file = format!(
        "the rig of {}",
        crate::core::content::name_of(skeleton_path)
    );
    let problems = fit
        .misfits
        .iter()
        .map(|misfit| skeleton::misfit_problem(file, &skeleton_file, misfit))
        .collect();
    FitReport { fit, problems }
}

pub struct ActorModel {
    def: ModelDef,
    skeleton: Arc<BuiltSkeleton>,
    /// The skins the model draws bare: those of the parts left showing once its own are laid, each
    /// as cut for its build.
    skins: Vec<Skin>,
}

/// What plays for a motion: the clip, the group it is kept to, its place among the skeleton's
/// clips, how long it plays once, and its cues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved<'a> {
    pub clip: &'a Clip,
    pub only: Option<&'a GroupName>,
    pub played: usize,
    pub length: Seconds,
    pub cues: &'a [Cue],
    /// The ground the clip covers at scale 1, when it is played to walk, run or sprint.
    pub pace: Option<MetersPerSec>,
}

impl ActorModel {
    pub fn get(content: &Content, model: ModelId) -> &ActorModel {
        content
            .built::<Models>()
            .0
            .get(model.index())
            .and_then(Option::as_ref)
            .unwrap_or_else(|| panic!("model {} failed to load", model.path(content)))
    }

    /// The model, when it loaded: what checks across files ask, which run on a game that may hold
    /// refused rows.
    pub fn loaded(content: &Content, model: ModelId) -> Option<&ActorModel> {
        content.has_built::<Models>().then_some(())?;
        content
            .built::<Models>()
            .0
            .get(model.index())
            .and_then(Option::as_ref)
    }

    fn new(content: &Content, def: &ModelDef, skeleton: Arc<BuiltSkeleton>) -> ActorModel {
        let look = appearance::laid(content, def.laid_parts());
        let skins = skins(content, def, &skeleton.def, &look);
        ActorModel {
            def: def.clone(),
            skeleton,
            skins,
        }
    }

    pub fn def(&self) -> &ModelDef {
        &self.def
    }

    pub fn hitbox(&self) -> Capsule {
        self.def.hitbox
    }

    /// What the model is drawn as bare: its skeleton's rig and the skins of its own parts.
    pub fn body(&self) -> Figure {
        Figure {
            rig: self.skeleton.rig.clone(),
            skins: self.skins.clone(),
        }
    }

    /// What a body of the model looking as `look` is drawn as: its skeleton's rig and the meshes
    /// of the look's parts as cut for the model's build, each painted in its colours.
    pub fn dressed(&self, content: &Content, look: &BodyLook) -> Figure {
        Figure {
            rig: self.skeleton.rig.clone(),
            skins: skins(content, &self.def, &self.skeleton.def, look),
        }
    }

    pub fn skeleton_id(&self) -> SkeletonId {
        self.def.skeleton
    }

    pub fn skeleton(&self) -> &Arc<BuiltSkeleton> {
        &self.skeleton
    }

    pub fn build(&self) -> Option<&BuildName> {
        self.def.build.as_ref()
    }

    /// Whether its build is given clips of its own, over those of its skeleton's first build.
    pub fn build_has_clips(&self) -> bool {
        self.build()
            .is_some_and(|build| self.skeleton.def.build_has_clips(build))
    }

    pub fn hands(&self) -> &[Hand] {
        &self.skeleton.def.hands
    }

    pub fn airborne(&self) -> bool {
        self.skeleton.def.airborne()
    }

    pub fn scale(&self) -> f32 {
        self.def.scale()
    }

    /// Where among its skeleton's clips is the clip a turntable stands the model in for `motion` in
    /// `stance` and `pose`.
    pub fn standing_clip(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> usize {
        self.resolve(stance, motion, pose, 0).played
    }

    /// Whether the model's sets name the clip of `motion` in `stance`, for a motion that plays only
    /// where one is named.
    pub fn plays(&self, stance: Option<StanceId>, motion: &Motion) -> bool {
        self.skeleton.def.plays(self.build(), stance, motion)
    }

    /// Whether the model's sets name a clip for `action` in `stance`.
    pub fn names(&self, stance: Option<StanceId>, action: Action) -> bool {
        self.skeleton.def.names(self.build(), stance, action)
    }

    /// The cell that plays `motion` in `stance`, holding `pose` while idle.
    pub fn cell(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> &ClipCell {
        self.named(stance, motion, pose).0
    }

    /// Where the sets named the cell that plays `motion` in `stance`, holding `pose` while idle.
    pub fn naming(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> Naming {
        self.named(stance, motion, pose).1
    }

    /// What plays as the clip picked at `pick` among `motion`'s in `stance`, holding `pose` while idle:
    /// the stance's set names it, else the base set; a move neither names plays the stance's
    /// attack, else the base's.
    pub fn resolve(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        pose: Pose,
        pick: usize,
    ) -> Resolved<'_> {
        let clip = self.cell(stance, motion, pose).nth(pick);
        let played = self
            .skeleton
            .played(clip.clip())
            .expect("a skeleton that built plays every clip it names");
        let found = &self.skeleton.clips[played];
        Resolved {
            clip: &found.clip,
            only: clip.only(),
            played,
            length: found.length,
            cues: &found.cues,
            pace: found.pace,
        }
    }

    /// The gait a body of this model covers ground at `speed` in, in `stance`: the one whose clip,
    /// at the model's scale, covers nearest that speed.
    pub fn gait(&self, stance: Option<StanceId>, speed: MetersPerSec) -> Action {
        let mismatch =
            |pace: MetersPerSec| (speed.0.0.max(1e-3) / (pace.0.0 * self.scale())).ln().abs();
        self.skeleton
            .gaits(self.build(), stance)
            .iter()
            .map(|paced| (paced.gait, mismatch(paced.pace)))
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map_or(Action::Walk, |(gait, _)| gait)
    }

    /// How fast `motion` plays on the clip `resolved` names: a hit at the attack speed, a gait as
    /// fast as covers `speed` at the clip's pace and the model's scale, anything else as authored.
    pub fn rate(
        &self,
        motion: &Motion,
        resolved: &Resolved,
        attack_speed: PlaybackRate,
        speed: MetersPerSec,
    ) -> PlaybackRate {
        match resolved.pace {
            _ if motion.strikes() => attack_speed.at_least(0.01),
            Some(pace) if motion.action().moves() && speed.0.0 > 0.0 => {
                PlaybackRate(speed.0.0 / (pace.0.0 * self.scale()))
            }
            _ => PlaybackRate(1.0),
        }
    }

    /// How long a clip picked for a motion plays and when it lands, in a stance, at an attack speed:
    /// only a hit plays faster or slower for it, apex and all.
    pub fn timing(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        pick: usize,
        attack_speed: PlaybackRate,
    ) -> Timing {
        let resolved = self.resolve(stance, motion, Pose::Idle, pick);
        let rate = self.rate(motion, &resolved, attack_speed, MetersPerSec::default());
        let apex = resolved
            .cues
            .iter()
            .find(|cue| cue.apex)
            .map_or(Millis(0.0), |cue| cue.at);
        Timing {
            duration: resolved.length / rate,
            apex: apex.seconds() / rate,
        }
    }

    /// The cues of `resolved`, playing `motion` at `rate`, crossed between the previous sample and
    /// `now`. `prev` is `None` when the motion just started, in which case every cue up to `now` is.
    pub fn cues<'a>(
        resolved: &Resolved<'a>,
        motion: &Motion,
        prev: Option<Seconds>,
        now: Seconds,
        rate: PlaybackRate,
    ) -> Vec<&'a Cue> {
        let authored = |t: Seconds| t.millis() * rate;
        crossed_cues(
            resolved.cues,
            resolved.length.millis(),
            motion.once(),
            prev.map(authored),
            authored(now),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Timing {
    pub duration: Seconds,
    pub apex: Seconds,
}

/// The `cues` of a clip `length` long that its playhead crossed going from `prev` to `now`, both
/// counted from when it began to play, so a looping clip crosses each cue once a loop and one that
/// plays `once` only the first time. `prev` is `None` when the clip just began: every cue up to
/// `now` is crossed.
pub fn crossed_cues(
    cues: &[Cue],
    length: Millis,
    once: bool,
    prev: Option<Millis>,
    now: Millis,
) -> Vec<&Cue> {
    cues.iter()
        .filter(|cue| crossed(once, length, cue.at, prev, now))
        .collect()
}

fn crossed(once: bool, total: Millis, at: Millis, prev: Option<Millis>, now: Millis) -> bool {
    let count = |t: Millis| {
        if t < at {
            0
        } else if once || total.0 <= 0.0 {
            1
        } else {
            (t - at).ratio(total) as i64 + 1
        }
    };
    count(now) > prev.map_or(0, count)
}

/// The skin of each part of `look` that draws on the model's build, painted in the part's colours
/// over the skeleton's when the part has a material.
fn skins(content: &Content, def: &ModelDef, skeleton: &SkeletonDef, look: &BodyLook) -> Vec<Skin> {
    look.parts
        .iter()
        .filter_map(|laid| {
            let part = laid.part.get(content);
            Some(Skin {
                mesh: part.mesh_for(def.build.as_ref())?.path(),
                paint: part
                    .material
                    .map(|material| material.get(content).paint(skeleton, &laid.colours)),
            })
        })
        .collect()
}

fn listed<T: AsRef<str>>(all: &[T]) -> String {
    all.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", ")
}

impl ActorModel {
    fn named(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> (&ClipCell, Naming) {
        self.skeleton
            .def
            .named(self.build(), stance, motion, pose)
            .expect("a skeleton that built names every action in base")
    }
}
