use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::SfxId;
use crate::core::content::{ContentFile, ContentId, NamedContent, Nearby, RowCheck};
use crate::core::math::Rgba;

pub type GroundId = ContentId<GroundDef>;
pub type SurfaceId = ContentId<SurfaceDef>;

/// A material the terrain is painted with: one flat colour, the surface it sounds like underfoot,
/// and whether anyone walks on it.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundDef {
    pub name: String,
    pub color: Rgba,
    pub surface: SurfaceId,
    pub walk: Walk,
}

impl NamedContent for GroundDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for GroundDef {
    const TABLE: &'static str = "ground";

    fn fresh(near: &Nearby) -> Result<GroundDef, String> {
        Ok(GroundDef {
            name: near.name(),
            color: Rgba::from([106, 143, 59, 255]),
            surface: near.file()?,
            walk: Walk::Walkable,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
    }
}

#[derive(
    JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Walk {
    Walkable,
    Blocks,
}

/// What ground sounds like underfoot.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceDef {
    pub name: String,
    pub footstep: SfxId,
}

impl NamedContent for SurfaceDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ContentFile for SurfaceDef {
    const TABLE: &'static str = "surface";

    fn fresh(near: &Nearby) -> Result<SurfaceDef, String> {
        Ok(SurfaceDef {
            name: near.name(),
            footstep: near.file()?,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
    }
}
