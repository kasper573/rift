use std::sync::OnceLock;

use crate::actors::SfxId;
use serde::Deserialize;

use crate::actors;
use crate::area;
use crate::assets;
use crate::items;
use crate::table;

const FILE: &str = "sfx_table.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SfxDef {
    pub id: SfxId,
    pub src: String,
    #[serde(default)]
    pub volume: SfxVolume,
    #[serde(default)]
    pub pitch: SfxPitch,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum SfxVolume {
    Fixed(f32),
    Random(f32, f32),
}

impl Default for SfxVolume {
    fn default() -> SfxVolume {
        SfxVolume::Fixed(1.0)
    }
}

impl SfxVolume {
    pub fn resolve(self, roll: f32) -> f32 {
        match self {
            SfxVolume::Fixed(volume) => volume,
            SfxVolume::Random(min, max) => min + roll.clamp(0.0, 1.0) * (max - min),
        }
    }

    fn valid(self) -> bool {
        let unit = |v: f32| (0.0..=1.0).contains(&v);
        match self {
            SfxVolume::Fixed(v) => unit(v),
            SfxVolume::Random(min, max) => unit(min) && unit(max) && min <= max,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum SfxPitch {
    Fixed(f32),
    Random(f32, f32),
}

impl Default for SfxPitch {
    fn default() -> SfxPitch {
        SfxPitch::Fixed(1.0)
    }
}

impl SfxPitch {
    pub fn resolve(self, roll: f32) -> f32 {
        match self {
            SfxPitch::Fixed(pitch) => pitch,
            SfxPitch::Random(min, max) => min + roll.clamp(0.0, 1.0) * (max - min),
        }
    }

    fn valid(self) -> bool {
        match self {
            SfxPitch::Fixed(p) => p > 0.0,
            SfxPitch::Random(min, max) => min > 0.0 && max > 0.0 && min <= max,
        }
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
            if !def.volume.valid() {
                panic!(
                    "{FILE}: sfx '{}' volume must be within 0..=1 with min <= max",
                    def.id.0
                );
            }
            if !def.pitch.valid() {
                panic!(
                    "{FILE}: sfx '{}' pitch must be > 0 with min <= max",
                    def.id.0
                );
            }
        }

        for id in actors::models().iter().flat_map(|m| m.sfx_ids()) {
            if !defs.iter().any(|def| def.id == *id) {
                panic!(
                    "{FILE}: cue '{}' referenced by an actor model but not in sfx table",
                    id.0
                );
            }
        }

        for item in items::items() {
            if let Some(id) = &item.sfx
                && !defs.iter().any(|def| def.id == *id)
            {
                panic!(
                    "{FILE}: sfx '{}' referenced by item '{}' but not in sfx table",
                    id.0, item.id
                );
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
