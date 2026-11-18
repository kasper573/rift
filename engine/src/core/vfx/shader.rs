use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use super::uniform::MAX_PATH_POINTS;
use super::{ParamValue, Stage, Stages};
use crate::core::content::fixture::FixtureLock;
use crate::core::content::{
    Bundle, ContentFile, ContentId, ContentProblem, ErrorPlace, FileShape, FileWriter, Loading,
    NamedContent, Nearby, PartSpec, Parts, RowCheck,
};
use crate::core::math::Rgba;

pub type VfxShaderId = ContentId<VfxShaderDef>;

/// The bytes the parameter uniform spans, however few parameters a shader declares.
pub const PARAMS_BYTES: usize = 256;

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
    /// A parameter of `kind` that holds its kind's blank value until something fills it.
    pub fn of(name: &str, kind: ParamKind) -> ShaderParam {
        ShaderParam {
            name: name.to_owned(),
            kind,
            default: kind.blank(),
            min: None,
            max: None,
        }
    }

    /// The range a number parameter is set within; a flag or a color has none.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.min?, self.max?))
    }
}

impl VfxShaderDef {
    /// What the engine reads of the file of a fixture shader that runs on `stage` and reads
    /// `params`: content tunes the rest, but never takes these away.
    pub fn fixture_locks(stage: Stage, params: &[ShaderParam]) -> Vec<FixtureLock> {
        let stage = FixtureLock::field(crate::field!(VfxShaderDef, stage), &stage);
        std::iter::once(stage)
            .chain(params.iter().map(|param| {
                FixtureLock::entry(
                    crate::field!(VfxShaderDef, params),
                    param,
                    &[
                        crate::field!(ShaderParam, name),
                        crate::field!(ShaderParam, kind),
                    ],
                )
            }))
            .collect()
    }

    /// What a designer's fixture shader holds where it is missing: a fresh shader that runs on
    /// `stage` and reads `params`.
    pub fn fixture_fresh(
        near: &Nearby,
        stage: Stage,
        params: Vec<ShaderParam>,
    ) -> Result<VfxShaderDef, String> {
        Ok(VfxShaderDef {
            stage,
            params,
            ..VfxShaderDef::fresh(near)?
        })
    }

    pub fn param(&self, name: &str) -> Option<&ShaderParam> {
        self.params.iter().find(|param| param.name == name)
    }

