use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{RowCheck, serial};
use crate::core::geometry::Degrees;
use crate::core::math::{Pos, Rgb};
use crate::core::units::Meters;

/// How many point lights an area holds at most: what a browser's forward renderer lights in one pass.
pub const MAX_LIGHTS: usize = 8;

/// The light an area stands in, fixed for the area: a sun, if it is outside, which also lights
/// the sky above it, the ambient light everywhere, and the grade its picture is tinted by. There
/// is no clock and no day-night cycle.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lighting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sun: Option<Sun>,
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

/// A tint laid over the whole picture, `strength` of the way toward `color`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    pub color: Rgb,
    pub strength: f32,
}

impl Lighting {
    /// A warm afternoon outside.
    pub fn day() -> Lighting {
        Lighting {
            sun: Some(Sun {
                elevation: Degrees(52.0),
                azimuth: Degrees(215.0),
                color: Rgb([255, 236, 190]),
            }),
            ambient: Rgb([98, 122, 160]),
            grade: Grade {
                color: Rgb::WHITE,
                strength: 0.0,
            },
        }
    }

    /// Dusk outside: the sun low and the picture graded toward night.
    pub fn night() -> Lighting {
        Lighting {
            sun: Some(Sun {
                elevation: Degrees(18.0),
                azimuth: Degrees(250.0),
                color: Rgb([210, 190, 200]),
            }),
            ambient: Rgb([60, 70, 110]),
            grade: Grade::NIGHT,
        }
    }

    /// Indoors: no sun, a dim cool ambient and the night grade; what light there is comes from the
    /// area's lights.
    pub fn interior() -> Lighting {
        Lighting {
            sun: None,
            ambient: Rgb([70, 80, 105]),
            grade: Grade::NIGHT,
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
    }
}

impl Grade {
    pub const NIGHT: Grade = Grade {
        color: Rgb([86, 128, 168]),
        strength: 0.55,
    };
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
