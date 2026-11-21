//! The stage a water's shader runs in: the sheet an area lays over its ground wherever one of
//! its waters lies, drawn after everything solid, over the picture of what lies behind it.

use crate::core::render::world::water::SHORE_REACH;
use crate::core::vfx::shader::CLOCK_CHECK;
use crate::core::vfx::stage::{BLANK_VFX, main_defs, with_defs};
use crate::core::vfx::{ShaderPass, Stage, StageKind, StageSpec, Stages};
use crate::systems::area::stages::{treading, weather_helpers};

/// A shader painting the surface of a body of water.
pub const WATER: Stage = Stage::registered(&StageSpec {
    name: "water",
    kind: StageKind::Pass,
    header,
    check_header,
    required: &["vfx"],
    blank: BLANK_VFX,
    wesl: true,
    names: &[
        "weather",
        "wind_gust",
        "bed",
        "column",
        "water",
        "silt",
        "seen_through",
        "mirror",
        "mirrors",
        "bends",
        "depth",
        "shore",
        "fall",
        "position",
        "eye",
        "light",
        "waterline",
        "wakes",
        "wake",
        "rings",
        "rest_reach",
    ],
    passes,
});

pub fn register(stages: &mut Stages) {
    stages.add(WATER);
}

const HEADER: &str = include_str!("shaders/water.wesl");
const CHECK_HEADER: &str = include_str!("shaders/water_check.wgsl");
const WADING: &str = include_str!("shaders/wading.wgsl");

fn header() -> String {
    format!("{HEADER}\n{}", helpers())
}

fn check_header() -> String {
    format!("{CHECK_HEADER}\n{CLOCK_CHECK}\n{}", helpers())
}

fn helpers() -> String {
    format!(
        "{}\n{}\n// How far from the shore `shore` tells a point lies at most, in metres.\nconst SHORE_REACH: f32 = {:?};",
        weather_helpers(),
        treading(WADING),
        SHORE_REACH.0
    )
}

/// A water draws as the sheet over the ground's patches alone.
fn passes() -> Vec<ShaderPass> {
    vec![ShaderPass {
        name: "sheet",
        defs: with_defs(main_defs(), &["SHEET_PATCH"]),
    }]
}
