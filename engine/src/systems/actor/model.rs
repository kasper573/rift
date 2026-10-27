use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use rayon::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use super::playing::Motion;
use super::skeleton::{
    self, BuiltSkeleton, ClipName, Cue, DeclaredMoves, GroupName, Hand, LibraryClip, SkeletonDef,
    SkeletonId,
};
use super::stance::StanceId;
use super::{Action, Pose};
use crate::core::assets::{FileSize, ImageRef, ModelRef};
use crate::core::content::gltf::{self, GltfInfo, NodeTree};
use crate::core::content::media::{GltfInfos, VisiblePixelColumns, visible_columns};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Loading,
    MISSING_FILE, ModuleRule, ModuleSet, Nearby, PartSpec, Parts, RowCheck, Shape,
};
use crate::core::geometry::Capsule;
use crate::core::render::world::mannequin::RigClip;
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::core::units::{Meters, MetersPerSec};

pub type ModelId = ContentId<ModelDef>;

/// A skin on a skeleton: a glTF binary of the mesh bound to the skeleton's rig, with any clips of
/// its own that play in place of the libraries' of the same name, the capsule it is hit in, and its
/// busts.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    #[serde(skip)]
    pub model: ModelRef,
    pub skeleton: SkeletonId,
    pub hitbox: Capsule,
    /// The cues of the clips `model.glb` carries itself, which may be timed unlike the library's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<ClipName, Vec<Cue>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModelModule>,
}

pub const MODEL_GLB: PartSpec = PartSpec::File {
    name: "model",
    exts: &["glb"],
};

pub const MODEL_BUSTS: PartSpec = PartSpec::Keyed {
    folder: "busts",
    exts: &["png"],
};

impl Bundle for ModelDef {
    const PARTS: &'static [PartSpec] = &[MODEL_GLB, MODEL_BUSTS];

