use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::{assets, table};
use crate::systems::{actor, area, item};

#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct SfxId(pub String);

const FILE: &str = "sfx_table.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SfxDef {
    pub id: SfxId,
    pub src: String,
    #[serde(default)]
    pub volume: Varying,
    #[serde(default)]
    pub pitch: Varying,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
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

pub fn sfx_table() -> &'static [SfxDef] {
    static TABLE: OnceLock<Vec<SfxDef>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let defs: Vec<SfxDef> = table::load(FILE);
        table::unique_ids(defs.iter().map(|def| def.id.0.as_str()), FILE);

        for def in &defs {
            if !assets::exists(&def.src) {
                panic!("{FILE}: sfx '{}' src '{}' not found", def.id.0, def.src);
            }
            let (vmin, vmax) = def.volume.range();
            if !((0.0..=1.0).contains(&vmin) && (0.0..=1.0).contains(&vmax) && vmin <= vmax) {
                panic!(
                    "{FILE}: sfx '{}' volume must be within 0..=1 with min <= max",
                    def.id.0
                );
            }
            let (pmin, pmax) = def.pitch.range();
            if !(pmin > 0.0 && pmax > 0.0 && pmin <= pmax) {
                panic!(
                    "{FILE}: sfx '{}' pitch must be > 0 with min <= max",
                    def.id.0
                );
            }
        }

        for id in actor::models().iter().flat_map(|m| m.sfx_ids()) {
            if !defs.iter().any(|def| def.id == *id) {
                panic!(
                    "{FILE}: cue '{}' referenced by an actor model but not in sfx table",
                    id.0
                );
            }
        }

        for item in item::items() {
            for id in [&item.sfx.on_use, &item.sfx.drop].into_iter().flatten() {
                if !defs.iter().any(|def| def.id == *id) {
                    panic!(
                        "{FILE}: sfx '{}' referenced by item '{}' but not in sfx table",
                        id.0, item.id
                    );
                }
            }
        }

        for area in area::areas() {
            for id in area.tile_sfx.iter().flatten() {
                if !defs.iter().any(|def| def.id == *id) {
                    panic!(
                        "{FILE}: sfx '{}' on area '{}' tiles but not in sfx table",
                        id.0, area.name
                    );
                }
            }
        }

        defs
    })
}
