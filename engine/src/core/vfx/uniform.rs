//! What a game's shader reads beside its mesh: its parameters, the site it draws on, the weather
//! the engine publishes, and the key its pipeline is built for.

use std::marker::PhantomData;

use bevy::asset::RenderAssetUsages;
use bevy::asset::uuid::Uuid;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::Shader;

use super::rain::{RAIN_CELL, RAIN_SIDE, RainField, SHELTER_CELL, SHELTER_SIDE, Shelter};
use super::shader::PARAMS_BYTES;
use super::{ParamKind, ParamValue, VfxShaderDef};

/// How many points of a path site a shader can read; a longer path keeps its first ones.
pub const MAX_PATH_POINTS: usize = 16;

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

/// How many limbs of a treading body a cover bends round: its lowest, which reach into it first.
pub const LIMBS_PER_TREAD: usize = 32;

/// How many of the places a treading body last stood a cover reads, the newest first, and how
/// long apart they are laid: the trail a body leaves through water for its wake.
pub const TRAIL_POINTS: usize = 12;
pub const TRAIL_EVERY: f32 = 0.25;

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

/// The weather as every cover reads it: a picture a texel per value, its changed texels written
/// over in place on the GPU, so a frame in which nothing of it changed writes nothing. The covers'
/// materials stand unchanged while the wind blows and bodies tread through them, so none of their
/// meshes is drawn up again for it.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct WeatherImage(pub Handle<Image>);

impl FromWorld for WeatherImage {
    fn from_world(world: &mut World) -> WeatherImage {
        WeatherImage(world.resource_mut::<Assets<Image>>().add(Weather::image()))
    }
}

/// The weather as the headers' `weather` reads it: the wind as a direction scaled by its
/// strength, how far through its day the game is, how clouded the sky, how much water
/// the cloud holds, how hard it rains under the wettest and how brightly lightning lights the
/// world, and the bodies treading through.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Weather {
    pub(super) wind: Vec2,
    pub(super) daylight: f32,
    pub(super) clouds: f32,
    pub(super) water: f32,
    pub(super) rain: f32,
    pub(super) flash: f32,
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
    /// Where it stood lately and when, the newest first, at most [`TRAIL_POINTS`].
    pub(super) trail: Vec<(Vec2, f32)>,
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
            daylight: 0.5,
            clouds: 0.0,
            water: 0.0,
            rain: 0.0,
            flash: 0.0,
            treads: Vec::new(),
        }
    }
}

/// How many texels a row of the weather's picture holds: it is read in order, row by row.
pub(super) const PICTURE_WIDTH: usize = 256;
/// Where the first tread's texel lies: after the climate's, the storm's, where the rain field and
/// the shelter lie, and the most the field holds.
const FIRST_TREAD: usize = 6;
const FIRST_LIMB: usize = FIRST_TREAD + TREADS;
const FIRST_TRAIL: usize = FIRST_LIMB + TREADS * LIMBS_PER_TREAD * 2;
const FIRST_RAIN: usize = FIRST_TRAIL + TREADS * TRAIL_POINTS;
const FIRST_SHELTER: usize = FIRST_RAIN + RAIN_SIDE * RAIN_SIDE;
/// Texels the weather's picture holds: the climate's two, the storm's, where the rain field and
/// the shelter lie, a texel a tread, two a limb, one a point of a tread's trail, one a cell of the
/// rain field and one every four of the shelter's, filling out its last row.
const WEATHER_TEXELS: usize =
    (FIRST_SHELTER + SHELTER_SIDE * SHELTER_SIDE / 4).div_ceil(PICTURE_WIDTH) * PICTURE_WIDTH;

impl Weather {
    /// The texels of the picture as it is first made, before any weather is known.
    pub(super) fn still() -> Vec<Vec4> {
        Weather::default().texels(&RainField::default(), &Shelter::default())
    }

