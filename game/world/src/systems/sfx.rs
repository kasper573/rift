use crate::core::assets;
use crate::data;

pub use crate::data::sfx::Id as SfxId;

pub struct SfxDef {
    pub src: &'static str,
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

/// Panics if any sound's source asset is missing or its ranges are invalid. Referenced ids are
/// compile-checked (every `SfxId` names a row in the table), so only asset/range validity remains.
pub fn validate() {
    for (id, def) in data::sfx::TABLE.iter() {
        assert!(
            assets::exists(def.src),
            "sfx {id:?} src '{}' not found",
            def.src
        );
        let (vmin, vmax) = def.volume.range();
        assert!(
            (0.0..=1.0).contains(&vmin) && (0.0..=1.0).contains(&vmax) && vmin <= vmax,
            "sfx {id:?} volume must be within 0..=1 with min <= max"
        );
        let (pmin, pmax) = def.pitch.range();
        assert!(
            pmin > 0.0 && pmax > 0.0 && pmin <= pmax,
            "sfx {id:?} pitch must be > 0 with min <= max"
        );
    }
}
