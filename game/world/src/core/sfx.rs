use crate::core::assets::AssetRef;
use crate::core::math::Rng;

pub use crate::data::sfx::Id as SfxId;

pub struct SfxDef {
    pub src: AssetRef,
    pub volume: SfxScalar,
    pub pitch: SfxScalar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxScalar {
    Fixed(f32),
    Random(f32, f32),
}

impl Default for SfxScalar {
    fn default() -> SfxScalar {
        SfxScalar::Fixed(1.0)
    }
}

impl SfxScalar {
    pub fn resolve(self, rng: &mut Rng) -> f32 {
        match self {
            SfxScalar::Fixed(value) => value,
            SfxScalar::Random(min, max) => min + rng.rand_float() * (max - min),
        }
    }
}
