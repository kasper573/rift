use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Loading, RowCheck, serial};
use crate::core::geometry::Degrees;
use crate::core::math::{Pos, Rgb};
use crate::core::render::camera::Tone;
use crate::core::render::world::sky::Sky;
use crate::core::units::{Meters, Stops};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::{ParamValue, VfxShaderId};
use crate::systems::area::AreaDef;

/// How many point lights an area holds at most: what a browser's forward renderer lights in one pass.
pub const MAX_LIGHTS: usize = 8;

/// The light an area stands in: a sun, if it is outside, standing where it stands at noon, which
/// also lights the sky above it unless the sky is dark, the full moon opposite it lighting the
/// world by night; how the dome of that sky is painted, the ambient light everywhere, and the
/// grade its picture is tinted by.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lighting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sun: Option<Sun>,
    #[serde(default, skip_serializing_if = "Sky::is_air")]
    pub sky: Sky,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dome: Option<DomeLook>,
    pub ambient: Rgb,
    pub grade: Grade,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sun {
    pub elevation: Degrees,
    pub azimuth: Degrees,
    pub color: Rgb,
}

/// How the dome of an open sky is painted, with the weather's clouds, the moon and the stars: by
/// a shader of the sky stage, filled with `params`.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomeLook {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("x-params-of" = "shader"))]
    pub params: BTreeMap<String, ParamValue>,
}

/// How the picture is finished: a tint laid over the whole of it, `strength` of the way toward
/// `color`, opened `exposure` brighter than the engine opens every picture, its colours
/// `saturation` times as saturated, its midtones `contrast` times as contrasted and its light
/// warmed toward amber by `warmth`, or cooled toward blue below 0.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    pub color: Rgb,
    pub strength: f32,
    #[serde(default, skip_serializing_if = "Stops::is_zero")]
    pub exposure: Stops,
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub saturation: f32,
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub contrast: f32,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub warmth: f32,
}

impl Grade {
    /// No tint and the engine's own finish.
    pub const PLAIN: Grade = Grade {
        color: Rgb::WHITE,
        strength: 0.0,
        exposure: Stops(0.0),
        saturation: 1.0,
        contrast: 1.0,
        warmth: 0.0,
    };

    /// The finish the camera gives the picture.
    pub fn tone(&self) -> Tone {
        Tone {
            exposure: self.exposure,
            saturation: self.saturation,
            contrast: self.contrast,
            warmth: self.warmth,
        }
    }
}

impl Lighting {
    /// Broad daylight outside.
    pub fn day() -> Lighting {
        Lighting {
            sun: Some(Sun {
                elevation: Degrees(52.0),
                azimuth: Degrees(215.0),
                color: Rgb([255, 248, 236]),
            }),
            sky: Sky::Air,
            dome: None,
            ambient: Rgb([80, 95, 125]),
            grade: Grade::PLAIN,
        }
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        if let Some(sun) = &self.sun {
            cx.range(
                format!("{pointer}/sun/elevation"),
                sun.elevation.0,
                0.0,
                90.0,
                "degrees above the horizon",
            );
            cx.range(
                format!("{pointer}/sun/azimuth"),
                sun.azimuth.0,
                0.0,
                360.0,
                "degrees from north",
            );
        }
        cx.range(
            format!("{pointer}/grade/strength"),
            self.grade.strength,
            0.0,
            1.0,
            "of the way toward the grade's colour,",
        );
        cx.range(
            format!("{pointer}/grade/exposure"),
            self.grade.exposure.0,
            -3.0,
            3.0,
            "stops",
        );
        cx.range(
            format!("{pointer}/grade/saturation"),
            self.grade.saturation,
            0.0,
            2.0,
            "times the saturation",
        );
        cx.range(
            format!("{pointer}/grade/contrast"),
            self.grade.contrast,
            0.5,
            2.0,
            "times the contrast",
        );
        cx.range(
            format!("{pointer}/grade/warmth"),
            self.grade.warmth,
            -1.0,
            1.0,
            "toward amber",
        );
    }
}

/// Every area's sky dome is painted by a shader of the sky stage, filled only with the parameters
/// it declares, each with a value of its kind and within its range.
pub fn check_dome(loading: &mut Loading) {
    let content = loading.content();
    for (id, area) in loading.rows::<AreaDef>(&content) {
        let Some(dome) = &area.lighting.dome else {
            continue;
        };
        let mut check = RowCheck::default();
        let shader = dome.shader.get(&content);
        if shader.stage != Stage::Sky {
            check.error(
                "/lighting/dome/shader",
                "a shader of another stage",
                shader.stage.label(),
                "a shader of the sky stage",
            );
        }
        for (name, value) in &dome.params {
            shader.check_fill(
                &mut check,
                &format!("/lighting/dome/params/{name}"),
                name,
                Some(value),
            );
        }
        loading.row_errors(id, check);
    }
}

/// A light standing in an area: a point `height` above the ground at `at`, reaching `radius`
/// around, flickering `flicker` of the way between dark and full.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointLightDef {
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    pub color: Rgb,
    pub intensity: f32,
    pub radius: Meters,
    pub height: Meters,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub flicker: f32,
}

impl PointLightDef {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/intensity"),
            self.intensity,
            0.0,
            100.0,
            "brightness",
        );
        cx.range(format!("{pointer}/radius"), self.radius.0, 0.5, 32.0, "m");
        cx.range(format!("{pointer}/height"), self.height.0, 0.0, 16.0, "m");
        cx.range(
            format!("{pointer}/flicker"),
            self.flicker,
            0.0,
            1.0,
            "flicker",
        );
    }
}