    fn read_parts(&mut self, parts: &mut Parts) {
        if let Some(glb) = parts.required(MODEL_GLB) {
            self.model = ModelRef::new(parts.path(glb));
            if let Some(info) = parts.gltf_info(glb) {
                let mut cx = RowCheck::default();
                check_skin(&info, parts.weight(glb), &mut cx);
                parts.config_errors(cx);
            }
        }
        let keyed = parts.keyed(MODEL_BUSTS);
        let measured: Vec<MeasuredBust> = keyed
            .iter()
            .map(|(_, file)| MeasuredBust {
                path: parts.path(file),
                size: parts.image_size(file),
                weight: parts.weight(file),
            })
            .collect();
        let busts = self.modules.iter_mut().find_map(|module| match module {
            ModelModule::Bust(busts) => Some(busts),
            _ => None,
        });
        let Some(busts) = busts else {
            for (_, file) in keyed {
                parts.error(
                    file,
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        "unknown part",
                        format!("file {file}"),
                        "a bust module in config.json for the model's busts",
                    ),
                );
            }
            return;
        };
        for &expression in GenericExpression::VARIANTS {
            let part = format!("busts/{}.png", expression.as_ref());
            if parts.has(&part) {
                *busts.generic.of_mut(expression) = ImageRef::new(parts.path(&part));
            } else {
                parts.error(
                    "",
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        MISSING_FILE,
                        format!("no {part}"),
                        "a bust for every generic expression in busts/",
                    ),
                );
            }
        }
        busts.own = keyed
            .into_iter()
            .filter(|(name, _)| name.parse::<GenericExpression>().is_err())
            .map(|(name, file)| OwnBust {
                name: name.to_owned(),
                file: ImageRef::new(parts.path(file)),
            })
            .collect();
        let mut cx = RowCheck::default();
        check_busts(&measured, &mut cx);
        parts.config_errors(cx);
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentFile for ModelDef {
    const TABLE: &'static str = "model";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<ModelDef, String> {
        Ok(ModelDef {
            model: ModelRef::default(),
            skeleton: near.file()?,
            hitbox: Capsule {
                radius: Meters(0.4),
                height: Meters(1.8),
            },
            overrides: BTreeMap::new(),
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/hitbox/radius", self.hitbox.radius.0, 0.1, 4.0, "m");
        cx.range("/hitbox/height", self.hitbox.height.0, 0.2, 8.0, "m");
        for (clip, cues) in &self.overrides {
            skeleton::check_cues(&format!("/overrides/{}", clip.0), cues, cx);
        }
        for (index, module) in self.modules.iter().enumerate() {
            if let ModelModule::Scale { factor } = module {
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

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelModule {
    Bust(Busts),
    Scale { factor: f32 },
}

impl ModelDef {
    pub fn busts(&self) -> Option<&Busts> {
        self.modules.iter().find_map(|module| match module {
            ModelModule::Bust(busts) => Some(busts),
            _ => None,
        })
    }

    pub fn scale(&self) -> f32 {
        self.modules
            .iter()
            .find_map(|module| match module {
                ModelModule::Scale { factor } => Some(*factor),
                _ => None,
            })
            .unwrap_or(1.0)
    }
}

/// A face for every generic expression and for each of the model's own, kept as parts of its bundle
/// under `busts/`: one `busts/<expression>.png` each.
#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Busts {
    #[serde(skip)]
    pub generic: GenericBusts,
    #[serde(skip)]
    pub own: Vec<OwnBust>,
}

#[derive(Clone)]
pub struct OwnBust {
    pub name: String,
    pub file: ImageRef,
}

#[derive(Clone, Default)]
pub struct GenericBusts {
    pub neutral: ImageRef,
    pub happy: ImageRef,
    pub sad: ImageRef,
    pub angry: ImageRef,
    pub surprised: ImageRef,
    pub thinking: ImageRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Expression {
    Generic(GenericExpression),
    Own(String),
}

impl From<String> for Expression {
    fn from(name: String) -> Expression {
        match name.parse() {
            Ok(expression) => Expression::Generic(expression),
            Err(_) => Expression::Own(name),
        }
    }
}

impl From<Expression> for String {
    fn from(face: Expression) -> String {
        match face {
            Expression::Generic(expression) => expression.as_ref().to_owned(),
            Expression::Own(name) => name,
        }
    }
}

impl JsonSchema for Expression {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Expression")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = String::json_schema(generator);
        schema.insert("x-face".to_owned(), true.into());
        schema
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    AsRefStr,
    EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum GenericExpression {
    #[default]
    Neutral,
    Happy,
    Sad,
    Angry,
    Surprised,
    Thinking,
}

impl Busts {
    pub fn all(&self) -> impl Iterator<Item = &ImageRef> {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(self.own.iter().map(|own| &own.file))
    }

    pub fn face(&self, face: &Expression) -> Option<&ImageRef> {
        match face {
            Expression::Generic(expression) => Some(self.generic.of(*expression)),
            Expression::Own(name) => self
                .own
                .iter()
                .find(|own| own.name == *name)
                .map(|own| &own.file),
        }
    }
}

impl GenericBusts {
    pub fn of(&self, expression: GenericExpression) -> &ImageRef {
        match expression {
            GenericExpression::Neutral => &self.neutral,
            GenericExpression::Happy => &self.happy,
            GenericExpression::Sad => &self.sad,
            GenericExpression::Angry => &self.angry,
            GenericExpression::Surprised => &self.surprised,
            GenericExpression::Thinking => &self.thinking,
        }
    }

    fn of_mut(&mut self, expression: GenericExpression) -> &mut ImageRef {
        match expression {
            GenericExpression::Neutral => &mut self.neutral,
            GenericExpression::Happy => &mut self.happy,
            GenericExpression::Sad => &mut self.sad,
            GenericExpression::Angry => &mut self.angry,
            GenericExpression::Surprised => &mut self.surprised,
            GenericExpression::Thinking => &mut self.thinking,
        }
    }
}

// A character is downloaded by every client that sees it, and drawn many times a frame.
/// The warning of a library carrying clips no set names, downloaded by every client for nothing.
pub const UNNAMED_CLIPS: &str = "clips no set names";

pub const MAX_CHARACTER_BYTES: FileSize = FileSize::kib(4 * 1024);
pub const MAX_CHARACTER_TRIANGLES: usize = 4000;
// The client loads every bust at startup, so each one, and the whole cast, must stay a quick download.
pub const MAX_BUST_BYTES: FileSize = FileSize::kib(120);
pub const MAX_CAST_BYTES: FileSize = FileSize::kib(3 * 1024);

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

/// Builds every skeleton and every model on one, holding each model to its skeleton's rig and each
/// set to the clips its wearers can play.
pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let infos = content.built::<GltfInfos>();
    let skeletons = build_skeletons(loading, &content, infos);
    let mut models: Vec<Option<ActorModel>> = content.ids::<ModelDef>().map(|_| None).collect();
    let rows = loading.rows::<ModelDef>(&content);
    let files = bust_files(loading, &rows);
    let mut cast = CastWeight::default();
    for ((id, def), busts) in rows.iter().zip(files) {
        let mut check = RowCheck::default();
        for (bust, weight) in &busts.weights {
            if let Some(total) = cast.add(bust, *weight) {
                check.error(
                    "/modules",
                    "cast too heavy",
                    format!("the busts up to {bust} weigh {total}"),
                    format!("at most {MAX_CAST_BYTES} for every bust in the game"),
                );
            }
        }
        // A model whose glb is missing or unreadable was refused at that file already, as was a
        // skeleton that did not load.
        let info = infos.0.get(def.model.as_str());
        let skeleton = skeletons.get(def.skeleton.index()).cloned().flatten();
        let (Some(info), Some(skeleton)) = (info, skeleton) else {
            loading.row_errors(*id, check);
            loading.fail(*id);
            continue;
        };
        check_fit(
            def,
            info,
            &skeleton,
            def.skeleton.path(&content),
            &mut check,
        );
        if check.is_empty() {
            models[id.index()] = Some(ActorModel::new(def, info, skeleton, busts.figure));
        } else {
            loading.row_errors(*id, check);
            loading.fail(*id);
        }
    }
    check_sets(loading, &content, &skeletons, &models);
    let declared = loading
        .rows::<SkeletonDef>(&content)
        .into_iter()
        .flat_map(|(_, def)| def.moves().cloned())
        .collect();
    loading.built(DeclaredMoves(declared));
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

/// What a model's glb must be to its skeleton: every joint of the rig at its path and resting as the
/// rig rests, and a clip for every override it times.
fn check_fit(
    def: &ModelDef,
    info: &GltfInfo,
    skeleton: &BuiltSkeleton,
    skeleton_path: &str,
    cx: &mut RowCheck,
) {
    for problem in fit_report(skeleton, skeleton_path, &info.tree).problems {
        cx.problem("/skeleton", problem);
    }
    for (clip, cues) in &def.overrides {
        let pointer = format!("/overrides/{}", clip.0);
        match info.clip(&clip.0) {
            Some(found) => skeleton::check_cue_times(&pointer, cues, found.length, cx),
            None => cx.error(
                pointer,
                "unknown clip",
                format!("model.glb carries no clip {clip}"),
                format!(
                    "one of the clips model.glb carries: {}",
                    listed(&info.clip_names())
                ),
            ),
        }
    }
}

/// How a model's joints fit the skeleton at `skeleton_path`, with the loader's word on each joint
/// that does not.
pub struct FitReport {
    pub fit: gltf::Fit,
    pub problems: Vec<ContentProblem>,
}

pub fn fit_report(skeleton: &BuiltSkeleton, skeleton_path: &str, tree: &NodeTree) -> FitReport {
    let fit = gltf::fit(&skeleton.rig, tree);
    let skeleton_file = format!("{}.skeleton", crate::core::content::name_of(skeleton_path));
    let problems = fit
        .misfits
        .iter()
        .map(|misfit| {
            let mut problem = skeleton::misfit_problem("model.glb", &skeleton_file, misfit);
            if let gltf::Misfit::Missing { joint } = misfit
                && let Some(hand) = skeleton
                    .def
                    .hands
                    .iter()
                    .find(|hand| hand.joint.0 == *joint)
            {
                problem.expected = format!("hand {:?} of {skeleton_file} needs it", hand.name);
            }
            problem
        })
        .collect();
    FitReport { fit, problems }
}

/// Every clip a skeleton's sets name must play on every body wearing it, from a library or the
/// model's own glb, and every clip that plays as a hit marks the one moment it lands. What is
/// downloaded and never played is named.
fn check_sets(
    loading: &mut Loading,
    content: &Content,
    skeletons: &[Option<Arc<BuiltSkeleton>>],
    models: &[Option<ActorModel>],
) {
    for (id, def) in loading.rows::<SkeletonDef>(content) {
        let Some(skeleton) = skeletons.get(id.index()).cloned().flatten() else {
            continue;
        };
        let wearers: Vec<(ModelId, &ActorModel)> = content
            .ids::<ModelDef>()
            .filter_map(|model| Some((model, models.get(model.index())?.as_ref()?)))
            .filter(|(_, model)| model.def.skeleton == id)
            .collect();
        let mut check = RowCheck::default();
        let mut named: HashSet<&ClipName> = HashSet::new();
        for (pointer, set) in def.sets() {
            let set_name = set_name(content, def, &pointer);
            let attacks: HashSet<String> = set.attacks().map(|(entry, _)| entry).collect();
            for (entry, clip) in set.entries() {
                let clip = clip.clip();
                named.insert(clip);
                let at = format!("{pointer}{entry}");
                let in_library = skeleton.clips.contains_key(clip);
                let lacking = wearers
                    .iter()
                    .find(|(_, model)| !in_library && !model.own.contains_key(clip));
                match lacking {
                    Some(_)
                        if !wearers
                            .iter()
                            .any(|(_, model)| model.own.contains_key(clip)) =>
                    {
                        check.error(
                            at.clone(),
                            "unknown clip",
                            format!(
                                "{set_name} set names {clip}: no such clip in clips/*.glb or any model that wears this skeleton"
                            ),
                            "a clip of a library, or of every model wearing the skeleton",
                        );
                    }
                    Some((model, _)) => check.error(
                        at.clone(),
                        "unknown clip",
                        format!(
                            "{set_name} set names {clip}, which {} carries not, nor any library",
                            model.path(content)
                        ),
                        "a clip of a library, or of every model wearing the skeleton",
                    ),
                    None if wearers.is_empty() && !in_library => check.error(
                        at.clone(),
                        "unknown clip",
                        format!("{set_name} set names {clip}: no such clip in clips/*.glb"),
                        "a clip of a library",
                    ),
                    None => {}
                }
                if !attacks.contains(&entry) {
                    continue;
                }
                if in_library {
                    let apexes = apexes(def.cues.get(clip));
                    if apexes != 1 {
                        check.error(
                            at.clone(),
                            "apex out of place",
                            format!("{clip} plays as an attack but has {apexes} apex cues"),
                            "exactly one, where the hit lands",
                        );
                    }
                }
                for (model, worn) in &wearers {
                    if !worn.own.contains_key(clip) {
                        continue;
                    }
                    let apexes = apexes(worn.def.overrides.get(clip));
                    if apexes != 1 {
                        loading.row_error(
                            *model,
                            &format!("/overrides/{}", clip.0),
                            ContentProblem::new(
                                "apex out of place",
                                format!("{clip} plays as an attack but has {apexes} apex cues"),
                                "exactly one, where the hit lands",
                            ),
                        );
                    }
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
        for (model, worn) in &wearers {
            for clip in worn.own.keys().filter(|clip| !named.contains(clip)) {
                loading.row_warning(
                    *model,
                    "",
                    ContentProblem::new(
                        "clip no set names",
                        format!("model.glb carries {clip}, which no set names"),
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

fn apexes(cues: Option<&Vec<Cue>>) -> usize {
    cues.map_or(0, |cues| cues.iter().filter(|cue| cue.apex).count())
}

/// What a model's glb must be on its own: within the style budget, its clips long enough to see and
/// short enough to wait for.
fn check_skin(info: &GltfInfo, weight: Option<FileSize>, cx: &mut RowCheck) {
    if let Some(weight) = weight
        && weight > MAX_CHARACTER_BYTES
    {
        cx.error(
            "",
            "model too heavy",
            format!("model.glb weighs {weight}"),
            format!("at most {MAX_CHARACTER_BYTES}"),
        );
    }
    cx.range("", info.triangles, 0, MAX_CHARACTER_TRIANGLES, "triangles");
    cx.range("", info.joints(), 0, skeleton::MAX_JOINTS, "joints");
    for clip in &info.clips {
        let length = clip.length.millis();
        if length < skeleton::MIN_CLIP || length > skeleton::MAX_CLIP {
            cx.error(
                "",
                "clip length",
                format!("{:?} plays {} ms", clip.name, length.0),
                format!("{} ms to {} ms", skeleton::MIN_CLIP.0, skeleton::MAX_CLIP.0),
            );
        }
    }
}

/// A bust of a model's bundle as its file measures.
struct MeasuredBust {
    path: String,
    size: Option<(u32, u32)>,
    weight: Option<FileSize>,
}

/// What a model's busts must be to each other: one size, each light enough to download.
fn check_busts(busts: &[MeasuredBust], cx: &mut RowCheck) {
    let mut first: Option<(&str, (u32, u32))> = None;
    for MeasuredBust {
        path: bust,
        size,
        weight,
    } in busts
    {
        if let Some(weight) = weight
            && *weight > MAX_BUST_BYTES
        {
            cx.error(
                "/modules",
                "bust too heavy",
                format!("{bust} weighs {weight}"),
                format!("at most {MAX_BUST_BYTES}"),
            );
        }
        let Some(own) = *size else {
            continue;
        };
        match first {
            None => first = Some((bust, own)),
            Some((earlier, (w, h))) if (w, h) != own => {
                cx.error(
                    "/modules",
                    "bust sizes differ",
                    format!("{bust} is {} × {}, {earlier} is {w} × {h}", own.0, own.1),
                    "every bust of one model at one size",
                );
                return;
            }
            Some(_) => {}
        }
    }
}

pub struct ActorModel {
    def: ModelDef,
    skeleton: Arc<BuiltSkeleton>,
    /// The clips the model's own glb carries: their place there and how long each plays.
    own: BTreeMap<ClipName, (usize, Seconds)>,
    figure: Option<BustFigure>,
}

/// Busts carry wide transparent margins, so a bust is placed by its figure; taking the span over
/// all expressions keeps a new face from moving it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BustFigure {
    pub left: f32,
    pub right: f32,
}

/// What plays for a motion: the clip, the group it is kept to, where it comes from, how long it
/// plays once, and its cues.
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

/// Where a clip is played from: the model's own glb, or a library of its skeleton, each by the
/// clip's place among that file's animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipSource {
    Model { index: usize },
    Library { library: usize, index: usize },
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

    fn new(
        def: &ModelDef,
        info: &GltfInfo,
        skeleton: Arc<BuiltSkeleton>,
        figure: Option<BustFigure>,
    ) -> ActorModel {
        ActorModel {
            def: def.clone(),
            skeleton,
            own: info
                .clips
                .iter()
                .enumerate()
                .map(|(index, clip)| (ClipName::new(clip.name.clone()), (index, clip.length)))
                .collect(),
            figure,
        }
    }

    pub fn bust_figure(&self) -> Option<BustFigure> {
        self.figure
    }

    pub fn hitbox(&self) -> Capsule {
        self.def.hitbox
    }

    pub fn model(&self) -> &ModelRef {
        &self.def.model
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

    /// The file and place among its animations of a clip `source` names for this model.
    pub fn rig_clip(&self, source: ClipSource) -> RigClip {
        match source {
            ClipSource::Model { index } => RigClip {
                file: self.def.model.path(),
                index,
            },
            ClipSource::Library { library, index } => RigClip {
                file: self.skeleton.def.libraries[library].file.path(),
                index,
            },
        }
    }

    /// The clip a turntable stands the model in for `motion` in `stance` and `pose`, when its
    /// skeleton or the model carries it.
    pub fn standing_clip(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        pose: Pose,
    ) -> Option<RigClip> {
        self.resolve(stance, motion, pose)
            .source
            .map(|source| self.rig_clip(source))
    }

    /// The clips the model's own glb carries, which play in place of the libraries'.
    pub fn own_clips(&self) -> impl Iterator<Item = (&ClipName, usize)> {
        self.own.iter().map(|(name, (index, _))| (name, *index))
    }

    /// What plays for `motion` in `stance`, holding `pose` while idle: the stance's set names it,
    /// else the base set; a move neither names plays the stance's attack, else the base's. The clip
    /// is the model's own when its glb carries one of the name, else the skeleton's libraries'.
    pub fn resolve(&self, stance: Option<StanceId>, motion: &Motion, pose: Pose) -> Resolved<'_> {
        let skeleton = &self.skeleton.def;
        let (clip, _) = skeleton.named(stance, motion, pose);
        let name = clip.clip();
        let (source, length, cues) = match self.own.get(name) {
            Some(&(index, length)) => (
                Some(ClipSource::Model { index }),
                length,
                self.def.overrides.get(name),
            ),
            None => match self.skeleton.clips.get(name) {
                Some(&LibraryClip {
                    library,
                    index,
                    length,
                }) => (
                    Some(ClipSource::Library { library, index }),
                    length,
                    skeleton.cues.get(name),
                ),
                None => (None, Seconds(0.0), None),
            },
        };
        Resolved {
            clip: name,
            only: clip.only(),
            source,
            length,
            cues: cues.map_or(&[], Vec::as_slice),
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

    /// How long a motion plays and when it lands, in a stance, at an attack speed: only a hit plays
    /// faster or slower for it, apex and all.
    pub fn timing(
        &self,
        stance: Option<StanceId>,
        motion: &Motion,
        attack_speed: PlaybackRate,
    ) -> Timing {
        let resolved = self.resolve(stance, motion, Pose::Idle);
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

#[derive(Default)]
struct CastWeight {
    counted: HashSet<String>,
    total: usize,
    over: bool,
}

impl CastWeight {
    /// Counts `bust` once however many models share it, and tells the total the first time it
    /// passes the budget.
    fn add(&mut self, bust: &str, weight: FileSize) -> Option<FileSize> {
        if !self.counted.insert(bust.to_owned()) {
            return None;
        }
        self.total += weight.0;
        let total = FileSize(self.total);
        (total > MAX_CAST_BYTES && !std::mem::replace(&mut self.over, true)).then_some(total)
    }
}

struct BustFiles {
    figure: Option<BustFigure>,
    weights: Vec<(String, FileSize)>,
}

fn bust_files(loading: &Loading, rows: &[(ModelId, &ModelDef)]) -> Vec<BustFiles> {
    let folder = loading.folder();
    rows.par_iter()
        .map(|(_, def)| {
            let files: Vec<(String, Vec<u8>)> = def
                .busts()
                .into_iter()
                .flat_map(Busts::all)
                .filter_map(|bust| {
                    let bytes = folder.read(&bust.path()).ok()?;
                    Some((bust.as_str().to_owned(), bytes))
                })
                .collect();
            let spans: Vec<VisiblePixelColumns> = files
                .iter()
                .filter_map(|(_, bytes)| visible_columns(bytes).ok().flatten())
                .collect();
            let left = spans
                .iter()
                .map(|span| span.first as f32 / span.image_width as f32);
            let right = spans
                .iter()
                .map(|span| (span.last + 1) as f32 / span.image_width as f32);
            let figure = left
                .reduce(f32::min)
                .zip(right.reduce(f32::max))
                .map(|(left, right)| BustFigure { left, right });
            BustFiles {
                figure,
                weights: files
                    .into_iter()
                    .map(|(path, bytes)| (path, FileSize(bytes.len())))
                    .collect(),
            }
        })
        .collect()
}

fn listed<T: AsRef<str>>(all: &[T]) -> String {
    all.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", ")
}
