//! What a game's shader reads beside its mesh: its parameters, the site it draws on, and the key its
//! pipeline is built for.

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

/// A shader's parameters laid out as its `Params` struct, built by `ParamsUniform::pack`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsUniform {
    words: [Vec4; PARAMS_BYTES / 16],
}

impl ParamsUniform {
    /// Each parameter the shader declares from `value`, or its default when that gives nothing or
    /// a value of another kind, laid out as the shader's `Params` struct.
    pub fn pack(
        shader: &VfxShaderDef,
        value: impl Fn(&str) -> Option<ParamValue>,
    ) -> ParamsUniform {
        let mut floats = [0.0f32; PARAMS_BYTES / 4];
        for slot in shader.layout() {
            let value = value(&slot.name)
                .filter(|value| value.kind() == slot.kind)
                .unwrap_or(slot.default);
            let at = slot.offset / 4;
            match (slot.kind, value) {
                (ParamKind::Number, ParamValue::Number(number)) => floats[at] = number,
                (ParamKind::Bool, ParamValue::Bool(flag)) => floats[at] = f32::from(u8::from(flag)),
                (ParamKind::Color, ParamValue::Color(color)) => {
                    let linear = color.color().to_linear();
                    floats[at..at + 4].copy_from_slice(&[
                        linear.red,
                        linear.green,
                        linear.blue,
                        linear.alpha,
                    ]);
                }
                _ => {}
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
