//! Where a shader runs: the stage it is composed for. The effect stage draws the layers of a vfx,
//! the text stage the plates the game letters; the features of a game register more, which a
//! system draws in a material of its own.

use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use bevy::shader::ShaderDefVal;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::shader::{
    effect_check_header, effect_header, effect_prelude, quad_header, site_prelude,
};

/// A shader that draws nothing, which every stage on a site has the function of.
pub const BLANK_VFX: &str = "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n";
/// An effect shader that draws nothing.
pub const BLANK_FX: &str = "fn fx(i: FxIn) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n";

/// The bind group a text quad's own bindings sit in, drawn over the presented frame.
const PRESENT_GROUP: u32 = 2;

/// A kind of place a shader runs, by the name its row gives it.
#[derive(Clone, Copy)]
pub struct Stage(&'static StageSpec);

/// What a stage composes its shaders with and where they run.
pub struct StageSpec {
    pub name: &'static str,
    pub kind: StageKind,
    /// The header a shader of the stage is composed with at run time, WGSL or, when `wesl` says
    /// so, WESL.
    pub header: fn() -> String,
    /// The header the validator composes a shader of the stage with: plain WGSL that declares what
    /// the runtime header declares and calls what the shader must define.
    pub check_header: fn() -> String,
    /// The helpers a shader of the stage is composed with after its header, at run time and for
    /// the validator alike.
    pub prelude: fn() -> String,
    /// The functions a shader of the stage must define.
    pub required: &'static [&'static str],
    /// A shader of the stage that defines them all and draws nothing.
    pub blank: &'static str,
    /// Whether the composed shader is WESL, importing the engine's own shader libraries, rather
    /// than plain WGSL.
    pub wesl: bool,
    /// What the stage's header gives a shader to read, beyond what the header and prelude declare
    /// as functions and constants: what an editor highlights as the vfx vocabulary.
    pub names: &'static [&'static str],
    /// The passes the stage's shaders draw in, each with the defs its pipeline compiles it under.
    pub passes: fn() -> Vec<ShaderPass>,
}

/// How the engine draws a stage's shaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    /// On the shapes, coats and patches of ground of a vfx's layers, in the world.
    Effect,
    /// In the text of a plate.
    Text,
    /// In a material of a system's own, which takes no site of a vfx.
    Pass,
}

/// A pass a shader draws in, and the defs its pipeline compiles it under.
pub struct ShaderPass {
    pub name: &'static str,
    pub defs: Vec<ShaderDefVal>,
}

/// The stages a game's shaders are written for: the effect and text stages, and those its
/// features register. Installed once, as serde and schemars, which read and describe shader rows,
/// take no context to find it in.
pub struct Stages {
    stages: Vec<Stage>,
}

static INSTALLED: OnceLock<Stages> = OnceLock::new();

impl Stage {
    pub const EFFECT: Stage = Stage(&StageSpec {
        name: "effect",
        kind: StageKind::Effect,
        header: effect_header,
        check_header: effect_check_header,
        prelude: effect_prelude,
        required: &["fx"],
        blank: BLANK_FX,
        wesl: true,
        names: &[
            "FxIn", "uv", "world", "local", "normal", "view", "life", "age", "seed", "front",
            "pixel", "size", "on", "frame", "params",
        ],
        passes: effect_passes,
    });
    pub const TEXT: Stage = Stage(&StageSpec {
        name: "text",
        kind: StageKind::Text,
        header: present_quad_header,
        check_header: present_quad_header,
        prelude: site_prelude,
        required: &["vfx"],
        blank: BLANK_VFX,
        wesl: false,
        names: &["site", "params", "sample"],
        passes: no_passes,
    });

    /// The stage `spec` describes, for a feature to register: a stage drawn by a system in a
    /// material of its own.
    pub const fn registered(spec: &'static StageSpec) -> Stage {
        Stage(spec)
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    pub fn kind(self) -> StageKind {
        self.0.kind
    }

    pub fn spec(self) -> &'static StageSpec {
        self.0
    }

    pub fn label(self) -> &'static str {
        self.0.name
    }

    /// Whether the composed shader is WESL, importing the engine's own shader libraries, rather
    /// than plain WGSL.
    pub fn composes_wesl(self) -> bool {
        self.0.wesl
    }
}

