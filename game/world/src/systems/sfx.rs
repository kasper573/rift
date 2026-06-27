use crate::core::assets::AssetRef;

pub use crate::data::sfx::Id as SfxId;

pub struct SfxDef {
    pub src: AssetRef,
    pub volume: Varying,
    pub pitch: Varying,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Varying {
    Fixed(f32),
    Random(f32, f32),
}

impl Default for Varying {
    fn default() -> Varying {
        Varying::Fixed(1.0)
    }
}

impl Varying {
    pub fn resolve(self, roll: f32) -> f32 {
        match self {
            Varying::Fixed(value) => value,
            Varying::Random(min, max) => min + roll.clamp(0.0, 1.0) * (max - min),
        }
    }

    pub fn range(self) -> (f32, f32) {
        (self.resolve(0.0), self.resolve(1.0))
    }
}
