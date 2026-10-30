pub mod cover;
pub mod play;
pub mod render;
pub mod shader;
pub mod sky;

pub use play::{LayerTime, sprite_cell};
pub use shader::{
    BodyShape, CompiledVfxShaders, ParamKind, ShaderError, ShaderParam, SiteKind, Stage,
    VfxShaderDef, VfxShaderId, prelude_names, shader_file,
};

use std::collections::BTreeMap;

use crate::core::math::PosSchema;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::ImageRef;
use crate::core::content::media::ImageSizes;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentRegistry, LoadStep, Loading, NamedContent, Nearby,
    RowCheck, serial,
};
use crate::core::geometry::{PlacedShape, Radians};
use crate::core::math::{FrameSize, Pos, Rgba};
use crate::core::time::{Hertz, Seconds, UnixMillis};
use crate::core::units::Meters;

pub type VfxId = ContentId<VfxDef>;

pub const MAX_PARAMS: usize = 32;
pub const MAX_LAYERS: usize = 16;

/// How long a layer may run or wait: within a minute, and long enough to be seen.
pub const SHORTEST_RUN: Seconds = Seconds(SHORTEST_RUN_EXACT as f32);
pub const LONGEST_RUN: Seconds = Seconds(LONGEST_RUN_EXACT as f32);
// The schema states the bounds as written; an f32 widened to f64 would read 0.0500000007.
const SHORTEST_RUN_EXACT: f64 = 0.05;
const LONGEST_RUN_EXACT: f64 = 60.0;
pub const MAX_FRAME_SIDE: u32 = 1024;
pub const MAX_FRAMES: u32 = 256;
pub const MAX_FPS: f32 = 120.0;

pub fn content(registry: &mut ContentRegistry) {
    registry
        .file::<VfxDef>()
        .named::<VfxDef>()
        .bundle::<VfxShaderDef>()
        .named::<VfxShaderDef>()
        .hook(LoadStep::Files, shader::compile_all)
        .hook(LoadStep::Across, check_all);
}

/// A visual effect: a stack of layers played together on one site. The row declares the parameters
/// a caller fills, which its text and shader layers read.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<VfxParam>,
    pub layers: Vec<VfxLayer>,
    /// Played, with the same parameters, on each actor this vfx's area reaches when it plays on
    /// an area: the targets a cast struck in it, else everyone standing in it. It plays on each
    /// headed the area's way, so a blow lands from where it came: what a blast does to whoever
    /// it catches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_actors_inside: Option<VfxId>,
}

/// A parameter a vfx row leaves to whoever plays it.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxParam {
    pub name: String,
    pub kind: ParamKind,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxLayer {
    Sprite {
        sheet: ImageRef,
        frame: FrameSize,
        #[schemars(extend("minimum" = 1, "maximum" = MAX_FRAMES))]
        frames: u32,
        #[schemars(extend("minimum" = 1.0, "maximum" = MAX_FPS))]
        fps: Hertz,
        #[serde(default, skip_serializing_if = "SpriteAnchor::is_default")]
        anchor: SpriteAnchor,
        #[serde(
            default = "serial::one_f32",
            skip_serializing_if = "serial::is_one_f32"
        )]
        #[schemars(extend("default" = serial::one_f32()))]
        scale: f32,
        #[serde(default, skip_serializing_if = "SpritePlay::is_once")]
        play: SpritePlay,
        #[serde(default, skip_serializing_if = "Seconds::is_zero")]
        delay: Seconds,
    },
    Shader {
        shader: VfxShaderId,
        /// Keyed by the shader's declared parameters, each filled or left to its default.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[schemars(extend("x-params-of" = "shader"))]
        params: BTreeMap<String, ParamFill>,
        #[schemars(extend("minimum" = SHORTEST_RUN_EXACT, "maximum" = LONGEST_RUN_EXACT))]
        duration: Seconds,
        #[serde(default, skip_serializing_if = "Seconds::is_zero")]
        delay: Seconds,
    },
}

