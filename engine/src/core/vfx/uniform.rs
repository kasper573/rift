//! What a game's shader reads beside its mesh: its parameters, the site it draws on or the frame
//! of the effect layer it draws, and the key its pipeline is built for.

use std::marker::PhantomData;

use bevy::asset::uuid::Uuid;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::shader::Shader;

use super::shader::PARAMS_BYTES;
use super::{ParamKind, ParamValue, VfxShaderDef};

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

/// The site as the prelude's `site` reads it, built by `SiteFrame::uniform`.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SiteUniform {
    pub(super) rect: Vec4,
    pub(super) size: Vec2,
    pub(super) pixel: Vec2,
    pub(super) kind: u32,
    pub(super) facing: f32,
    pub(super) heading: f32,
    pub(super) progress: f32,
    pub(super) time: f32,
    pub(super) overscan: Vec2,
    pub(super) shape: u32,
    pub(super) point_count: u32,
    pub(super) image: u32,
    pub(super) faded: f32,
    pub(super) origin: Vec2,
    pub(super) extent: Vec2,
    pub(super) metres: Vec2,
    pub(super) points: [Vec4; MAX_PATH_POINTS / 2],
}

/// Where an effect layer stands and how far through its run it is, as `effect_frame.wesl` reads
/// it.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectFrame {
    pub origin: Vec4,
    pub rotation: Vec4,
    pub scale: Vec4,
    pub life: Vec4,
    pub site: Vec4,
    pub extra: Vec4,
    pub mask: Vec4,
}

/// A shader's parameters laid out as its `Params` struct, built by `ParamsUniform::pack`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsUniform {
    words: [Vec4; PARAMS_BYTES / 16],
}

/// A value a shader's parameter is filled with: a number, a flag, or a colour already in linear
/// light, above 1 glowing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderValue {
    Number(f32),
    Bool(bool),
    Light(LinearRgba),
}

impl From<ParamValue> for ShaderValue {
    fn from(value: ParamValue) -> ShaderValue {
        match value {
            ParamValue::Number(number) => ShaderValue::Number(number),
            ParamValue::Bool(flag) => ShaderValue::Bool(flag),
            ParamValue::Color(color) => ShaderValue::Light(color.color().to_linear()),
        }
    }
}

impl ShaderValue {
    fn kind(self) -> ParamKind {
        match self {
            ShaderValue::Number(_) => ParamKind::Number,
            ShaderValue::Bool(_) => ParamKind::Bool,
            ShaderValue::Light(_) => ParamKind::Color,
        }
    }
}

impl ParamsUniform {
    /// Each parameter the shader declares from `value`, or its default when that gives nothing or
    /// a value of another kind, laid out as the shader's `Params` struct.
    pub fn pack(
        shader: &VfxShaderDef,
        value: impl Fn(&str) -> Option<ShaderValue>,
    ) -> ParamsUniform {
        let mut floats = [0.0f32; PARAMS_BYTES / 4];
        for slot in shader.layout() {
            let value = value(&slot.name)
                .filter(|value| value.kind() == slot.kind)
                .unwrap_or(ShaderValue::from(slot.default));
            let at = slot.offset / 4;
            match value {
                ShaderValue::Number(number) => floats[at] = number,
                ShaderValue::Bool(flag) => floats[at] = f32::from(u8::from(flag)),
                ShaderValue::Light(light) => {
                    floats[at..at + 4].copy_from_slice(&light.to_f32_array());
                }
            }
        }
        let mut words = [Vec4::ZERO; PARAMS_BYTES / 16];
        let (chunks, _) = floats.as_chunks::<4>();
        for (word, chunk) in words.iter_mut().zip(chunks) {
            *word = Vec4::from_array(*chunk);
        }
        ParamsUniform { words }
    }

    /// The parameters bit for bit, so materials filled alike can be told apart from others.
    pub fn bits(&self) -> [u32; PARAMS_BYTES / 4] {
        let mut bits = [0; PARAMS_BYTES / 4];
        for (word, value) in self.words.iter().flat_map(Vec4::to_array).enumerate() {
            bits[word] = value.to_bits();
        }
        bits
    }
}

impl Default for ParamsUniform {
    fn default() -> ParamsUniform {
        ParamsUniform {
            words: [Vec4::ZERO; PARAMS_BYTES / 16],
        }
    }
}

/// Which compiled shader a material draws with, as its pipeline is specialized by. Every compiled
/// shader is held under a uuid, so the key copies and the pipeline finds the shader by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VfxShaderKey(Uuid);

impl VfxShaderKey {
    pub fn of(shader: &Handle<Shader>) -> VfxShaderKey {
        VfxShaderKey(match shader.id() {
            AssetId::Uuid { uuid } => uuid,
            AssetId::Index { .. } => unreachable!("every compiled shader is held under a uuid"),
        })
    }

    pub fn handle(self) -> Handle<Shader> {
        Handle::Uuid(self.0, PhantomData)
    }
}
