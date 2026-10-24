use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use super::ParamValue;
use crate::core::content::{
    Bundle, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Loading, NamedContent,
    Nearby, PartSpec, Parts, RowCheck, Shape,
};
use crate::core::math::Rgba;

pub type VfxShaderId = ContentId<VfxShaderDef>;

/// The bytes the parameter uniform spans, however few parameters a shader declares.
pub const PARAMS_BYTES: usize = 128;

/// A shader a game ships: the WGSL part of its bundle, `shader.wgsl`, defining
/// `fn vfx(uv: vec2<f32>) -> vec4<f32>`, the stage it runs on, and the parameters it reads from
/// `params`, each with the default it gets when a layer leaves it unfilled and, for a number, the
/// range the designer means it to be set within. A flag is `true` or `false` in the data and 0.0 or
/// 1.0 in the shader.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxShaderDef {
    pub name: String,
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ShaderParam>,
}

/// Where a shader runs: in the material of the thing it plays on, as a decal on the ground under
/// it, on a quad facing the camera at it, over the whole screen, or in the text of a plate.
#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Stage {
    Surface,
    Ground,
    Billboard,
    Screen,
    Text,
}

impl Stage {
    /// Whether a layer on this stage plays on a site of `kind`: a surface needs a mesh, the ground
    /// and a billboard a place in the world, the screen and the text their own sites.
    pub fn takes(self, site: SiteKind) -> bool {
        match self {
            Stage::Surface => matches!(site, SiteKind::Actor | SiteKind::Object),
            Stage::Ground | Stage::Billboard => matches!(
                site,
                SiteKind::Actor | SiteKind::Object | SiteKind::Area | SiteKind::Path
            ),
            Stage::Screen => site == SiteKind::Screen,
            Stage::Text => site == SiteKind::Text,
        }
    }

    /// The stages a site of `kind` takes, named.
    pub fn taken_by(site: SiteKind) -> String {
        Stage::VARIANTS
            .iter()
            .filter(|stage| stage.takes(site))
            .map(|stage| stage.label())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

pub const SHADER_SOURCE: PartSpec = PartSpec::File {
    name: "shader",
    exts: &["wgsl"],
};

/// Where the WGSL of the shader bundle at `path` lives.
pub fn shader_file(path: &str) -> String {
    format!("{path}/{}", SHADER_SOURCE.named("", "wgsl"))
}

impl Bundle for VfxShaderDef {
    const PARTS: &'static [PartSpec] = &[SHADER_SOURCE];

    fn read_parts(&mut self, parts: &mut Parts) {
        parts.required(SHADER_SOURCE);
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderParam {
    pub name: String,
    pub kind: ParamKind,
    pub default: ParamValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
}

#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ParamKind {
    Number,
    Bool,
    Color,
}

impl ParamKind {
    /// The value a parameter of the kind holds before anything fills it.
    pub fn blank(self) -> ParamValue {
        match self {
            ParamKind::Number => ParamValue::Number(1.0),
            ParamKind::Bool => ParamValue::Bool(false),
            ParamKind::Color => ParamValue::Color(Rgba::WHITE),
        }
    }
}

/// What a shader is drawn on, as the prelude's `site.kind` and `SITE_*` constants name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum SiteKind {
    Actor,
    Object,
    Area,
    Path,
    Screen,
    Text,
}

impl SiteKind {
    /// The site as a sentence names it: "an actor", "the screen".
    pub fn label(self) -> &'static str {
        match self {
            SiteKind::Actor => "an actor",
            SiteKind::Object => "an object",
            SiteKind::Area => "an area",
            SiteKind::Path => "a path",
            SiteKind::Screen => "the screen",
            SiteKind::Text => "a text",
        }
    }
}

/// The body of an area site, as the prelude's `site.shape` and `SHAPE_*` constants name it; a site
/// without one is its image's box, or the corridor along a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum BodyShape {
    #[default]
    None,
    Circle,
    Rect,
    Cone,
    Line,
}

/// Where each parameter sits in the uniform, by the layout rules WGSL gives the generated struct,
/// and what fills it when nothing else does.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSlot {
    pub name: String,
    pub kind: ParamKind,
    pub offset: usize,
    pub default: ParamValue,
}

/// The composed source of every shader row, built once at load so every client compiles the same
/// text the load validated.
pub struct CompiledVfxShaders(pub HashMap<VfxShaderId, String>);

#[derive(Clone, Debug, PartialEq)]
pub struct ShaderError {
    pub place: ErrorPlace,
    pub message: String,
}