    /// The picture as it is first made, holding `still`'s texels.
    pub(super) fn image() -> Image {
        Image::new(
            Extent3d {
                width: PICTURE_WIDTH as u32,
                height: (WEATHER_TEXELS / PICTURE_WIDTH) as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            texel_bytes(&Weather::still()),
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        )
    }

    /// The weather laid out as the headers read it, texel by texel, row by row: the wind and the
    /// count of treads in the first texel, the daylight, the cloud, its water and
    /// the rain in the second; the flash and where the rain field's, the shelter's and the
    /// treads' first texels lie in the third; where the rain field and the shelter lie on the
    /// ground in the fourth and fifth, their north-west corner, a cell's side and the cells each
    /// way, or nothing for one not held; the wettest ground and the hardest rain over the field in
    /// the sixth, so a shader skips reading the field where it lies dry; then a texel a tread, its ground point, its reach and its
    /// count of limbs, and after every tread's, [`LIMBS_PER_TREAD`] places of two texels for each
    /// tread's limbs, a bone's start and girth, then its end, and after those [`TRAIL_POINTS`]
    /// texels for each tread's trail, a point's place, when it was laid and 1, or nothing past the
    /// trail's end; then a texel a cell of the rain field, its share of the downpour, its
    /// wetness and its rain, and the tops of the shelter's cells, four to a texel.
    pub(super) fn texels(&self, rain: &RainField, shelter: &Shelter) -> Vec<Vec4> {
        let mut texels = vec![Vec4::ZERO; WEATHER_TEXELS];
        texels[0] = Vec4::new(self.wind.x, self.wind.y, 0.0, self.treads.len() as f32);
        texels[1] = Vec4::new(self.daylight, self.clouds, self.water, self.rain);
        texels[2] = Vec4::new(
            self.flash,
            FIRST_RAIN as f32,
            FIRST_SHELTER as f32,
            FIRST_TREAD as f32,
        );
        if rain.cells.len() == RAIN_SIDE * RAIN_SIDE {
            texels[3] = rain.corner.extend(RAIN_CELL.0).extend(RAIN_SIDE as f32);
            texels[5] = rain.cells.iter().fold(Vec4::ZERO, |most, cell| {
                most.max(Vec4::new(cell.wetness, cell.rain, 0.0, 0.0))
            });
            for (texel, cell) in texels[FIRST_RAIN..].iter_mut().zip(&rain.cells) {
                *texel = Vec4::new(cell.share, cell.wetness, cell.rain, 0.0);
            }
        }
        if shelter.tops.len() == SHELTER_SIDE * SHELTER_SIDE {
            texels[4] = shelter
                .corner
                .extend(SHELTER_CELL.0)
                .extend(SHELTER_SIDE as f32);
            let (fours, _) = shelter.tops.as_chunks::<4>();
            for (texel, four) in texels[FIRST_SHELTER..].iter_mut().zip(fours) {
                *texel = Vec4::from_array(*four);
            }
        }
        for (index, tread) in self.treads.iter().take(TREADS).enumerate() {
            let limbs = &tread.limbs[..tread.limbs.len().min(LIMBS_PER_TREAD)];
            texels[FIRST_TREAD + index] =
                tread.ground.extend(tread.reach).extend(limbs.len() as f32);
            let first = FIRST_LIMB + index * LIMBS_PER_TREAD * 2;
            for (slot, limb) in limbs.iter().enumerate() {
                texels[first + slot * 2] = limb.from.extend(limb.girth);
                texels[first + slot * 2 + 1] = limb.to.extend(0.0);
            }
            let trail = FIRST_TRAIL + index * TRAIL_POINTS;
            for (slot, (at, laid)) in tread.trail.iter().take(TRAIL_POINTS).enumerate() {
                texels[trail + slot] = at.extend(*laid).extend(1.0);
            }
        }
        texels
    }
}

/// The weather's texels as the picture holds them.
pub(super) fn texel_bytes(texels: &[Vec4]) -> Vec<u8> {
    texels
        .iter()
        .flat_map(Vec4::to_array)
        .flat_map(f32::to_le_bytes)
        .collect()
}
