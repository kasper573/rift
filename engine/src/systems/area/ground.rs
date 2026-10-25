use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::SfxId;
use std::collections::BTreeMap;

use crate::core::content::media::GltfInfos;
use crate::core::content::{
    ContentFile, ContentId, Loading, NamedContent, Nearby, RowCheck, serial,
};
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
/// it with, drawing a sheet over the painted cells if the cover has one, and the pieces it is
/// populated with, each a kind of thing scattered at its own ratio, size and colour, all of them
/// laid out from `seed`, so a designer can turn the seed until the scatter pleases.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cover {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("x-params-of" = "shader"))]
    pub params: BTreeMap<String, ParamFill>,
    #[serde(default, skip_serializing_if = "serial::is_zero_u64")]
    pub seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<CoverSheet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<CoverPiece>,
}

/// A sheet over the painted cells, lying on the ground (a sheen of gravel, moss) or flat at a
/// `level` the ground dips under (water, with the depth beneath each point told to the shader).
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverSheet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Meters>,
}

/// One kind of thing a cover is populated with: its `shape`, the `ratio` of the ground it covers
/// where the material is painted at full weight (1 covers the ground entirely with it, 0 none),
/// and the sizes and colours its copies are drawn from, each copy picking its own.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverPiece {
    pub shape: PieceShape,
    pub ratio: f32,
    #[serde(default = "SizeRange::one", skip_serializing_if = "SizeRange::is_one")]
    pub size: SizeRange,
    pub color: ColorRange,
}

/// What a piece is: a blade standing up from the ground, `width` across at its root and `height`
/// tall, in the piece's colour; or copies of a prop, tinted by the piece's colour.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PieceShape {
    Blade { width: Meters, height: Meters },
    Prop { prop: PropId },
}

/// The scales a piece's copies are drawn between.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeRange {
    pub min: f32,
    pub max: f32,
}

/// The colours a piece's copies are drawn between, each copy somewhere on the way from one to
/// the other.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorRange {
    pub from: Rgba,
    pub to: Rgba,
}

/// How thick a cover may grow: what a whole area at full weight draws in one pass.
pub const MAX_BLADES_PER_SQUARE_METER: f32 = 256.0;
pub const MAX_STREWN_PER_SQUARE_METER: f32 = 8.0;
pub const MAX_PIECES: usize = 8;
/// The least ground a copy is taken to cover, so a tiny prop at a full ratio stays countable.
const LEAST_BASE_AREA: f32 = 0.0025;

impl SizeRange {
    pub fn one() -> SizeRange {
        SizeRange { min: 1.0, max: 1.0 }
    }

    pub fn is_one(&self) -> bool {
        *self == SizeRange::one()
    }

    pub fn mean(&self) -> f32 {
        (self.min + self.max) / 2.0
    }
}

impl CoverPiece {
    /// How many copies a square metre holds at full weight: the ratio over the ground a copy of
    /// the mean size covers, `base_area` being what a copy of size 1 covers.
    pub fn per_square_meter(&self, base_area: f32) -> f32 {
        let mean = self.size.mean();
        self.ratio / (base_area.max(LEAST_BASE_AREA) * mean * mean)
    }
}

impl PieceShape {
    /// The ground a copy of size 1 covers: a blade its width squared, a prop the ground under its
    /// model's bounds.
    pub fn base_area(&self, prop_bounds: impl Fn(PropId) -> Option<(f32, f32)>) -> f32 {
        match self {
            PieceShape::Blade { width, .. } => width.0 * width.0,
            PieceShape::Prop { prop } => prop_bounds(*prop).map_or(1.0, |(x, z)| x * z),
        }
    }
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
            cover: None,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        if let Some(cover) = &self.cover {
            if let Some(level) = cover.sheet.and_then(|sheet| sheet.level) {
                cx.range(
                    "/cover/sheet/level",
                    level.0,
                    terrain::LOWEST.0,
                    terrain::HIGHEST.0,
                    "m",
                );
            }
            if cover.pieces.len() > MAX_PIECES {
                cx.error(
                    "/cover/pieces",
                    "too many pieces",
                    cover.pieces.len(),
                    format!("at most {MAX_PIECES}"),
                );
            }
            for (index, piece) in cover.pieces.iter().enumerate() {
                let at = format!("/cover/pieces/{index}");
                cx.range(
                    format!("{at}/ratio"),
                    piece.ratio,
                    0.0,
                    1.0,
                    "of the ground",
                );
                cx.range(
                    format!("{at}/size/min"),
                    piece.size.min,
                    0.05,
                    4.0,
                    "a scale",
                );
                cx.range(
                    format!("{at}/size/max"),
                    piece.size.max,
                    piece.size.min,
                    4.0,
                    "a scale",
                );
                if let PieceShape::Blade { width, height } = piece.shape {
                    cx.range(format!("{at}/shape/width"), width.0, 0.01, 0.5, "m");
                    cx.range(format!("{at}/shape/height"), height.0, 0.05, 1.5, "m");
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

/// Every ground's cover names a shader of the cover stage, fills only the parameters it declares,
/// each with a value of its kind and within its range, and grows no thicker than the engine
/// draws: at a full ratio a piece's copies per square metre, counted over the ground a copy
/// covers, stay within the budget of its shape.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let models = content.built::<GltfInfos>();
    let prop_bounds = |prop: PropId| {
        let bounds = models.0.get(&prop.get(&content).model.path())?.bounds;
        Some((bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z))
    };
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
        for (index, piece) in cover.pieces.iter().enumerate() {
            let density = piece.per_square_meter(piece.shape.base_area(prop_bounds));
            let (most, what) = match piece.shape {
                PieceShape::Blade { .. } => (MAX_BLADES_PER_SQUARE_METER, "blades"),
                PieceShape::Prop { .. } => (MAX_STREWN_PER_SQUARE_METER, "copies"),
            };
            if density > most {
                check.error(
                    format!("/cover/pieces/{index}/ratio"),
                    "too thick",
                    format!("{density:.0} {what} per square metre at full weight"),
                    format!("at most {most:.0}: a lower ratio or larger pieces"),
                );
            }
        }
        for (name, fill) in &cover.params {
            let Some(declared) = shader.params.iter().find(|param| &param.name == name) else {
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
