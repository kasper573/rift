use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};

use rayon::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use super::{Action, Pose};
use crate::core::assets::{FileSize, ImageRef, ModelRef};
use crate::core::audio::playback::SfxId;
use crate::core::content::gltf::GltfInfo;
use crate::core::content::media::{GltfInfos, VisiblePixelColumns, visible_columns};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Loading,
    MISSING_FILE, ModuleRule, ModuleSet, Nearby, PartSpec, Parts, RowCheck, Shape, serial,
};
use crate::core::geometry::Capsule;
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::core::units::Meters;

pub type ModelId = ContentId<ModelDef>;

/// A character's rig: a glTF binary with one clip for each action and a clip for each pose it can
/// hold, cues at moments on those clips, the capsule it is hit in, and its busts.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    #[serde(skip)]
    pub model: ModelRef,
    pub hitbox: Capsule,
    pub clips: BTreeMap<Action, Clip>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub poses: BTreeMap<Pose, Clip>,
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

/// An animation of the model's glTF, named as it is there, with the moments on it that matter to
/// the game.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
}

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

impl Bundle for ModelDef {
    const PARTS: &'static [PartSpec] = &[MODEL_GLB, MODEL_BUSTS];

    fn read_parts(&mut self, parts: &mut Parts) {
        if let Some(glb) = parts.required(MODEL_GLB) {
            self.model = ModelRef::new(parts.path(glb));
            if let Some(info) = parts.gltf_info(glb) {
                let mut cx = RowCheck::default();
                check_rig(self, &info, parts.weight(glb), &mut cx);
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

    fn fresh(_: &Nearby) -> Result<ModelDef, String> {
        Ok(ModelDef {
            model: ModelRef::default(),
            hitbox: Capsule {
                radius: Meters(0.4),
                height: Meters(1.8),
            },
            clips: Action::VARIANTS
                .iter()
                .map(|&action| {
                    let cues = if action.takes_apex() {
                        vec![Cue {
                            at: Millis(400.0),
                            step: false,
                            apex: true,
                            sound: None,
                        }]
                    } else {
                        Vec::new()
                    };
                    (
                        action,
                        Clip {
                            name: action.as_ref().to_owned(),
                            cues,
                        },
                    )
                })
                .collect(),
            poses: BTreeMap::new(),
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/hitbox/radius", self.hitbox.radius.0, 0.1, 4.0, "m");
        cx.range("/hitbox/height", self.hitbox.height.0, 0.2, 8.0, "m");
        for &action in Action::VARIANTS {
            match self.clips.get(&action) {
                Some(clip) => check_clip(
                    clip,
                    &format!("/clips/{}", action.as_ref()),
                    action.takes_apex(),
                    cx,
                ),
                None => cx.error(
                    "/clips",
                    "missing clip",
                    format!("no {} clip", action.as_ref()),
                    format!("a clip for every action: {}", listed(Action::VARIANTS)),
                ),
            }
        }
        for (pose, clip) in &self.poses {
            let pointer = format!("/poses/{}", pose.as_ref());
            if *pose == Pose::Idle {
                cx.error(
                    &pointer,
                    "pose is an action",
                    "a clip for the idle pose",
                    "the idle action's clip, which idling plays already",
                );
            }
            check_clip(clip, &pointer, false, cx);
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
    Scale {
        factor: f32,
    },
    /// Walks ignore blocked cells; wild ones keep out of safe zones.
    Airborne,
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

    pub fn airborne(&self) -> bool {
        self.modules
            .iter()
            .any(|module| matches!(module, ModelModule::Airborne))
    }

    /// Every clip the model names, with where it stands in the config.
    pub fn named_clips(&self) -> impl Iterator<Item = (String, &Clip)> {
        let actions = self
            .clips
            .iter()
            .map(|(action, clip)| (format!("/clips/{}", action.as_ref()), clip));
        let poses = self
            .poses
            .iter()
            .map(|(pose, clip)| (format!("/poses/{}", pose.as_ref()), clip));
        actions.chain(poses)
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
pub const MAX_CHARACTER_BYTES: FileSize = FileSize::kib(4 * 1024);
pub const MAX_CHARACTER_TRIANGLES: usize = 4000;
pub const MAX_JOINTS: usize = 64;
pub const MIN_CLIP: Millis = Millis(100.0);
pub const MAX_CLIP: Millis = Millis(60_000.0);
pub const MAX_CUES: usize = 32;
// The client loads every bust at startup, so each one, and the whole cast, must stay a quick download.
pub const MAX_BUST_BYTES: FileSize = FileSize::kib(120);
pub const MAX_CAST_BYTES: FileSize = FileSize::kib(3 * 1024);

pub struct Models(pub Vec<Option<ActorModel>>);

pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let infos = content.built::<GltfInfos>();
    let mut models: Vec<Option<ActorModel>> = content.ids::<ModelDef>().map(|_| None).collect();
    let rows = loading.rows::<ModelDef>(&content);
    let files = bust_files(loading, &rows);
    let mut cast = CastWeight::default();
    for ((id, def), busts) in rows.into_iter().zip(files) {
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
        // A model whose glb is missing or unreadable was refused at that file already.
        let info = infos.0.get(def.model.as_str());
        match info {
            Some(info) if check.is_empty() => {
                models[id.index()] = Some(ActorModel::new(def, info, busts.figure));
            }
            _ => {
                loading.row_errors(id, check);
                loading.fail(id);
            }
        }
    }
    loading.built(Models(models));
}

/// What a clip must be on its own: a name, a few cues, and an apex only where a hit lands.
fn check_clip(clip: &Clip, pointer: &str, takes_apex: bool, cx: &mut RowCheck) {
    cx.text(format!("{pointer}/name"), &clip.name, 1, 64);
    cx.count(
        format!("{pointer}/cues"),
        clip.cues.len(),
        0,
        MAX_CUES,
        "cues",
    );
    let apexes: Vec<usize> = clip
        .cues
        .iter()
        .enumerate()
        .filter(|(_, cue)| cue.apex)
        .map(|(index, _)| index)
        .collect();
    if takes_apex && apexes.len() != 1 {
        cx.error(
            format!("{pointer}/cues"),
            "apex out of place",
            format!("{} apex cues", apexes.len()),
            "exactly one cue marked apex, where the hit lands",
        );
    } else if !takes_apex && !apexes.is_empty() {
        cx.error(
            format!("{pointer}/cues"),
            "apex out of place",
            format!("apex cues at {}", listed_indices(&apexes)),
            "an apex in the attack clip only",
        );
    }
    for (index, cue) in clip.cues.iter().enumerate() {
        if cue.at.0 < 0.0 {
            cx.error(
                format!("{pointer}/cues/{index}/at"),
                "cue before the clip",
                format!("{} ms", cue.at.0),
                "a moment from 0 ms",
            );
        }
    }
}

/// What a model's config must be to its glb: within the style budget, every clip an animation
/// there, long enough to see and short enough to wait for, with every cue on it.
fn check_rig(def: &ModelDef, info: &GltfInfo, weight: Option<FileSize>, cx: &mut RowCheck) {
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
    cx.range("", info.joints, 0, MAX_JOINTS, "joints");
    for (pointer, clip) in def.named_clips() {
        let Some(found) = info.clip(&clip.name) else {
            cx.error(
                format!("{pointer}/name"),
                "unknown clip",
                format!("no animation {:?} in model.glb", clip.name),
                format!("one of {}", listed(&info.clip_names())),
            );
            continue;
        };
        let length = found.length.millis();
        if length < MIN_CLIP || length > MAX_CLIP {
            cx.error(
                format!("{pointer}/name"),
                "clip length",
                format!("{:?} plays {} ms", clip.name, length.0),
                format!("{} ms to {} ms", MIN_CLIP.0, MAX_CLIP.0),
            );
        }
        for (index, cue) in clip.cues.iter().enumerate() {
            if cue.at > length {
                cx.error(
                    format!("{pointer}/cues/{index}/at"),
                    "cue past the clip",
                    format!("{} ms into a clip of {} ms", cue.at.0, length.0),
                    "a moment within the clip",
                );
            }
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
    lengths: BTreeMap<Action, Seconds>,
    pose_lengths: BTreeMap<Pose, Seconds>,
    figure: Option<BustFigure>,
}

/// Busts carry wide transparent margins, so a bust is placed by its figure; taking the span over
/// all expressions keeps a new face from moving it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BustFigure {
    pub left: f32,
    pub right: f32,
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

    // Only a config that passed the load checks, with a clip in the glb for every action, gets here.
    fn new(def: &ModelDef, info: &GltfInfo, figure: Option<BustFigure>) -> ActorModel {
        let length = |clip: &Clip| {
            info.clip(&clip.name)
                .map_or(Seconds(0.0), |clip| clip.length)
        };
        ActorModel {
            def: def.clone(),
            lengths: def
                .clips
                .iter()
                .map(|(action, clip)| (*action, length(clip)))
                .collect(),
            pose_lengths: def
                .poses
                .iter()
                .map(|(pose, clip)| (*pose, length(clip)))
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

    pub fn airborne(&self) -> bool {
        self.def.airborne()
    }

    pub fn scale(&self) -> f32 {
        self.def.scale()
    }

    pub fn clip(&self, action: Action) -> &Clip {
        &self.def.clips[&action]
    }

    /// How long the action's clip plays once, as authored.
    pub fn length(&self, action: Action) -> Seconds {
        self.lengths[&action]
    }

    /// The clip held in a pose, with its length, when the model has one.
    pub fn pose(&self, pose: Pose) -> Option<(&Clip, Seconds)> {
        Some((self.def.poses.get(&pose)?, *self.pose_lengths.get(&pose)?))
    }

    /// How long an action plays and when it lands, at an attack speed: only an attack plays faster
    /// or slower for it.
    pub fn timing(&self, action: Action, attack_speed: PlaybackRate) -> Timing {
        let rate = rate(action, attack_speed);
        let apex = self
            .clip(action)
            .cues
            .iter()
            .find(|cue| cue.apex)
            .map_or(Millis(0.0), |cue| cue.at);
        Timing {
            duration: self.length(action) / rate,
            apex: apex.seconds() / rate,
        }
    }

    /// The cues crossed between the previous sample and `now` of an action's playback: the sounds
    /// to play and whether a foot came down. `prev` is `None` when the action just started, in
    /// which case every cue up to `now` fires.
    pub fn cues(
        &self,
        action: Action,
        prev: Option<Seconds>,
        now: Seconds,
        attack_speed: PlaybackRate,
    ) -> (Vec<&SfxId>, bool) {
        let rate = rate(action, attack_speed);
        let authored = |t: Seconds| t.millis() * rate;
        let total = self.length(action).millis();
        let once = action == Action::Dead;
        let (mut sfx, mut stepped) = (Vec::new(), false);
        for cue in &self.clip(action).cues {
            if crossed(once, total, cue.at, prev.map(authored), authored(now)) {
                sfx.extend(cue.sound.as_ref());
                stepped |= cue.step;
            }
        }
        (sfx, stepped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Timing {
    pub duration: Seconds,
    pub apex: Seconds,
}

fn rate(action: Action, attack_speed: PlaybackRate) -> PlaybackRate {
    if action == Action::Attack {
        attack_speed.at_least(0.01)
    } else {
        PlaybackRate(1.0)
    }
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

fn listed_indices(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" and ")
}