impl ParamKind {
    pub fn label(self) -> &'static str {
        match self {
            ParamKind::Bool => "bool",
            ParamKind::Number => "number",
            ParamKind::Color => "color",
        }
    }

    // A flag reaches the shader as 0.0 or 1.0, which mixes and selects without a cast.
    fn wgsl(self) -> &'static str {
        match self {
            ParamKind::Bool | ParamKind::Number => "f32",
            ParamKind::Color => "vec4<f32>",
        }
    }

    fn size(self) -> usize {
        match self {
            ParamKind::Bool | ParamKind::Number => 4,
            ParamKind::Color => 16,
        }
    }
}

impl ShaderParam {
    /// The range a number parameter is set within; a flag or a color has none.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.min?, self.max?))
    }
}

impl VfxShaderDef {
    pub fn param(&self, name: &str) -> Option<&ShaderParam> {
        self.params.iter().find(|param| param.name == name)
    }

    pub fn layout(&self) -> Vec<ParamSlot> {
        layout(&self.params)
    }

    /// The shader's text as the pipeline compiles it: the row's file first, so its lines keep their
    /// numbers, then the prelude, the site's constants and the struct its parameters are read
    /// through.
    pub fn compose(&self, source: &str) -> String {
        let mut composed = String::with_capacity(source.len() + PRELUDE.len() + 512);
        composed.push_str(source);
        if !source.ends_with('\n') {
            composed.push('\n');
        }
        composed.push_str(PRELUDE);
        composed.push('\n');
        for (index, kind) in SiteKind::VARIANTS.iter().enumerate() {
            composed.push_str(&format!("const SITE_{}: u32 = {index}u;\n", kind.name()));
        }
        for (index, shape) in BodyShape::VARIANTS.iter().enumerate() {
            composed.push_str(&format!("const SHAPE_{}: u32 = {index}u;\n", shape.name()));
        }
        composed.push_str("\nstruct Params {\n");
        for param in &self.params {
            composed.push_str(&format!("    {}: {},\n", param.name, param.kind.wgsl()));
        }
        composed.push_str(&params_padding(&self.params));
        composed.push_str("}\n");
        composed
    }

    pub fn check_source(&self, source: &str) -> Result<String, ShaderError> {
        if !source.contains("fn vfx(") {
            return Err(ShaderError {
                place: ErrorPlace::WholeFile,
                message: "no vfx function".to_owned(),
            });
        }
        let composed = self.compose(source);
        let own_lines = source.lines().count();
        validate(&composed).map_err(|error| ShaderError {
            place: match error.place {
                ErrorPlace::Text { line, .. } if line > own_lines => ErrorPlace::WholeFile,
                place => place,
            },
            ..error
        })?;
        Ok(composed)
    }
}

/// Every name the prelude and the composed constants give a shader to read: what an editor
/// highlights as the vfx vocabulary.
pub fn prelude_names() -> Vec<String> {
    let mut names = vec!["vfx".to_owned(), "site".to_owned(), "params".to_owned()];
    for line in PRELUDE.lines() {
        let declared = line
            .strip_prefix("fn ")
            .and_then(|rest| rest.split_once('('))
            .or_else(|| line.strip_prefix("const ")?.split_once(':'))
            .map(|(name, _)| name.trim().to_owned());
        names.extend(declared);
    }
    names.extend(
        SiteKind::VARIANTS
            .iter()
            .map(|kind| format!("SITE_{}", kind.name())),
    );
    names.extend(
        BodyShape::VARIANTS
            .iter()
            .map(|shape| format!("SHAPE_{}", shape.name())),
    );
    names
}

impl NamedContent for VfxShaderDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for VfxShaderDef {
    const TABLE: &'static str = "vfx_shader";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<VfxShaderDef, String> {
        Ok(VfxShaderDef {
            name: near.name(),
            stage: Stage::Billboard,
            params: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count(
            "/params",
            self.params.len(),
            0,
            super::MAX_PARAMS,
            "parameters",
        );
        for (index, param) in self.params.iter().enumerate() {
            let at = format!("/params/{index}/name");
            check_param_name(check, &at, &param.name);
            if self.params[..index]
                .iter()
                .any(|other| other.name == param.name)
            {
                check.error(
                    at,
                    "parameter listed twice",
                    &param.name,
                    "each parameter once",
                );
            }
            if param.default.kind() != param.kind {
                check.error(
                    format!("/params/{index}/default"),
                    "wrong parameter kind",
                    param.default.kind().label(),
                    format!("a {}", param.kind.label()),
                );
            }
            match (param.kind, param.min, param.max, param.default) {
                (ParamKind::Number, Some(min), Some(max), ParamValue::Number(default)) => {
                    if min >= max {
                        check.error(
                            format!("/params/{index}/max"),
                            "empty range",
                            format!("{min}..{max}"),
                            "a max above the min",
                        );
                    } else if default < min || default > max {
                        check.error(
                            format!("/params/{index}/default"),
                            "default out of range",
                            default,
                            format!("{min}..{max}"),
                        );
                    }
                }
                (ParamKind::Number, min, max, _) => {
                    if min.is_none() {
                        check.error(
                            format!("/params/{index}/min"),
                            "range missing",
                            "nothing",
                            "the least a number may be set to",
                        );
                    }
                    if max.is_none() {
                        check.error(
                            format!("/params/{index}/max"),
                            "range missing",
                            "nothing",
                            "the most a number may be set to",
                        );
                    }
                }
                (ParamKind::Color | ParamKind::Bool, min, max, _) => {
                    if min.is_some() || max.is_some() {
                        check.error(
                            format!("/params/{index}/min"),
                            "range on a flag or color",
                            "a min or max",
                            "no range",
                        );
                    }
                }
            }
        }
        let size = layout(&self.params)
            .last()
            .map_or(0, |slot| slot.offset + slot.kind.size());
        if size > PARAMS_BYTES {
            check.error(
                "/params",
                "parameters too large",
                format!("{size} bytes"),
                format!("at most {PARAMS_BYTES} bytes of parameters"),
            );
        }
    }
}

/// A parameter is a field of the shader's `Params` struct, so it must be a WGSL identifier that
/// is not a reserved word, nor one of the struct's padding fields, which alone start with `_`.
pub fn check_param_name(check: &mut RowCheck, pointer: &str, name: &str) {
    let identifier = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit() || c == '_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !identifier || naga::keywords::wgsl::RESERVED.contains(&name) {
        check.error(
            pointer,
            "invalid parameter name",
            name,
            "letters, digits and underscores, starting with a letter, not a WGSL reserved word",
        );
    }
}