impl Stages {
    /// The stages of the game, once `install` made them.
    pub fn get() -> &'static Stages {
        INSTALLED
            .get()
            .expect("the shader stages are installed before a shader is read or described")
    }

    /// Installs the stages `register` adds to the built-in ones, unless some are installed already.
    pub fn install(register: impl FnOnce(&mut Stages)) -> &'static Stages {
        INSTALLED.get_or_init(|| {
            let mut stages = Stages {
                stages: vec![Stage::EFFECT, Stage::TEXT],
            };
            register(&mut stages);
            stages
        })
    }

    pub fn add(&mut self, stage: Stage) -> &mut Stages {
        assert!(
            self.stages.iter().all(|known| known.name() != stage.name()),
            "two shader stages are called {}",
            stage.name()
        );
        assert_eq!(
            stage.kind(),
            StageKind::Pass,
            "{} is drawn by the engine itself, which only the built-in stages are",
            stage.name()
        );
        self.stages.push(stage);
        self
    }

    pub fn all(&self) -> &[Stage] {
        &self.stages
    }

    /// Every name the stages' headers give a shader to read beyond the functions and constants
    /// they declare.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.stages
            .iter()
            .flat_map(|stage| stage.0.names.iter().copied())
    }

    fn named(&self, name: &str) -> Option<Stage> {
        self.stages
            .iter()
            .find(|stage| stage.name() == name)
            .copied()
    }
}

/// The passes a shader of a lit mesh draws in: the main pass alone.
pub fn main_pass() -> Vec<ShaderPass> {
    vec![ShaderPass {
        name: "main",
        defs: main_defs(),
    }]
}

pub fn no_passes() -> Vec<ShaderPass> {
    Vec::new()
}

/// The defs a lit mesh with positions, normals, uvs and colours is compiled under in the main
/// pass.
pub fn main_defs() -> Vec<ShaderDefVal> {
    vec![
        ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            bevy::pbr::MATERIAL_BIND_GROUP_INDEX as u32,
        ),
        "MESH_PIPELINE".into(),
        "VERTEX_OUTPUT_INSTANCE_INDEX".into(),
        "VERTEX_POSITIONS".into(),
        "VERTEX_NORMALS".into(),
        "VERTEX_UVS".into(),
        "VERTEX_UVS_A".into(),
        "VERTEX_COLORS".into(),
        "VIEW_PROJECTION_PERSPECTIVE".into(),
        "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(),
    ]
}

/// The defs of the prepass, where a stage writes depth and normals ahead of the main pass.
pub fn prepass_defs() -> Vec<ShaderDefVal> {
    with_defs(
        main_defs(),
        &["PREPASS_PIPELINE", "DEPTH_PREPASS", "NORMAL_PREPASS"],
    )
}

/// `base` with the defs `more` on top.
pub fn with_defs(base: Vec<ShaderDefVal>, more: &[&'static str]) -> Vec<ShaderDefVal> {
    base.into_iter()
        .chain(more.iter().map(|def| ShaderDefVal::from(*def)))
        .collect()
}

/// An effect is drawn in the main pass alone, adding its light or blended over what lies behind
/// it, and softened where it meets what stands behind it when the view has a depth prepass.
fn effect_passes() -> Vec<ShaderPass> {
    vec![
        ShaderPass {
            name: "blended",
            defs: main_defs(),
        },
        ShaderPass {
            name: "added",
            defs: with_defs(main_defs(), &["FX_ADD"]),
        },
        ShaderPass {
            name: "softened",
            defs: with_defs(main_defs(), &["DEPTH_PREPASS"]),
        },
    ]
}

fn present_quad_header() -> String {
    quad_header(PRESENT_GROUP)
}

impl PartialEq for Stage {
    fn eq(&self, other: &Stage) -> bool {
        self.name() == other.name()
    }
}

impl Eq for Stage {}

impl Hash for Stage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

impl Debug for Stage {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Stage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Stage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Stage, D::Error> {
        let name = String::deserialize(deserializer)?;
        let stages = Stages::get();
        stages.named(&name).ok_or_else(|| {
            let known: Vec<String> = stages
                .all()
                .iter()
                .map(|stage| format!("`{}`", stage.name()))
                .collect();
            D::Error::custom(format!(
                "unknown variant `{name}`, expected one of {}",
                known.join(", ")
            ))
        })
    }
}

impl JsonSchema for Stage {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Stage".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<&str> = Stages::get()
            .all()
            .iter()
            .map(|stage| stage.name())
            .collect();
        json_schema!({ "type": "string", "enum": names })
    }
}
