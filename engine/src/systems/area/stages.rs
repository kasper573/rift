//! The stages of the scenery's own shaders, which the area system draws in materials of its own:
//! the cover a terrain grows or lies under, a prop's material, an area's whole sky and the rain
//! falling round the eye. Every one reads the weather the scenery publishes.

use super::heavens::SKY_PICTURE;
use super::rain::{LEANING, OPEN_TOP, SHELTER_DEPTH, SHELTER_STEP, SHELTER_STEPS};
use super::shown_weather::{LIMBS_PER_TREAD, RIPPLE_WINDOW, TREADS};
use crate::core::render::world::sky::AIR_WAYS;
use crate::core::vfx::shader::{CLOCK_CHECK, site_prelude};
use crate::core::vfx::stage::{BLANK_VFX, main_defs, main_pass, prepass_defs, with_defs};
use crate::core::vfx::{ShaderPass, Stage, StageKind, StageSpec, Stages};

/// A shader the cover a terrain grows or lies under runs in.
pub const COVER: Stage = Stage::registered(&StageSpec {
    name: "cover",
    kind: StageKind::Pass,
    header: cover_header,
    check_header: cover_check_header,
    prelude: site_prelude,
    required: &["vfx", "sway"],
    blank: concat!(
        "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n",
        "fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {\n    return world;\n}\n"
    ),
    wesl: true,
    names: &[
        "weather",
        "wind_gust",
        "wind_lean",
        "tread_push",
        "lean",
        "sway",
        "weight",
        "tip",
        "seed",
        "piece",
        "mark",
        "root",
        "position",
        "light",
        "albedo",
    ],
    passes: cover_passes,
});

/// A shader run in a material of a prop.
pub const PROP: Stage = Stage::registered(&StageSpec {
    name: "prop",
    kind: StageKind::Pass,
    header: prop_header,
    check_header: prop_check_header,
    prelude: site_prelude,
    required: &["vfx", "sway"],
    blank: concat!(
        "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n",
        "fn sway(world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {\n    return world;\n}\n"
    ),
    wesl: true,
    names: &[
        "local",
        "origin",
        "size",
        "in_world",
        "tint",
        "screen_right",
        "screen_up",
        "normal",
        "toward_eye",
    ],
    passes: prop_passes,
});

/// A shader painting an area's whole sky.
pub const SKY: Stage = Stage::registered(&StageSpec {
    name: "sky",
    kind: StageKind::Pass,
    header: sky_header,
    check_header: sky_check_header,
    prelude: site_prelude,
    required: &["vfx"],
    blank: BLANK_VFX,
    wesl: true,
    names: &[
        "look",
        "light_way",
        "light_color",
        "sun_way",
        "sun_color",
        "moon_way",
        "moon_size",
        "night",
        "through_air",
        "sky_turned",
        "pixel_angle",
        "twinkles",
        "cloud_base",
        "cloud_drift",
        "cloud_at",
        "sky_light",
    ],
    passes: sky_passes,
});

/// A shader painting the rain falling round the eye.
pub const RAIN: Stage = Stage::registered(&StageSpec {
    name: "rain",
    kind: StageKind::Pass,
    header: rain_header,
    check_header: rain_check_header,
    prelude: site_prelude,
    required: &["vfx"],
    blank: BLANK_VFX,
    wesl: true,
    names: &[],
    passes: main_pass,
});

pub fn register(stages: &mut Stages) {
    stages.add(COVER).add(PROP).add(SKY).add(RAIN);
}

/// The weather's helpers, as every shader that reads the weather is composed with them, and the
/// ground's lit shader with them.
pub fn weather_helpers() -> String {
    format!(
        "{WEATHER_HELPERS}\n// How far under a roof's or a canopy's top a point lies wholly sheltered, in metres.\nconst SHELTER_DEPTH: f32 = {SHELTER_DEPTH:?};\n// The top of a cell of the shelter where nothing stands over the ground.\nconst OPEN_TOP: f32 = {OPEN_TOP:?};\n// How the rain's way down to a point is followed for what shelters it.\nconst SHELTER_STEPS: i32 = {SHELTER_STEPS};\nconst SHELTER_STEP: f32 = {SHELTER_STEP:?};\nconst LEANING: f32 = {LEANING:?};",
    )
}

/// `helpers`, which read who treads the ground from the weather, told how the weather lays
/// them out.
pub fn treading(helpers: &str) -> String {
    helpers
        .replace("LIMBS_PER_TREAD", &format!("{LIMBS_PER_TREAD}u"))
        .replace("RIPPLE_WINDOW", &format!("{RIPPLE_WINDOW}"))
        .replace("TREADS", &format!("{TREADS}u"))
}

