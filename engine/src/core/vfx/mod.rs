pub mod render;
pub mod shader;

pub use shader::{
    CompiledVfxShaders, ParamKind, ShaderError, ShaderParam, VfxShaderDef, VfxShaderId, shader_file,
};

use std::collections::BTreeMap;

use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::content::media::ImageSizes;
use crate::core::content::{ContentId, ContentRegistry, ContentRow, LoadStep, Loading, RowCheck};
use crate::core::geometry::Footprint;
use crate::core::math::{FrameSize, Pos, Rgba};
use crate::core::tiling::Tiles;
use crate::core::time::{Seconds, UnixMillis};

pub type VfxId = ContentId<VfxDef>;

pub const MAX_PARAMS: usize = 8;
pub const MAX_LAYERS: usize = 16;

pub fn content(registry: &mut ContentRegistry) {
    registry
        .row::<VfxDef>()
        .row::<VfxShaderDef>()
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
    pub params: Vec<ParamDecl>,
    pub layers: Vec<VfxLayer>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamDecl {
    pub name: String,
    pub kind: ParamKind,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxLayer {
    Sprite {
        sheet: AssetRef,
        frame: FrameSize,
        frames: u32,
        fps: f32,
        #[serde(default, skip_serializing_if = "SpriteAnchor::is_default")]
        anchor: SpriteAnchor,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        #[schemars(extend("default" = one()))]
        scale: f32,
        #[serde(default, skip_serializing_if = "Play::is_once")]
        play: Play,
        #[serde(default, skip_serializing_if = "is_no_time")]
        delay: Seconds,
    },
    Text {
        template: String,
        color: Rgba,
        #[serde(
            default = "default_text_size",
            skip_serializing_if = "is_default_text_size"
        )]
        #[schemars(extend("default" = default_text_size()))]
        size: f32,
        #[serde(default = "default_rise", skip_serializing_if = "is_default_rise")]
        #[schemars(extend("default" = default_rise()))]
        rise: Tiles,
        duration: Seconds,
        #[serde(default, skip_serializing_if = "is_no_time")]
        delay: Seconds,
    },
    Shader {
        shader: VfxShaderId,
        /// Keyed by the shader's declared parameters, each filled or left to its default.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[schemars(extend("x-params-of" = "shader"))]
        params: BTreeMap<String, ParamFill>,
        duration: Seconds,
        #[serde(default, skip_serializing_if = "is_no_time")]
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
pub enum Play {
    #[default]
    Once,
    Loop {
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
    pub started: UnixMillis,
    pub until: UnixMillis,
}

/// Where a vfx plays: always one thing. A screen or widget site is the screen of whoever the
/// instance is shown to.
#[derive(Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub enum VfxSite {
    Actor(#[entities] Entity),
    Object(#[entities] Entity),
    Area(Footprint),
    Path(Vec<Pos<Tiles>>),
    Screen,
    Widget(WidgetName),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetName(pub String);

impl ParamValue {
    pub fn kind(self) -> ParamKind {
        match self {
            ParamValue::Bool(_) => ParamKind::Bool,
            ParamValue::Number(_) => ParamKind::Number,
            ParamValue::Color(_) => ParamKind::Color,
        }
    }

    pub fn text(self) -> String {
        match self {
            ParamValue::Bool(flag) => flag.to_string(),
            ParamValue::Number(number) => {
                let rounded = (number * 100.0).round() / 100.0;
                if rounded.fract() == 0.0 {
                    format!("{rounded:.0}")
                } else {
                    format!("{rounded:.2}").trim_end_matches('0').to_owned()
                }
            }
            ParamValue::Color(color) => format!("{:08x}", color.0),
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
    pub fn delay(&self) -> Seconds {
        match self {
            VfxLayer::Sprite { delay, .. }
            | VfxLayer::Text { delay, .. }
            | VfxLayer::Shader { delay, .. } => *delay,
        }
    }

    pub fn duration(&self) -> Seconds {
        match self {
            VfxLayer::Sprite {
                frames, fps, play, ..
            } => match play {
                Play::Once => Seconds(*frames as f32 / fps.max(0.001)),
                Play::Loop { seconds } => *seconds,
            },
            VfxLayer::Text { duration, .. } | VfxLayer::Shader { duration, .. } => *duration,
        }
    }

    pub fn ends(&self) -> Seconds {
        self.delay() + self.duration()
    }

    pub fn kind(&self) -> &'static str {
        match self {
            VfxLayer::Sprite { .. } => "sprite",
            VfxLayer::Text { .. } => "text",
            VfxLayer::Shader { .. } => "shader",
        }
    }
}

impl VfxDef {
    /// How long the whole stack plays: when its last layer ends.
    pub fn lasts(&self) -> Seconds {
        self.layers
            .iter()
            .map(VfxLayer::ends)
            .fold(
                Seconds(0.0),
                |longest, ends| {
                    if ends > longest { ends } else { longest }
                },
            )
    }

    pub fn declares(&self, name: &str) -> Option<ParamKind> {
        self.params
            .iter()
            .find(|param| param.name == name)
            .map(|param| param.kind)
    }

    /// Every `{name}` a text template reads.
    pub fn placeholders(template: &str) -> Vec<String> {
        template
            .split('{')
            .skip(1)
            .filter_map(|rest| rest.split_once('}'))
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    pub fn fill(template: &str, params: &VfxParams) -> String {
        let mut text = template.to_owned();
        for (name, value) in &params.0 {
            text = text.replace(&format!("{{{name}}}"), &value.text());
        }
        text
    }
}

impl ContentRow for VfxDef {
    const TABLE: &'static str = "vfx";

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
            check.range(at("delay"), layer.delay().0, 0.0, 60.0, "seconds");
            match layer {
                VfxLayer::Sprite {
                    frame,
                    frames,
                    fps,
                    scale,
                    play,
                    ..
                } => {
                    check.range(at("frame/width"), frame.width, 1, 1024, "pixels");
                    check.range(at("frame/height"), frame.height, 1, 1024, "pixels");
                    check.range(at("frames"), *frames, 1, 256, "frames");
                    check.range(at("fps"), *fps, 1.0, 120.0, "frames per second");
                    check.range(at("scale"), *scale, 0.1, 10.0, "a scale");
                    if let Play::Loop { seconds } = play {
                        check.range(at("play/seconds"), seconds.0, 0.05, 60.0, "seconds");
                    }
                }
                VfxLayer::Text {
                    template,
                    size,
                    rise,
                    duration,
                    ..
                } => {
                    check.text(at("template"), template, 1, 40);
                    check.range(at("size"), *size, 6.0, 64.0, "points");
                    check.range(at("rise"), rise.0, -4.0, 4.0, "tiles");
                    check.range(at("duration"), duration.0, 0.05, 60.0, "seconds");
                    for name in VfxDef::placeholders(template) {
                        if self.declares(&name).is_none() {
                            check.error(
                                at("template"),
                                "undeclared parameter",
                                format!("{{{name}}}"),
                                "a name the row declares under params",
                            );
                        }
                    }
                }
                VfxLayer::Shader {
                    params, duration, ..
                } => {
                    check.range(at("duration"), duration.0, 0.05, 60.0, "seconds");
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

fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    for (id, def) in loading.rows::<VfxDef>(&content) {
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
                VfxLayer::Text { .. } => {}
            }
        }
        loading.row_errors(id, check);
    }
}

fn one() -> f32 {
    1.0
}

fn is_one(scale: &f32) -> bool {
    *scale == 1.0
}

fn default_text_size() -> f32 {
    14.0
}

fn is_default_text_size(size: &f32) -> bool {
    *size == default_text_size()
}

fn default_rise() -> Tiles {
    Tiles(1.0)
}

fn is_default_rise(rise: &Tiles) -> bool {
    *rise == default_rise()
}

fn is_no_time(seconds: &Seconds) -> bool {
    seconds.0 == 0.0
}

impl SpriteAnchor {
    fn is_default(&self) -> bool {
        *self == SpriteAnchor::Center
    }
}

impl Play {
    fn is_once(&self) -> bool {
        *self == Play::Once
    }
}