#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpriteAnchor {
    #[default]
    Center,
    Feet,
    Head,
}

#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpritePlay {
    #[default]
    Once,
    Loop {
        #[schemars(extend("minimum" = SHORTEST_RUN_EXACT, "maximum" = LONGEST_RUN_EXACT))]
        seconds: Seconds,
    },
}

/// What a shader layer hands one of its shader's parameters: a value of its own, or one of the
/// row's declared parameters, filled by whoever plays the vfx.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[schemars(extend("x-param-fill" = true))]
pub enum ParamFill {
    Param { param: String },
    Value(ParamValue),
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f32),
    Color(Rgba),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(from = "WireParams", into = "WireParams")]
pub struct VfxParams(pub Vec<(String, ParamValue)>);

/// One playing vfx on one site, spawned by the server and drawn by every client that sees it.
#[derive(Component, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct VfxInstance {
    pub vfx: VfxId,
    #[entities]
    pub site: VfxSite,
    pub params: VfxParams,
    /// The way the vfx travels: from whoever dealt it toward the one it lands on, the facing of
    /// the area it plays on, else east. A shader reads it through the prelude's `site_heading`.
    pub heading: Radians,
    pub started: UnixMillis,
    pub until: UnixMillis,
}

/// Where a vfx plays: always one thing. A screen or widget site is the screen of whoever the
/// instance is shown to.
#[derive(Serialize, Deserialize, JsonSchema, MapEntities, Clone, Debug, PartialEq)]
pub enum VfxSite {
    Actor(
        #[entities]
        #[schemars(with = "u64")]
        Entity,
    ),
    Object(
        #[entities]
        #[schemars(with = "u64")]
        Entity,
    ),
    Area(PlacedShape),
    Path(#[schemars(with = "Vec<PosSchema>")] Vec<Pos<Meters>>),
    Screen,
    Widget(WidgetName),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetName(pub String);

impl ParamValue {
    pub fn kind(self) -> ParamKind {
        match self {
            ParamValue::Bool(_) => ParamKind::Bool,
            ParamValue::Number(_) => ParamKind::Number,
            ParamValue::Color(_) => ParamKind::Color,
        }
    }
}

impl VfxParams {
    pub fn get(&self, name: &str) -> Option<ParamValue> {
        self.0
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| *value)
    }
}

impl ParamFill {
    /// What the fill gives: its own value, or the caller's for the parameter it names.
    pub fn resolve(&self, given: &VfxParams) -> Option<ParamValue> {
        match self {
            ParamFill::Value(value) => Some(*value),
            ParamFill::Param { param } => given.get(param),
        }
    }
}

impl VfxLayer {
    pub fn delay(&self) -> Seconds {
        match self {
            VfxLayer::Sprite { delay, .. } | VfxLayer::Shader { delay, .. } => *delay,
        }
    }

    pub fn duration(&self) -> Seconds {
        match self {
            VfxLayer::Sprite {
                frames, fps, play, ..
            } => match play {
                SpritePlay::Once => Seconds(*frames as f32 / fps.0.max(0.001)),
                SpritePlay::Loop { seconds } => *seconds,
            },
            VfxLayer::Shader { duration, .. } => *duration,
        }
    }

    pub fn ends(&self) -> Seconds {
        self.delay() + self.duration()
    }
}

impl VfxDef {
    /// How long the whole stack plays: when its last layer ends.
    pub fn lasts(&self) -> Seconds {
        Seconds(
            self.layers
                .iter()
                .map(|layer| layer.ends().0)
                .fold(0.0, f32::max),
        )
    }

    pub fn declares(&self, name: &str) -> Option<ParamKind> {
        self.params
            .iter()
            .find(|param| param.name == name)
            .map(|param| param.kind)
    }
}

impl NamedContent for VfxDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for VfxDef {
    const TABLE: &'static str = "vfx";

