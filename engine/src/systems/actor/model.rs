use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};

use rayon::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use super::Action;
use crate::core::assets::{FileSize, ImageRef};
use crate::core::audio::playback::SfxId;
use crate::core::content::media::{ImageSizes, VisiblePixelColumns, visible_columns};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Loading,
    MISSING_FILE, ModuleRule, ModuleSet, Nearby, PartSpec, Parts, RowCheck, Shape, serial,
};
use crate::core::geometry::Radians;
use crate::core::math::{Direction, FrameSize, Pos, Rect, Size, WorldPx};
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::core::units::Meters;

pub type ModelId = ContentId<ModelDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    pub hitbox: HitboxSize,
    pub sprites: SpriteSheet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModelModule>,
}

pub const MODEL_SPRITES: PartSpec = PartSpec::File {
    name: "sprites",
    exts: &["png"],
};

pub const MODEL_BUSTS: PartSpec = PartSpec::Keyed {
    folder: "busts",
    exts: &["png"],
};

impl Bundle for ModelDef {
    const PARTS: &'static [PartSpec] = &[MODEL_SPRITES, MODEL_BUSTS];

    fn read_parts(&mut self, parts: &mut Parts) {
        if let Some(sheet) = parts.required(MODEL_SPRITES) {
            self.sprites.image = ImageRef::new(parts.path(sheet));
            if let Some(size) = parts.image_size(sheet) {
                let mut cx = RowCheck::default();
                check_sheet(&self.sprites, size, &mut cx);
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
        let still = || {
            vec![SheetFrame {
                frame: 0,
                ms: 100,
                step: false,
                apex: false,
                sound: None,
            }]
        };
        let strips = || {
            Direction::VARIANTS
                .iter()
                .map(|&direction| (direction, still()))
                .collect()
        };
        Ok(ModelDef {
            hitbox: HitboxSize {
                width: 1.0,
                height: 1.0,
            },
            sprites: SpriteSheet {
                image: ImageRef::default(),
                frame_size: FrameSize {
                    width: 1,
                    height: 1,
                },
                actions: Action::VARIANTS
                    .iter()
                    .map(|&action| (action, strips()))
                    .collect(),
            },
            modules: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/hitbox/width", self.hitbox.width, 0.1, 8.0, "m");
        cx.range("/hitbox/height", self.hitbox.height, 0.1, 8.0, "m");
        check_strips(&self.sprites, cx);
        let sheet = &self.sprites;
        cx.range(
            "/sprites/frame_size/width",
            sheet.frame_size.width,
            1,
            MAX_FRAME_PX,
            "pixels",
        );
        cx.range(
            "/sprites/frame_size/height",
            sheet.frame_size.height,
            1,
            MAX_FRAME_PX,
            "pixels",
        );
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

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitboxSize {
    pub width: f32,
    pub height: f32,
}

impl HitboxSize {
    pub fn tiles(self) -> Size<Meters> {
        Size::new(self.width, self.height)
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSheet {
    #[serde(skip)]
    pub image: ImageRef,
    pub frame_size: FrameSize,
    pub actions: BTreeMap<Action, BTreeMap<Direction, Vec<SheetFrame>>>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetFrame {
    pub frame: u32,
    pub ms: u32,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub step: bool,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub apex: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SfxId>,
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

pub const MAX_STRIP_FRAMES: usize = 256;
pub const MAX_FRAME_PX: u32 = 8192;
// The client loads every bust at startup, so each one, and the whole cast, must stay a quick download.
pub const MAX_BUST_BYTES: FileSize = FileSize::kib(120);
pub const MAX_CAST_BYTES: FileSize = FileSize::kib(3 * 1024);
pub const MAX_FRAME_MS: u32 = 60_000;

pub struct Models(pub Vec<Option<ActorModel>>);

pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
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
        if check.is_empty() {
            let image = sizes.0.get(def.sprites.image.as_str()).copied();
            models[id.index()] = Some(ActorModel::new(def, image, busts.figure));
        } else {
            loading.row_errors(id, check);
            loading.fail(id);
        }
    }
    loading.built(Models(models));
}

/// What a model's sheet must be to its config: whole frames, each of the frames the strips show.
fn check_sheet(sheet: &SpriteSheet, (width, height): (u32, u32), cx: &mut RowCheck) {
    let frame = sheet.frame_size;
    if width % frame.width.max(1) != 0 || height % frame.height.max(1) != 0 {
        cx.error(
            "/sprites/frame_size",
            "image size",
            format!(
                "{width} × {height}, not a multiple of {} × {}",
                frame.width, frame.height
            ),
            "an image of whole frames",
        );
    }
    let frames = (width / frame.width.max(1)) * (height / frame.height.max(1));
    for (action, strips) in &sheet.actions {
        for (direction, strip) in strips {
            for (index, shown) in strip.iter().enumerate() {
                if shown.frame >= frames {
                    cx.error(
                        format!(
                            "/sprites/actions/{}/{}/{index}/frame",
                            action.as_ref(),
                            direction.as_ref()
                        ),
                        "tile out of range",
                        format!("frame {} of {frames}", shown.frame),
                        format!("0 to {}", frames.saturating_sub(1)),
                    );
                }
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

/// What a model's strips must be on their own: frames for every action in every direction, each
/// shown a while, with one apex in each attack strip at most.
fn check_strips(sheet: &SpriteSheet, cx: &mut RowCheck) {
    for &action in Action::VARIANTS {
        let Some(strips) = sheet.actions.get(&action) else {
            cx.error(
                "/sprites/actions",
                "missing frames",
                format!("no {} action", action.as_ref()),
                format!("every action: {}", listed(Action::VARIANTS)),
            );
            continue;
        };
        for &direction in Direction::VARIANTS {
            let pointer = format!(
                "/sprites/actions/{}/{}",
                action.as_ref(),
                direction.as_ref()
            );
            let Some(strip) = strips.get(&direction) else {
                cx.error(
                    format!("/sprites/actions/{}", action.as_ref()),
                    "missing frames",
                    format!("direction {} has no frames", direction.as_ref()),
                    format!(
                        "1 to {MAX_STRIP_FRAMES} frames in each of {}",
                        listed(Direction::VARIANTS)
                    ),
                );
                continue;
            };
            cx.count(&pointer, strip.len(), 1, MAX_STRIP_FRAMES, "frames");
            let mut apexes = Vec::new();
            for (index, frame) in strip.iter().enumerate() {
                cx.range(
                    format!("{pointer}/{index}/ms"),
                    frame.ms,
                    1,
                    MAX_FRAME_MS,
                    "whole milliseconds",
                );
                if frame.apex {
                    apexes.push(index);
                }
            }
            if (apexes.len() > 1) || (!apexes.is_empty() && !action.takes_apex()) {
                cx.error(
                    &pointer,
                    "apex out of place",
                    format!(
                        "{} · {}: frames {}",
                        action.as_ref(),
                        direction.as_ref(),
                        apexes
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                    "one apex, in attack strips only",
                );
            }
        }
    }
}

pub struct ActorModel {
    def: ModelDef,
    columns: u32,
    strips: Vec<Vec<Vec<SheetFrame>>>,
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

    // Only a sheet that passed the load checks, with frames for every action and direction, gets here.
    fn new(def: &ModelDef, image: Option<(u32, u32)>, figure: Option<BustFigure>) -> ActorModel {
        let sheet = &def.sprites;
        let columns = image
            .map_or(1, |(width, _)| width / sheet.frame_size.width.max(1))
            .max(1);
        let strips = Action::VARIANTS
            .iter()
            .map(|action| {
                Direction::VARIANTS
                    .iter()
                    .map(|direction| sheet.actions[action][direction].clone())
                    .collect()
            })
            .collect();
        ActorModel {
            def: def.clone(),
            columns,
            strips,
            figure,
        }
    }

    pub fn bust_figure(&self) -> Option<BustFigure> {
        self.figure
    }

    pub fn hitbox(&self) -> Size<Meters> {
        self.def.hitbox.tiles()
    }

    pub fn sheet(&self) -> &ImageRef {
        &self.def.sprites.image
    }

    pub fn airborne(&self) -> bool {
        self.def.airborne()
    }

    pub fn drawn_size(&self, frame: Rect<WorldPx>) -> Size<WorldPx> {
        frame.size * self.def.scale()
    }

    pub fn frame(
        &self,
        action: Action,
        facing: Radians,
        t: Seconds,
        attack_speed: PlaybackRate,
    ) -> Rect<WorldPx> {
        let strip = self.strip(action, facing);
        let elapsed = (t.millis() * rate(action, attack_speed)).max(Millis(0.0));
        let total = total_ms(strip);
        let position = if action == Action::Dead {
            elapsed
        } else {
            elapsed % total
        };
        let mut cursor = Millis(0.0);
        for frame in strip {
            cursor += Millis(frame.ms as f32);
            if position < cursor {
                return self.region(frame.frame);
            }
        }
        self.region(strip[strip.len() - 1].frame)
    }

    /// How long an action plays and when it lands, at an attack speed: only an attack plays faster
    /// or slower for it.
    pub fn timing(&self, action: Action, facing: Radians, attack_speed: PlaybackRate) -> Timing {
        let strip = self.strip(action, facing);
        let rate = rate(action, attack_speed);
        let mut apex = Millis(0.0);
        let mut cursor = Millis(0.0);
        for frame in strip {
            if frame.apex {
                apex = cursor;
            }
            cursor += Millis(frame.ms as f32);
        }
        Timing {
            duration: cursor.seconds() / rate,
            apex: apex.seconds() / rate,
        }
    }

    /// `prev` is the playback position at the previous sample, or `None` when the action just
    /// started, in which case every cue up to `now` fires.
    pub fn cues(
        &self,
        action: Action,
        facing: Radians,
        prev: Option<Seconds>,
        now: Seconds,
        attack_speed: PlaybackRate,
    ) -> (Vec<&SfxId>, bool) {
        let strip = self.strip(action, facing);
        let rate = rate(action, attack_speed);
        let authored = |t: Seconds| t.millis() * rate;
        let total = total_ms(strip);
        let once = action == Action::Dead;
        let (mut sfx, mut stepped) = (Vec::new(), false);
        let mut cursor = Millis(0.0);
        for frame in strip {
            if crossed(once, total, cursor, prev.map(authored), authored(now)) {
                sfx.extend(frame.sound.as_ref());
                stepped |= frame.step;
            }
            cursor += Millis(frame.ms as f32);
        }
        (sfx, stepped)
    }

    fn strip(&self, action: Action, facing: Radians) -> &[SheetFrame] {
        &self.strips[action as usize][Direction::from(facing.heading()) as usize]
    }

    fn region(&self, frame: u32) -> Rect<WorldPx> {
        let size = self.def.sprites.frame_size;
        let frame_size = Size::new(size.width as f32, size.height as f32);
        Rect::new(
            Pos::new(
                (frame % self.columns) as f32 * frame_size.width,
                (frame / self.columns) as f32 * frame_size.height,
            ),
            frame_size,
        )
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

fn total_ms(strip: &[SheetFrame]) -> Millis {
    Millis(strip.iter().map(|frame| frame.ms as f32).sum())
}

fn crossed(once: bool, total: Millis, at: Millis, prev: Option<Millis>, now: Millis) -> bool {
    let count = |t: Millis| {
        if t < at {
            0
        } else if once {
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
