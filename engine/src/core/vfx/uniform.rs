//! What a game's shader reads beside its mesh: its parameters, the site it draws on, the weather
//! the engine publishes, and the key its pipeline is built for.

use std::marker::PhantomData;

use bevy::asset::RenderAssetUsages;
use bevy::asset::uuid::Uuid;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::Shader;

use super::shader::PARAMS_BYTES;
use super::{ParamKind, ParamValue, VfxShaderDef};

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

/// How many limbs of a treading body a cover bends round: its lowest, which reach into it first.
pub const LIMBS_PER_TREAD: usize = 32;

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

/// The weather as every cover reads it: a picture a texel per value, painted over in place each
/// frame. The covers' materials stand unchanged while the wind blows and bodies tread through
/// them, so none of their meshes is drawn up again for it.
#[derive(Resource)]
pub struct WeatherImage(pub Handle<Image>);

impl FromWorld for WeatherImage {
    fn from_world(world: &mut World) -> WeatherImage {
        WeatherImage(
            world
                .resource_mut::<Assets<Image>>()
                .add(Weather::default().image()),
        )
    }
}

/// The weather as the cover header's `weather` reads it: the wind as a direction scaled by its
/// strength, the moment, how far through its day the game is and how clouded the sky, and the
/// bodies treading through.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Weather {
    pub(super) wind: Vec2,
    pub(super) time: f32,
    pub(super) daylight: f32,
    pub(super) clouds: f32,
    pub(super) water: f32,
    /// At most [`TREADS`].
    pub(super) treads: Vec<Tread>,
}

/// A body treading through the cover: the ground point it stands on (x east, y south), how far
/// from it its limbs reach over the ground, and its limbs, at most [`LIMBS_PER_TREAD`].
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tread {
    pub(super) ground: Vec2,
    pub(super) reach: f32,
    pub(super) limbs: Vec<TreadingLimb>,
}

/// A limb as it stands in the world: its bone from `from` to `to`, and how far round it its flesh
/// lies, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TreadingLimb {
    pub(super) from: Vec3,
    pub(super) to: Vec3,
    pub(super) girth: f32,
}

impl Default for Weather {
    fn default() -> Weather {
        Weather {
            wind: Vec2::ZERO,
            time: 0.0,
            daylight: 0.5,
            clouds: 0.0,
            water: 0.0,
            treads: Vec::new(),
        }
    }
}

/// Texels the weather's picture holds: the climate's two, a texel a tread, and two a limb.
const WEATHER_TEXELS: usize = 2 + TREADS + TREADS * LIMBS_PER_TREAD * 2;

impl Weather {
    pub(super) fn image(&self) -> Image {
        Image::new(
            Extent3d {
                width: WEATHER_TEXELS as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.texels(),
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        )
    }

    /// The weather laid out as the cover header reads it: the wind, the moment and the count of
    /// treads in the first texel, the daylight, the cloud and its water in the second, then a
    /// texel a tread, its ground point, its reach and its count of limbs, and after every tread's,
    /// [`LIMBS_PER_TREAD`] places of two texels for each tread's limbs, a bone's start and girth,
    /// then its end.
    pub(super) fn texels(&self) -> Vec<u8> {
        let mut texels = vec![Vec4::ZERO; WEATHER_TEXELS];
        texels[0] = Vec4::new(
            self.wind.x,
            self.wind.y,
            self.time,
            self.treads.len() as f32,
        );
        texels[1] = Vec4::new(self.daylight, self.clouds, self.water, 0.0);
        for (index, tread) in self.treads.iter().take(TREADS).enumerate() {
            let limbs = &tread.limbs[..tread.limbs.len().min(LIMBS_PER_TREAD)];
            texels[2 + index] = tread.ground.extend(tread.reach).extend(limbs.len() as f32);
            let first = 2 + TREADS + index * LIMBS_PER_TREAD * 2;
            for (slot, limb) in limbs.iter().enumerate() {
                texels[first + slot * 2] = limb.from.extend(limb.girth);
                texels[first + slot * 2 + 1] = limb.to.extend(0.0);
            }
        }
        texels
            .iter()
            .flat_map(Vec4::to_array)
            .flat_map(f32::to_le_bytes)
            .collect()
    }
}
