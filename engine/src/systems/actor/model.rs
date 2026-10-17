use std::borrow::Cow;
use std::collections::BTreeMap;

use rayon::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use super::Action;
use crate::core::assets::AssetRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::media::{ImageSizes, VisiblePixelColumns, visible_columns};
use crate::core::content::{
    Content, ContentId, ContentRow, Loading, ModuleRule, ModuleSet, RowCheck, serial,
};
use crate::core::math::{Direction, FrameSize, Pos, Rect, Size, WorldPx};
use crate::core::tiling::Tiles;
use crate::core::time::{Millis, PlaybackRate, Seconds};

pub type ModelId = ContentId<ModelDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    pub hitbox: HitboxSize,
    pub sprites: Sprites,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModelModule>,
}

impl ContentRow for ModelDef {
    const TABLE: &'static str = "model";

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/hitbox/width", self.hitbox.width, 0.1, 8.0, "tiles");
        cx.range("/hitbox/height", self.hitbox.height, 0.1, 8.0, "tiles");
        let sheet = self.sheet();
        cx.range(
            "/sprites/frame_size/width",
            sheet.frame_size.width,
            1,
            8192,
            "pixels",
        );
        cx.range(
            "/sprites/frame_size/height",
            sheet.frame_size.height,
            1,
            8192,
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
    pub fn tiles(self) -> Size<Tiles> {
        Size::new(self.width, self.height)
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Sprites {
    Sheet(Sheet),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub image: AssetRef,
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

    pub fn sheet(&self) -> &Sheet {
        match &self.sprites {
            Sprites::Sheet(sheet) => sheet,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Busts {
    #[serde(flatten)]
    pub generic: GenericBusts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub own: Vec<OwnBust>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnBust {
    pub name: String,
    pub file: AssetRef,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct GenericBusts {
    pub neutral: AssetRef,
    pub happy: AssetRef,
    pub sad: AssetRef,
    pub angry: AssetRef,
    pub surprised: AssetRef,
    pub thinking: AssetRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Face {
    Generic(GenericExpression),
    Own(String),
}

impl From<String> for Face {
    fn from(name: String) -> Face {
        match name.parse() {
            Ok(expression) => Face::Generic(expression),
            Err(_) => Face::Own(name),
        }
    }
}

impl From<Face> for String {
    fn from(face: Face) -> String {
        match face {
            Face::Generic(expression) => expression.as_ref().to_owned(),
            Face::Own(name) => name,
        }
    }
}

impl JsonSchema for Face {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Face")
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
    pub fn all(&self) -> impl Iterator<Item = &AssetRef> {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(self.own.iter().map(|own| &own.file))
    }

    pub fn face(&self, face: &Face) -> Option<&AssetRef> {
        match face {
            Face::Generic(expression) => Some(self.generic.of(*expression)),
            Face::Own(name) => self
                .own
                .iter()
                .find(|own| own.name == *name)
                .map(|own| &own.file),
        }
    }
}

impl GenericBusts {
    pub fn of(&self, expression: GenericExpression) -> &AssetRef {
        match expression {
            GenericExpression::Neutral => &self.neutral,
            GenericExpression::Happy => &self.happy,
            GenericExpression::Sad => &self.sad,
            GenericExpression::Angry => &self.angry,
            GenericExpression::Surprised => &self.surprised,
            GenericExpression::Thinking => &self.thinking,
        }
    }
}

pub const MAX_STRIP_FRAMES: usize = 256;
pub const MAX_FRAME_MS: u32 = 60_000;

pub struct Models(pub Vec<Option<ActorModel>>);

pub fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    let mut models: Vec<Option<ActorModel>> = content.ids::<ModelDef>().map(|_| None).collect();
    let rows = loading.rows::<ModelDef>(&content);
    let figures = bust_figures(loading, &rows);
    for ((id, def), figure) in rows.into_iter().zip(figures) {
        let mut check = RowCheck::default();
        let sheet = def.sheet();
        let image = sizes.0.get(sheet.image.as_str()).copied();
        if let Some((width, height)) = image
            && (width % sheet.frame_size.width.max(1) != 0
                || height % sheet.frame_size.height.max(1) != 0)
        {
            check.error(
                "/sprites/image",
                "image size",
                format!(
                    "{width} × {height}, not a multiple of {} × {}",
                    sheet.frame_size.width, sheet.frame_size.height
                ),
                "an image of whole frames",
            );
        }
        let frames = image.map(|(width, height)| {
            (width / sheet.frame_size.width.max(1)) * (height / sheet.frame_size.height.max(1))
        });
        for &action in Action::VARIANTS {
            let Some(strips) = sheet.actions.get(&action) else {
                check.error(
                    "/sprites/actions",
                    "missing frames",
                    format!("no {} action", action.as_ref()),
                    "every action: idle, walk, run, attack, death",
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
                    check.error(
                        format!("/sprites/actions/{}", action.as_ref()),
                        "missing frames",
                        format!("direction {} has no frames", direction.as_ref()),
                        "1 to 256 frames in each of s, sw, nw, n, ne, se, e, w",
                    );
                    continue;
                };
                check.count(&pointer, strip.len(), 1, MAX_STRIP_FRAMES, "frames");
                let mut apexes = Vec::new();
                for (index, frame) in strip.iter().enumerate() {
                    check.range(
                        format!("{pointer}/{index}/ms"),
                        frame.ms,
                        1,
                        MAX_FRAME_MS,
                        "whole milliseconds",
                    );
                    if let Some(frames) = frames
                        && frame.frame >= frames
                    {
                        check.error(
                            format!("{pointer}/{index}/frame"),
                            "tile out of range",
                            format!("frame {} of {frames}", frame.frame),
                            format!("0 to {}", frames.saturating_sub(1)),
                        );
                    }
                    if frame.apex {
                        apexes.push(index);
                    }
                }
                if (apexes.len() > 1) || (!apexes.is_empty() && action != Action::Attack) {
                    check.error(
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
        if let Some(busts) = def.busts() {
            let mut size: Option<(u32, u32, &str)> = None;
            for bust in busts.all() {
                let Some(&own) = sizes.0.get(bust.as_str()) else {
                    continue;
                };
                match size {
                    None => size = Some((own.0, own.1, bust.as_str())),
                    Some((w, h, first)) if (w, h) != own => {
                        check.error(
                            "/modules",
                            "bust sizes differ",
                            format!(
                                "{} is {} × {}, {first} is {w} × {h}",
                                bust.as_str(),
                                own.0,
                                own.1
                            ),
                            "every bust of one model at one size",
                        );
                        break;
                    }
                    Some(_) => {}
                }
            }
        }
        if check.is_empty() {
            models[id.index()] = Some(ActorModel::new(def, image, figure));
        } else {
            loading.row_errors(id, check);
            loading.fail(id);
        }
    }
    loading.built(Models(models));
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
            .unwrap_or_else(|| panic!("model {} failed to load", model.key(content)))
    }

    // Only a sheet that passed the load checks, with frames for every action and direction, gets here.
    fn new(def: &ModelDef, image: Option<(u32, u32)>, figure: Option<BustFigure>) -> ActorModel {
        let sheet = def.sheet();
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

    pub fn hitbox(&self) -> Size<Tiles> {
        self.def.hitbox.tiles()
    }

    pub fn sheet(&self) -> &crate::core::assets::AssetRef {
        &self.def.sheet().image
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
        dir: Direction,
        t: Seconds,
        attack_speed: PlaybackRate,
    ) -> Rect<WorldPx> {
        let strip = self.strip(action, dir);
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

    pub fn timing(&self, action: Action, dir: Direction) -> Timing {
        let strip = self.strip(action, dir);
        let mut apex = Millis(0.0);
        let mut cursor = Millis(0.0);
        for frame in strip {
            if frame.apex {
                apex = cursor;
            }
            cursor += Millis(frame.ms as f32);
        }
        Timing {
            duration: cursor.seconds(),
            apex: apex.seconds(),
        }
    }

    /// `prev` is the playback position at the previous sample, or `None` when the action just
    /// started, in which case every cue up to `now` fires.
    pub fn cues(
        &self,
        action: Action,
        dir: Direction,
        prev: Option<Seconds>,
        now: Seconds,
        attack_speed: PlaybackRate,
    ) -> (Vec<&SfxId>, bool) {
        let strip = self.strip(action, dir);
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

    fn strip(&self, action: Action, dir: Direction) -> &[SheetFrame] {
        &self.strips[action as usize][dir as usize]
    }

    fn region(&self, frame: u32) -> Rect<WorldPx> {
        let size = self.def.sheet().frame_size;
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

fn bust_figures(loading: &Loading, rows: &[(ModelId, &ModelDef)]) -> Vec<Option<BustFigure>> {
    let folder = loading.folder();
    rows.par_iter()
        .map(|(_, def)| {
            let spans: Vec<VisiblePixelColumns> = def
                .busts()?
                .all()
                .filter_map(|bust| folder.read(&bust.path()).ok())
                .filter_map(|bytes| visible_columns(&bytes).ok().flatten())
                .collect();
            let left = spans
                .iter()
                .map(|span| span.first as f32 / span.image_width as f32);
            let right = spans
                .iter()
                .map(|span| (span.last + 1) as f32 / span.image_width as f32);
            Some(BustFigure {
                left: left.reduce(f32::min)?,
                right: right.reduce(f32::max)?,
            })
        })
        .collect()
}
