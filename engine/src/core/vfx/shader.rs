use std::collections::HashMap;
use std::fmt::Display;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ParamValue;
use crate::core::content::{
    ContentId, ContentProblem, ContentRow, ErrorPlace, Loading, RowCheck, asset_file,
};

pub type VfxShaderId = ContentId<VfxShaderDef>;

/// The prelude every shader is composed with: the vertex output, the site bindings and the helpers
/// a shader reads the site through. It defines the fragment entry, which calls the shader's `vfx`.
pub const PRELUDE: &str = include_str!("prelude.wgsl");

/// The bytes the parameter uniform spans, however few parameters a shader declares.
pub const PARAMS_BYTES: usize = 128;

const RESERVED: [&str; 4] = ["site", "params", "sample", "site_uv"];

/// A shader a game ships: the WGSL file at `shader_file(key)` defining
/// `fn vfx(uv: vec2<f32>) -> vec4<f32>`, and the parameters it reads from `params`, each with the
/// default it gets when a layer leaves it unfilled and, for a number, the range the designer means
/// it to be set within. A flag is `true` or `false` in the data and 0.0 or 1.0 in the shader.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxShaderDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ShaderParam>,
}

/// Where a shader row's WGSL lives: beside the other assets, named after the row.
pub fn shader_file(key: &str) -> String {
    asset_file(&format!("shaders/{key}.wgsl"))
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

impl ShaderParam {
    /// The range a number parameter is set within; a color has none.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.min?, self.max?))
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Number,
    Bool,
    Color,
}

/// Where each parameter sits in the uniform, by the layout rules WGSL gives the generated struct.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSlot {
    pub name: String,
    pub kind: ParamKind,
    pub offset: usize,
}

/// The composed source of every shader row, built once at load so every client compiles the same
/// text the load validated.
pub struct CompiledVfxShaders(pub HashMap<VfxShaderId, String>);

#[derive(Clone, Debug, PartialEq)]
pub struct ShaderError {
    pub line: Option<(usize, usize)>,
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

impl VfxShaderDef {
    pub fn param(&self, name: &str) -> Option<&ShaderParam> {
        self.params.iter().find(|param| param.name == name)
    }

    pub fn layout(&self) -> Vec<ParamSlot> {
        layout(&self.params)
    }

    /// The shader's text as the pipeline compiles it: the row's file first, so its lines keep their
    /// numbers, then the prelude and the struct its parameters are read through.
    pub fn compose(&self, source: &str) -> String {
        let mut composed = String::with_capacity(source.len() + PRELUDE.len() + 256);
        composed.push_str(source);
        if !source.ends_with('\n') {
            composed.push('\n');
        }
        composed.push_str(PRELUDE);
        composed.push_str("\nstruct Params {\n");
        if self.params.is_empty() {
            composed.push_str("    unused: f32,\n");
        }
        for param in &self.params {
            composed.push_str(&format!("    {}: {},\n", param.name, param.kind.wgsl()));
        }
        composed.push_str("}\n");
        composed
    }

    pub fn check_source(&self, source: &str) -> Result<String, ShaderError> {
        if !source.contains("fn vfx(") {
            return Err(ShaderError {
                line: None,
                message: "no vfx function".to_owned(),
            });
        }
        let composed = self.compose(source);
        let own_lines = source.lines().count();
        validate(&composed).map_err(|error| ShaderError {
            line: error.line.filter(|(line, _)| *line <= own_lines),
            ..error
        })?;
        Ok(composed)
    }
}

impl ContentRow for VfxShaderDef {
    const TABLE: &'static str = "vfx_shader";

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

pub fn check_param_name(check: &mut RowCheck, pointer: &str, name: &str) {
    let identifier = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !identifier || RESERVED.contains(&name) {
        check.error(
            pointer,
            "invalid parameter name",
            name,
            "letters, digits and underscores, not starting with a digit, not a reserved name",
        );
    }
}

pub fn layout(params: &[ShaderParam]) -> Vec<ParamSlot> {
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
            }
        })
        .collect()
}

/// Parses and validates a composed shader as the GPU driver would, headless.
pub fn validate(composed: &str) -> Result<(), ShaderError> {
    let module = naga::front::wgsl::parse_str(composed).map_err(|error| ShaderError {
        line: error.location(composed).map(place),
        message: error.message().to_owned(),
    })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module).map_err(|error| ShaderError {
        line: error.location(composed).map(place),
        message: chain(error.as_inner()),
    })?;
    Ok(())
}

pub fn compile_all(loading: &mut Loading) {
    let content = loading.content();
    let mut compiled = HashMap::new();
    for (id, def) in loading.rows::<VfxShaderDef>(&content) {
        let file = shader_file(id.key(&content));
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
                match error.line {
                    Some((line, column)) => ErrorPlace::Text { line, column },
                    None => ErrorPlace::WholeFile,
                },
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

fn place(location: naga::SourceLocation) -> (usize, usize) {
    (
        location.line_number as usize,
        location.line_position as usize,
    )
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

impl Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.line {
            Some((line, column)) => write!(f, "{line}:{column}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}