const COVER_HEADER: &str = include_str!("shaders/cover.wesl");
const COVER_CHECK_HEADER: &str = include_str!("shaders/cover_check.wgsl");
const PROP_HEADER: &str = include_str!("shaders/prop.wesl");
const PROP_CHECK_HEADER: &str = include_str!("shaders/prop_check.wgsl");
const SKY_HEADER: &str = include_str!("shaders/sky.wesl");
const SKY_CHECK_HEADER: &str = include_str!("shaders/sky_check.wgsl");
const RAIN_HEADER: &str = include_str!("shaders/rain.wesl");
const RAIN_CHECK_HEADER: &str = include_str!("shaders/rain_check.wgsl");
/// What a shader under the open sky reads of it, the sky's and the rain's alike: the light, the
/// moon and the clouds; and the light the sky sheds, at run time.
const HEAVENS: &str = include_str!("shaders/heavens.wgsl");
const SKY_LIGHT: &str = include_str!("shaders/sky_light.wesl");
const WEATHER_HELPERS: &str = include_str!("shaders/weather.wgsl");
/// What a cover's blades bend by, on top of the weather.
const TREAD_HELPERS: &str = include_str!("shaders/tread.wgsl");

fn cover_header() -> String {
    format!("{COVER_HEADER}\n{}", cover_helpers())
}

fn cover_check_header() -> String {
    format!("{COVER_CHECK_HEADER}\n{CLOCK_CHECK}\n{}", cover_helpers())
}

fn prop_header() -> String {
    format!("{PROP_HEADER}\n{}", weather_helpers())
}

fn prop_check_header() -> String {
    format!("{PROP_CHECK_HEADER}\n{CLOCK_CHECK}\n{}", weather_helpers())
}

fn sky_header() -> String {
    format!(
        "{SKY_HEADER}\n{SKY_LIGHT}\n{}\n{}",
        heavens(),
        weather_helpers()
    )
}

fn sky_check_header() -> String {
    format!(
        "{SKY_CHECK_HEADER}\n{}\n{CLOCK_CHECK}\n{}",
        heavens(),
        weather_helpers()
    )
}

fn rain_header() -> String {
    format!(
        "{RAIN_HEADER}\n{SKY_LIGHT}\n{}\n{}",
        heavens(),
        weather_helpers()
    )
}

fn rain_check_header() -> String {
    format!(
        "{RAIN_CHECK_HEADER}\n{}\n{CLOCK_CHECK}\n{}",
        heavens(),
        weather_helpers()
    )
}

fn heavens() -> String {
    format!("const AIR_WAYS: u32 = {AIR_WAYS}u;\n{HEAVENS}")
}

fn cover_helpers() -> String {
    format!("{}\n{}", weather_helpers(), treading(TREAD_HELPERS))
}

/// A cover draws as a mesh a copy of a prop at a time, and as the blades of its grass and the sheet
/// under them, which the weather blends over the ground, so only the copies draw in the prepass.
fn cover_passes() -> Vec<ShaderPass> {
    let uvs_b = |base| with_defs(base, &["VERTEX_UVS_B"]);
    vec![
        ShaderPass {
            name: "copies",
            defs: uvs_b(main_defs()),
        },
        ShaderPass {
            name: "copies prepass",
            defs: uvs_b(prepass_defs()),
        },
        ShaderPass {
            name: "blades",
            defs: uvs_b(with_defs(main_defs(), &["GRASS_BLADE"])),
        },
        ShaderPass {
            name: "sheet",
            defs: uvs_b(with_defs(main_defs(), &["SHEET_PATCH"])),
        },
    ]
}

/// A sky's shader draws the sky, and the picture of it the water mirrors.
fn sky_passes() -> Vec<ShaderPass> {
    vec![
        ShaderPass {
            name: "main",
            defs: main_defs(),
        },
        ShaderPass {
            name: "picture",
            defs: with_defs(main_defs(), &[SKY_PICTURE]),
        },
    ]
}

/// A prop's shader bakes its model's impostor as well as drawing it, and writes depth and normals
/// ahead of the main pass.
fn prop_passes() -> Vec<ShaderPass> {
    vec![
        ShaderPass {
            name: "main",
            defs: main_defs(),
        },
        ShaderPass {
            name: "prepass",
            defs: prepass_defs(),
        },
        ShaderPass {
            name: "bake",
            defs: with_defs(main_defs(), &["IMPOSTOR_BAKE"]),
        },
    ]
}
