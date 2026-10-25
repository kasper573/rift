use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ground::GroundId;
use crate::core::content::{PartSpec, RowCheck};
use crate::core::geometry::Degrees;
use crate::core::terrain::{MATERIALS, MIN_SIDE, TerrainSize};

pub const MIN_SLOPE: Degrees = Degrees(10.0);
pub const MAX_SLOPE: Degrees = Degrees(60.0);

pub const HEIGHT: PartSpec = PartSpec::File {
    name: "terrain/height",
    exts: &["png"],
};
pub const PAINT: PartSpec = PartSpec::File {
    name: "terrain/paint",
    exts: &["png"],
};

/// The ground of an area: how far it reaches, the four materials its paint blends, and how steep a
/// slope still walks. Its heights and paint are parts of the area's bundle.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    pub size: TerrainSize,
    pub materials: [GroundId; MATERIALS],
    pub max_slope: Degrees,
}

impl Terrain {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        self.size.check(&format!("{pointer}/size"), MIN_SIDE, cx);
        cx.range(
            format!("{pointer}/max_slope"),
            self.max_slope.0,
            MIN_SLOPE.0,
            MAX_SLOPE.0,
            "degrees of slope",
        );
    }
}
