use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::SfxId;
use std::collections::BTreeMap;

use crate::core::content::{ContentFile, ContentId, Loading, NamedContent, Nearby, RowCheck};
use crate::core::math::Rgba;
use crate::core::terrain;
use crate::core::units::Meters;
use crate::core::vfx::shader::Stage;
use crate::core::vfx::{ParamFill, ParamValue, VfxShaderId};
use crate::systems::area::prop::PropId;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<Cover>,
}

/// What a ground grows or lies under where it is painted, from heavily to not at all as the
/// material blends into others: a shader of the cover stage, with the parameters the ground fills
/// it with, over the geometry the engine lays out from the paint.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cover {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("x-params-of" = "shader"))]
    pub params: BTreeMap<String, ParamFill>,
    pub geometry: CoverGeometry,
}

/// The geometry a cover is laid out as: a sheet over the painted cells, lying on the ground (a
/// sheen of gravel, moss) or flat at a level the ground dips under (water, with the depth beneath
/// each point told to the shader), blades standing up from the cells (grass), or copies of a prop
/// strewn over them (flowers, mushrooms, stones), as many per square metre as the material is
/// painted at full weight.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoverGeometry {
    Sheet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<Meters>,
    },
    Blades {
        per_square_meter: f32,
        height: Meters,
        width: Meters,
    },
    Strewn {
        prop: PropId,
        per_square_meter: f32,
        scale: f32,
    },
}

/// How thick a cover may grow: what a whole area at full weight draws in one pass.
pub const MAX_BLADES_PER_SQUARE_METER: f32 = 256.0;
pub const MAX_STREWN_PER_SQUARE_METER: f32 = 8.0;

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
            cover: None,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        if let Some(cover) = &self.cover {
            match &cover.geometry {
                CoverGeometry::Sheet { level } => {
                    if let Some(level) = level {
                        cx.range(
                            "/cover/geometry/level",
                            level.0,
                            terrain::LOWEST.0,
                            terrain::HIGHEST.0,
                            "m",
                        );
                    }
                }
                CoverGeometry::Blades {
                    per_square_meter,
                    height,
                    width,
                } => {
                    cx.range(
                        "/cover/geometry/per_square_meter",
                        *per_square_meter,
                        0.0,
                        MAX_BLADES_PER_SQUARE_METER,
                        "blades per square metre",
                    );
                    cx.range("/cover/geometry/height", height.0, 0.05, 1.5, "m");
                    cx.range("/cover/geometry/width", width.0, 0.01, 0.5, "m");
                }
                CoverGeometry::Strewn {
                    per_square_meter,
                    scale,
                    ..
                } => {
                    cx.range(
                        "/cover/geometry/per_square_meter",
                        *per_square_meter,
                        0.0,
                        MAX_STREWN_PER_SQUARE_METER,
                        "copies per square metre",
                    );
                    cx.range("/cover/geometry/scale", *scale, 0.05, 4.0, "a scale");
                }
            }
            for (name, fill) in &cover.params {
                if let ParamFill::Param { param } = fill {
                    cx.error(
                        format!("/cover/params/{name}"),
                        "a ground has no parameters to pass on",
                        param,
                        "a value",
                    );
                }
            }
        }
    }
}

/// Every ground's cover names a shader of the cover stage and fills only the parameters it
/// declares, each with a value of its kind and within its range.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, ground) in loading.rows::<GroundDef>(&content) {
        let Some(cover) = &ground.cover else {
            continue;
        };
        let mut check = RowCheck::default();
        let shader = cover.shader.get(&content);
        if shader.stage != Stage::Cover {
            check.error(
                "/cover/shader",
                "a shader of another stage",
                shader.stage.label(),
                "a shader of the cover stage",
            );
        }
        for (name, fill) in &cover.params {
            let Some(declared) = shader.param(name) else {
                check.error(
                    format!("/cover/params/{name}"),
                    "unknown shader parameter",
                    name,
                    format!(
                        "one of {}",
                        shader
                            .params
                            .iter()
                            .map(|param| param.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
                continue;
            };
            let ParamFill::Value(value) = fill else {
                continue;
            };
            if value.kind() != declared.kind {
                check.error(
                    format!("/cover/params/{name}"),
                    "wrong parameter kind",
                    value.kind().label(),
                    format!("a {}", declared.kind.label()),
                );
            }
            if let (ParamValue::Number(number), Some((min, max))) = (value, declared.range())
                && (*number < min || *number > max)
            {
                check.error(
                    format!("/cover/params/{name}"),
                    "parameter out of range",
                    number,
                    format!("{min}..{max}"),
                );
            }
        }
        loading.row_errors(id, check);
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