    fn fresh(near: &Nearby) -> Result<VfxDef, String> {
        let layer = VfxLayer::Sprite {
            sheet: near.image()?,
            frame: FrameSize {
                width: 1,
                height: 1,
            },
            frames: 1,
            fps: Hertz(12.0),
            anchor: SpriteAnchor::default(),
            scale: 1.0,
            play: SpritePlay::Once,
            delay: Seconds(0.0),
        };
        Ok(VfxDef {
            name: near.name(),
            params: Vec::new(),
            layers: vec![layer],
            on_actors_inside: None,
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count("/layers", self.layers.len(), 1, MAX_LAYERS, "layers");
        check.count("/params", self.params.len(), 0, MAX_PARAMS, "parameters");
        for (index, param) in self.params.iter().enumerate() {
            shader::check_param_name(check, &format!("/params/{index}/name"), &param.name);
            if self.params[..index]
                .iter()
                .any(|other| other.name == param.name)
            {
                check.error(
                    format!("/params/{index}/name"),
                    "parameter listed twice",
                    &param.name,
                    "each parameter once",
                );
            }
        }
        for (index, layer) in self.layers.iter().enumerate() {
            let at = |field: &str| format!("/layers/{index}/{field}");
            check.range(at("delay"), layer.delay().0, 0.0, LONGEST_RUN.0, "seconds");
            match layer {
                VfxLayer::Sprite {
                    frame,
                    frames,
                    fps,
                    scale,
                    play,
                    ..
                } => {
                    let side = MAX_FRAME_SIDE;
                    check.range(at("frame/width"), frame.width, 1, side, "pixels");
                    check.range(at("frame/height"), frame.height, 1, side, "pixels");
                    check.range(at("frames"), *frames, 1, MAX_FRAMES, "frames");
                    check.range(at("fps"), fps.0, 1.0, MAX_FPS, "frames per second");
                    check.range(at("scale"), *scale, 0.1, 10.0, "a scale");
                    if let SpritePlay::Loop { seconds } = play {
                        check_run(check, at("play/seconds"), *seconds);
                    }
                }
                VfxLayer::Shader {
                    params, duration, ..
                } => {
                    check_run(check, at("duration"), *duration);
                    for (name, fill) in params {
                        if let ParamFill::Param { param } = fill
                            && self.declares(param).is_none()
                        {
                            check.error(
                                at(&format!("params/{name}")),
                                "undeclared parameter",
                                param,
                                "a name the row declares under params",
                            );
                        }
                    }
                }
            }
        }
    }
}

// Replication serializes with postcard, which cannot read the untagged value content uses.
#[derive(Serialize, Deserialize)]
struct WireParams(Vec<(String, WireValue)>);

#[derive(Serialize, Deserialize)]
enum WireValue {
    Bool(bool),
    Number(f32),
    Color(Rgba),
}

impl From<VfxParams> for WireParams {
    fn from(params: VfxParams) -> WireParams {
        WireParams(
            params
                .0
                .into_iter()
                .map(|(name, value)| {
                    let wire = match value {
                        ParamValue::Bool(flag) => WireValue::Bool(flag),
                        ParamValue::Number(number) => WireValue::Number(number),
                        ParamValue::Color(color) => WireValue::Color(color),
                    };
                    (name, wire)
                })
                .collect(),
        )
    }
}

impl From<WireParams> for VfxParams {
    fn from(wire: WireParams) -> VfxParams {
        VfxParams(
            wire.0
                .into_iter()
                .map(|(name, value)| {
                    let value = match value {
                        WireValue::Bool(flag) => ParamValue::Bool(flag),
                        WireValue::Number(number) => ParamValue::Number(number),
                        WireValue::Color(color) => ParamValue::Color(color),
                    };
                    (name, value)
                })
                .collect(),
        )
    }
}

impl VfxLayer {
    /// The stage the layer plays on: a sprite is a billboard, a shader says.
    pub fn stage(&self, content: &Content) -> Stage {
        match self {
            VfxLayer::Sprite { .. } => Stage::Billboard,
            VfxLayer::Shader { shader, .. } => shader.get(content).stage,
        }
    }
}

impl VfxDef {
    /// Refuses, at each layer, a stage that one of the `sites` the vfx plays on, `by` whom, cannot
    /// take.
    pub fn check_sites(
        &self,
        content: &Content,
        sites: &[SiteKind],
        by: &str,
        check: &mut RowCheck,
    ) {
        for (index, layer) in self.layers.iter().enumerate() {
            let stage = layer.stage(content);
            for site in sites.iter().filter(|site| !stage.takes(**site)) {
                check.error(
                    format!("/layers/{index}"),
                    "stage off its site",
                    format!(
                        "a {} layer played on {} site by {by}",
                        stage.label(),
                        site.label()
                    ),
                    format!(
                        "a layer on a stage the site takes: {}",
                        Stage::taken_by(*site)
                    ),
                );
            }
        }
    }
}

async fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    for (id, def) in loading.rows::<VfxDef>(&content) {
        if let Some(inside) = def.on_actors_inside {
            let mut check = RowCheck::default();
            let by = format!("{} on the actors inside it", id.path(&content));
            inside
                .get(&content)
                .check_sites(&content, &[SiteKind::Actor], &by, &mut check);
            loading.row_errors(inside, check);
        }
        let mut check = RowCheck::default();
        for (index, layer) in def.layers.iter().enumerate() {
            let at = |field: &str| format!("/layers/{index}/{field}");
            match layer {
                VfxLayer::Sprite {
                    sheet,
                    frame,
                    frames,
                    ..
                } => {
                    let Some(&(width, height)) = sizes.0.get(sheet.as_str()) else {
                        continue;
                    };
                    if width % frame.width != 0 || height % frame.height != 0 {
                        check.error(
                            at("sheet"),
                            "image size",
                            format!(
                                "{width} × {height}, not a multiple of {} × {}",
                                frame.width, frame.height
                            ),
                            "an image of whole frames",
                        );
                    }
                    let has = (width / frame.width) * (height / frame.height);
                    if *frames > has {
                        check.error(
                            at("frames"),
                            "frames out of range",
                            frames,
                            format!("at most {has}, the frames on the sheet"),
                        );
                    }
                }
                VfxLayer::Shader { shader, params, .. } => {
                    let shader = shader.get(&content);
                    for (name, fill) in params {
                        let Some(declared) = shader.param(name) else {
                            check.error(
                                at(&format!("params/{name}")),
                                "unknown shader parameter",
                                name,
                                format!(
                                    "one of {}",
                                    shader
                                        .params
                                        .iter()
                                        .map(|param| param.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            );
                            continue;
                        };
                        let given = match fill {
                            ParamFill::Param { param } => def.declares(param),
                            ParamFill::Value(value) => Some(value.kind()),
                        };
                        if let Some(given) = given
                            && given != declared.kind
                        {
                            check.error(
                                at(&format!("params/{name}")),
                                "wrong parameter kind",
                                given.label(),
                                format!("a {}", declared.kind.label()),
                            );
                        }
                        if let (ParamFill::Value(ParamValue::Number(number)), Some((min, max))) =
                            (fill, declared.range())
                            && (*number < min || *number > max)
                        {
                            check.error(
                                at(&format!("params/{name}")),
                                "parameter out of range",
                                number,
                                format!("{min}..{max}"),
                            );
                        }
                    }
                }
            }
        }
        loading.row_errors(id, check);
    }
}

fn check_run(check: &mut RowCheck, pointer: String, run: Seconds) {
    check.range(pointer, run.0, SHORTEST_RUN.0, LONGEST_RUN.0, "seconds");
}

impl SpriteAnchor {
    fn is_default(&self) -> bool {
        *self == SpriteAnchor::Center
    }
}

impl SpritePlay {
    fn is_once(&self) -> bool {
        *self == SpritePlay::Once
    }
}