    /// Checks a value filled in for the parameter `name` at `at`: one the shader declares, of its
    /// kind and within its range; a value left to whoever plays the shader is `None`.
    pub fn check_fill(
        &self,
        check: &mut RowCheck,
        at: &str,
        name: &str,
        value: Option<&ParamValue>,
    ) {
        let Some(declared) = self.param(name) else {
            check.error(
                at,
                "unknown shader parameter",
                name,
                format!(
                    "one of {}",
                    self.params
                        .iter()
                        .map(|param| param.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            return;
        };
        let Some(value) = value else {
            return;
        };
        if value.kind() != declared.kind {
            check.error(
                at,
                "wrong parameter kind",
                value.kind().label(),
                format!("a {}", declared.kind.label()),
            );
        }
        if let (ParamValue::Number(number), Some((min, max))) = (value, declared.range())
            && (*number < min || *number > max)
        {
            check.error(
                at,
                "parameter out of range",
                number,
                format!("{min}..{max}"),
            );
        }
    }

    pub fn layout(&self) -> Vec<ParamSlot> {
        layout(&self.params)
    }

    /// The shader's text as the pipeline compiles it for its stage: the row's file first, so its
    /// lines keep their numbers, then the header of the stage it runs on, the prelude, the site's
    /// constants and the struct its parameters are read through.
    pub fn compose(&self, source: &str) -> String {
        self.composed(&(self.stage.spec().header)(), source)
    }

    /// The source goes first so its lines keep their numbers in what the validator reports; a WESL
    /// header's imports alone go before it, as the module must open with them.
    fn composed(&self, header: &str, source: &str) -> String {
        let prelude = prelude();
        let mut composed = String::with_capacity(source.len() + header.len() + prelude.len() + 512);
        let (imports, declarations) = split_imports(header);
        for import in imports {
            composed.push_str(import);
            composed.push('\n');
        }
        composed.push_str(source);
        if !source.ends_with('\n') {
            composed.push('\n');
        }
        for line in declarations {
            composed.push_str(line);
            composed.push('\n');
        }
        composed.push_str(&prelude);
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
        // WGSL has no empty struct.
        if self.params.is_empty() {
            composed.push_str("    _none: vec4<f32>,\n");
        }
        composed.push_str("}\n");
        composed
    }

    pub fn check_source(&self, source: &str) -> Result<String, ShaderError> {
        for required in self.stage.spec().required {
            if !source.contains(&format!("fn {required}(")) {
                return Err(ShaderError {
                    place: ErrorPlace::WholeFile,
                    message: format!("no {required} function"),
                });
            }
        }
        let checked = self.composed(&(self.stage.spec().check_header)(), source);
        let own_lines = source.lines().count();
        validate(&checked).map_err(|error| ShaderError {
            place: match error.place {
                ErrorPlace::Text { line, .. } if line > own_lines => ErrorPlace::WholeFile,
                place => place,
            },
            ..error
        })?;
        Ok(self.compose(source))
    }
}

/// Every name the prelude, the composed constants and the headers of the stages give a shader to
/// read: what an editor highlights as the vfx vocabulary.
pub fn prelude_names() -> Vec<String> {
    let mut names: Vec<String> = ["vfx", "site", "params", "sample"]
        .into_iter()
        .chain(Stages::get().names())
        .map(str::to_owned)
        .collect();
    for line in prelude().lines() {
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
    const SHAPE: FileShape = FileShape::Bundle;

    fn fresh(near: &Nearby) -> Result<VfxShaderDef, String> {
        Ok(VfxShaderDef {
            name: near.name(),
            stage: Stage::BILLBOARD,
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
/// is not a reserved word, nor the field standing in for none, which alone starts with `_`.
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

/// The prelude every shader is composed with: the site and the helpers a shader reads it through.
const PRELUDE: &str = include_str!("prelude.wgsl");
/// The header of a shader drawn on a quad: its vertex output, its bindings in the group `GROUP`
/// stands for, `sample` and the fragment entry, which calls the shader's `vfx`.
const QUAD_HEADER: &str = include_str!("quad.wgsl");
/// The header of a shader run in the material of the thing it plays on.
const SURFACE_HEADER: &str = include_str!("surface.wesl");
/// Numbers drawn from seeds, which the prelude and the helpers of features read alike.
const HASH_HELPERS: &str = include_str!("hash.wgsl");
/// The frame's globals as the validator sees them, where the runtime headers import bevy's.
pub const CLOCK_CHECK: &str = "struct Globals {\n    time: f32,\n    delta_time: f32,\n    frame_count: u32,\n}\n@group(0) @binding(11) var<uniform> globals: Globals;";

/// The hashes helpers of features may draw on, for a shader composed with them and no prelude.
pub fn hash_helpers() -> &'static str {
    HASH_HELPERS
}

fn prelude() -> String {
    format!(
        "{}\n{HASH_HELPERS}",
        PRELUDE.replace("PATH_POINTS", &format!("{MAX_PATH_POINTS}u"))
    )
}

pub(super) fn quad_header(group: u32) -> String {
    QUAD_HEADER.replace("GROUP", &group.to_string())
}

pub(super) fn surface_header() -> String {
    SURFACE_HEADER.to_owned()
}

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

/// A header's import lines, each with the `@if`/`@elif`/`@else` line that guards it, apart from
/// its declarations, so the imports can lead the composed shader.
fn split_imports(header: &str) -> (Vec<&str>, Vec<&str>) {
    let mut imports = Vec::new();
    let mut declarations = Vec::new();
    let mut guard: Option<&str> = None;
    for line in header.lines() {
        if line.starts_with("@if(") || line.starts_with("@elif(") || line == "@else" {
            if let Some(earlier) = guard.replace(line) {
                declarations.push(earlier);
            }
            continue;
        }
        let into = if line.starts_with("import ") {
            &mut imports
        } else {
            &mut declarations
        };
        into.extend(guard.take());
        into.push(line);
    }
    declarations.extend(guard);
    (imports, declarations)
}
