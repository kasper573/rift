//! A model: a body put together from parts on one skeleton, in one of its builds, with the capsule
//! it is hit in. What it plays and how it times its hits are its skeleton's.

use std::collections::HashSet;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::appearance::{self, BodyLook, LaidPart};
use super::material;
use super::part::{ModelPartDef, ModelPartId};
use super::playing::Motion;
use super::skeleton::{
    self, BuildName, BuiltSkeleton, ClipCell, ClipName, Colours, Cue, Declared, GroupName, Hand,
    LibraryClip, SkeletonDef, SkeletonId,
};
use super::stance::StanceId;
use super::{Action, Pose};
use crate::core::assets::ModelRef;
use crate::core::content::gltf::{self, GltfInfo};
use crate::core::content::media::GltfInfos;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, ErrorPlace, Loading, ModuleRule, ModuleSet,
    Nearby, RowCheck,
};
use crate::core::geometry::Capsule;
use crate::core::render::world::body::{BodyFiles, Skin};
use crate::core::render::world::mannequin::RigClip;
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
/// The warning of a library carrying clips no set names, downloaded by every client for nothing.
pub const UNNAMED_CLIPS: &str = "clips no set names";

impl ContentFile for ModelDef {
    const TABLE: &'static str = "model";

    fn fresh(near: &Nearby) -> Result<ModelDef, String> {
        Ok(ModelDef {
            skeleton: near.file()?,
            build: None,
            hitbox: Capsule {
                radius: Meters(0.4),
                height: Meters(1.8),
            },
            parts: vec![near.file()?],
            colours: Colours::default(),
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/hitbox/radius", self.hitbox.radius.0, 0.1, 4.0, "m");
        cx.range("/hitbox/height", self.hitbox.height.0, 0.2, 8.0, "m");
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

/// Every skeleton that loaded, by its id: its rig and where each clip of its libraries is.
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
    let skeletons = build_skeletons(loading, &content, infos);
    for (id, part) in loading.rows::<ModelPartDef>(&content) {
        let Some(skeleton) = skeletons.get(part.skeleton.index()).cloned().flatten() else {
            loading.fail(id);
            continue;
        };
        let problems = part_problems(part, &skeleton, part.skeleton.path(&content), infos);
        let mut check = RowCheck::default();
        check_part(&content, part, &skeleton.def, &mut check);
        if !problems.is_empty() || !check.is_empty() {
            for (file, problem) in problems {
                loading.file_error(file, ErrorPlace::WholeFile, problem);
            }
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
        let skeleton = skeletons.get(def.skeleton.index()).cloned().flatten();
        // A skeleton or part that did not load was refused at its own file already.
        let (Some(skeleton), true) = (
            skeleton,
            def.parts.iter().all(|part| built_parts.contains(part)),
        ) else {
            loading.fail(id);
            continue;
        };
        let mut check = RowCheck::default();
        check_model(&content, def, &skeleton, infos, &mut check);
        if check.is_empty() {
            models[id.index()] = Some(ActorModel::new(&content, def, skeleton));
        } else {
            loading.row_errors(id, check);
            loading.fail(id);
        }
    }
    check_sets(loading, &content, &skeletons);
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
        let Some(rig) = infos.0.get(def.rig.as_str()) else {
            loading.fail(id);
            continue;
        };
        let libraries: Vec<Option<&GltfInfo>> = def
            .libraries
            .iter()
            .map(|library| infos.0.get(library.file.as_str()))
            .collect();
        let skeleton = BuiltSkeleton::new(def, rig, &libraries);
        built[id.index()] = Some(Arc::new(skeleton));
    }
    built
}

/// What a part's meshes must be to its skeleton: each cut for one of its builds, and skinned to
/// joints of its rig at the rig's paths, resting as the rig rests them.
fn part_problems(
    part: &ModelPartDef,
    skeleton: &BuiltSkeleton,
    skeleton_path: &str,
    infos: &GltfInfos,
) -> Vec<(String, ContentProblem)> {
    let mut problems = Vec::new();
    for (build, mesh) in part.meshes() {
        if let Some(build) = build
            && !skeleton.def.builds.contains(build)
        {
            problems.push((
                mesh.path(),
                ContentProblem::new(
                    "unknown build",
                    format!("a mesh for build {build}"),
                    expected_build(&skeleton.def.builds, skeleton_path),
                ),
            ));
        }
        if let Some(info) = infos.0.get(mesh.as_str()) {
            let report = fit_report(skeleton, skeleton_path, &mesh_label(mesh), info);
            problems.extend(
                report
                    .problems
                    .into_iter()
                    .map(|problem| (mesh.path(), problem)),
            );
        }
    }
    problems
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
    let skeleton_file = format!("{}.skeleton", crate::core::content::name_of(skeleton_path));
    let problems = fit
        .misfits
        .iter()
        .map(|misfit| skeleton::misfit_problem(file, &skeleton_file, misfit))
        .collect();
    FitReport { fit, problems }
}

/// A part's mesh as its bundle names it: `mesh.glb`, `builds/<build>.glb`.
pub fn mesh_label(mesh: &ModelRef) -> String {
    let path = mesh.path();
    match path.rsplit_once("/builds/") {
        Some((_, build)) => format!("builds/{build}"),
        None => path
            .rsplit_once('/')
            .map_or(path.clone(), |(_, file)| file.to_owned()),
    }
}

/// Every clip a skeleton's sets name must be in its libraries, and every clip that plays as a hit
/// marks the one moment it lands. What is downloaded and never played is named.
fn check_sets(loading: &mut Loading, content: &Content, skeletons: &[Option<Arc<BuiltSkeleton>>]) {
    for (id, def) in loading.rows::<SkeletonDef>(content) {
        let Some(skeleton) = skeletons.get(id.index()).cloned().flatten() else {
            continue;
        };
        let mut check = RowCheck::default();
        let mut named: HashSet<&ClipName> = HashSet::new();
        for (pointer, set) in def.sets() {
            let set_name = set_name(content, def, &pointer);
            let attacks: HashSet<String> = set.attacks().map(|(entry, _)| entry).collect();
            for (entry, clip) in set.entries() {
                let clip = clip.clip();
                named.insert(clip);
                let at = format!("{pointer}{entry}");
                if !skeleton.clips.contains_key(clip) {
                    check.error(
                        at.clone(),
                        "unknown clip",
                        format!("{set_name} set names {clip}: no such clip in clips/*.glb"),
                        "a clip of a library",
                    );
                    continue;
                }
                if !attacks.contains(&entry) {
                    continue;
                }
                let apexes = def
                    .cues
                    .get(clip)
                    .map_or(0, |cues| cues.iter().filter(|cue| cue.apex).count());
                if apexes != 1 {
                    check.error(
                        at.clone(),
                        "apex out of place",
                        format!("{clip} plays as an attack but has {apexes} apex cues"),
                        "exactly one, where the hit lands",
                    );
                }
            }
        }
        for (index, library) in def.libraries.iter().enumerate() {
            let unnamed = skeleton
                .clips
                .iter()
                .filter(|(name, clip)| clip.library == index && !named.contains(name))
                .count();
            if unnamed > 0 {
                loading.row_warning(
                    id,
                    "",
                    ContentProblem::new(
                        UNNAMED_CLIPS,
                        format!(
                            "clips/{}.glb carries {} no set names",
                            library.key,
                            ui::words::counted(unnamed, "clip")
                        ),
                        "every clip downloaded played by some set",
                    ),
                );
            }
        }
        loading.row_errors(id, check);
    }
}

fn set_name(content: &Content, def: &SkeletonDef, pointer: &str) -> String {
    def.stances
        .iter()
        .enumerate()
        .find(|(index, _)| pointer == format!("/stances/{index}/set"))
        .map_or_else(
            || "base".to_owned(),
            |(_, set)| set.stance.name(content).to_owned(),
        )
}

pub struct ActorModel {
    def: ModelDef,
    skeleton: Arc<BuiltSkeleton>,
    /// The skins the model draws bare: those of the parts left showing once its own are laid, each
    /// as cut for its build.
    skins: Vec<Skin>,
}

/// What plays for a motion: the clip, the group it is kept to, where it is in the skeleton's
/// libraries, how long it plays once, and its cues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved<'a> {
    pub clip: &'a ClipName,
    pub only: Option<&'a GroupName>,
    pub source: Option<ClipSource>,
    pub length: Seconds,
    pub cues: &'a [Cue],
    /// The ground the clip covers at scale 1, when it is played to walk, run or sprint.
    pub pace: Option<MetersPerSec>,
}

/// Where a clip is played from: a library of the skeleton, and the clip's place among that file's
/// animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipSource {
    pub library: usize,
    pub index: usize,
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

    /// The files the model is drawn from bare: its skeleton's rig and the skins of its own parts.
    pub fn body(&self) -> BodyFiles {
        BodyFiles {
            rig: self.skeleton.def.rig.path(),
            skins: self.skins.clone(),
        }
    }

    /// The files a body of the model looking as `look` is drawn from: its skeleton's rig and the
    /// meshes of the look's parts as cut for the model's build, each painted in its colours.
    pub fn dressed(&self, content: &Content, look: &BodyLook) -> BodyFiles {
        BodyFiles {
            rig: self.skeleton.def.rig.path(),
            skins: skins(content, &self.def, &self.skeleton.def, look),
        }
    }

    pub fn skeleton_id(&self) -> SkeletonId {
        self.def.skeleton
    }

    pub fn skeleton(&self) -> &BuiltSkeleton {
        &self.skeleton
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

    /// The file and place among its animations of the clip at `source`.
    pub fn rig_clip(&self, source: ClipSource) -> RigClip {
        RigClip {
            file: self.skeleton.def.libraries[source.library].file.path(),
            index: source.index,
        }
    }

    /// The clip a turntable stands the model in for `motion` in `stance` and `pose`, when its
    /// skeleton carries it.
    pub fn standing_clip(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        pose: Pose,
    ) -> Option<RigClip> {
        self.resolve(stance, motion, pose, 0)
            .source
            .map(|source| self.rig_clip(source))
    }

    /// The cell that plays `motion` in `stance`, holding `pose` while idle.
    pub fn cell(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> &ClipCell {
        self.skeleton.def.named(stance, motion, pose).0
    }

    /// What plays as the `variant`th variant of `motion` in `stance`, holding `pose` while idle:
    /// the stance's set names it, else the base set; a move neither names plays the stance's
    /// attack, else the base's.
    pub fn resolve(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        pose: Pose,
        variant: usize,
    ) -> Resolved<'_> {
        let skeleton = &self.skeleton.def;
        let clip = self.cell(stance, motion, pose).variant(variant);
        let name = clip.clip();
        let (source, length) = match self.skeleton.clips.get(name) {
            Some(&LibraryClip {
                library,
                index,
                length,
            }) => (Some(ClipSource { library, index }), length),
            None => (None, Seconds(0.0)),
        };
        Resolved {
            clip: name,
            only: clip.only(),
            source,
            length,
            cues: skeleton.cues.get(name).map_or(&[], Vec::as_slice),
            pace: skeleton.paces.get(name).copied(),
        }
    }

    /// The gait a body of this model covers ground at `speed` in, in `stance`: the one whose clip,
    /// at the model's scale, covers nearest that speed.
    pub fn gait(&self, stance: Option<StanceId>, speed: MetersPerSec) -> Action {
        let mismatch =
            |pace: MetersPerSec| (speed.0.0.max(1e-3) / (pace.0.0 * self.scale())).ln().abs();
        self.skeleton
            .gaits(stance)
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

    /// How long a variant of a motion plays and when it lands, in a stance, at an attack speed:
    /// only a hit plays faster or slower for it, apex and all.
    pub fn timing(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        variant: usize,
        attack_speed: PlaybackRate,
    ) -> Timing {
        let resolved = self.resolve(stance, motion, Pose::Idle, variant);
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
