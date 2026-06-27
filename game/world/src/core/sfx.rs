use crate::core::assets::AssetRef;

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
    pub fn resolve(self, roll: f32) -> f32 {
        match self {
            SfxScalar::Fixed(value) => value,
            SfxScalar::Random(min, max) => min + roll.clamp(0.0, 1.0) * (max - min),
        }
    }

    pub fn range(self) -> (f32, f32) {
        (self.resolve(0.0), self.resolve(1.0))
    }
}