pub fn compile_all(loading: &mut Loading) {
    let content = loading.content();
    let mut compiled = HashMap::new();
    for (id, def) in loading.rows::<VfxShaderDef>(&content) {
        let file = shader_file(id.path(&content));
        let source = match loading.read_text(&file) {
            Ok(source) => source,
            Err(_) => {
                loading.row_error(
                    id,
                    "",
                    ContentProblem::new("shader file missing", "nothing", &file),
                );
                continue;
            }
        };
        match def.check_source(&source) {
            Ok(composed) => {
                compiled.insert(id, composed);
            }
            Err(error) => loading.file_error(
                file,
                error.place,
                ContentProblem::new(
                    "shader does not compile",
                    error.message,
                    "WGSL defining fn vfx(uv: vec2<f32>) -> vec4<f32>",
                ),
            ),
        }
    }
    loading.built(CompiledVfxShaders(compiled));
}

impl SiteKind {
    fn name(self) -> &'static str {
        self.into()
    }
}

impl BodyShape {
    fn name(self) -> &'static str {
        self.into()
    }
}

impl Display for ShaderError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self.place {
            ErrorPlace::Text { line, column } => write!(f, "{line}:{column}: {}", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

/// The prelude every shader is composed with: the vertex output, the site bindings and the helpers
/// a shader reads the site through. It defines the fragment entry, which calls the shader's `vfx`.
const PRELUDE: &str = include_str!("prelude.wgsl");

fn layout(params: &[ShaderParam]) -> Vec<ParamSlot> {
    let mut cursor: usize = 0;
    params
        .iter()
        .map(|param| {
            let align = param.kind.size();
            let offset = cursor.div_ceil(align) * align;
            cursor = offset + param.kind.size();
            ParamSlot {
                name: param.name.clone(),
                kind: param.kind,
                offset,
                default: param.default,
            }
        })
        .collect()
}

// WebGL2 binds a uniform only in whole 16-byte rows, so `Params` is filled out to the full
// PARAMS_BYTES the uniform buffer spans, however few bytes the shader's own parameters take.
fn params_padding(params: &[ShaderParam]) -> String {
    let mut end = layout(params)
        .last()
        .map_or(0, |slot| slot.offset + slot.kind.size());
    let mut padding = String::new();
    while !end.is_multiple_of(16) {
        padding.push_str(&format!("    _pad{end}: f32,\n"));
        end += 4;
    }
    if end < PARAMS_BYTES {
        let rows = (PARAMS_BYTES - end) / 16;
        padding.push_str(&format!("    _rows: array<vec4<f32>, {rows}>,\n"));
    }
    padding
}

/// Parses and validates a composed shader as the GPU driver would, headless.
fn validate(composed: &str) -> Result<(), ShaderError> {
    let module = naga::front::wgsl::parse_str(composed).map_err(|error| ShaderError {
        place: place(error.location(composed)),
        message: error.message().to_owned(),
    })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module).map_err(|error| ShaderError {
        place: place(error.location(composed)),
        message: chain(error.as_inner()),
    })?;
    Ok(())
}

fn place(location: Option<naga::SourceLocation>) -> ErrorPlace {
    match location {
        Some(location) => ErrorPlace::Text {
            line: location.line_number as usize,
            column: location.line_position as usize,
        },
        None => ErrorPlace::WholeFile,
    }
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}
