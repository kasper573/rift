use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::{FileSize, ModelRef};
use crate::core::content::gltf::GltfInfo;
use crate::core::content::{
    Bundle, ContentFile, ContentId, FileHashes, FileWriter, Loading, Nearby, PartSpec, Parts,
    RowCheck, Shape, serial,
};
use crate::core::geometry::Degrees;
use crate::core::math::{Offset, Pos, Rect, Rng};
use crate::core::units::Meters;
use crate::systems::area::prop_material::PropMaterialId;

pub type PropId = ContentId<PropDef>;

pub const MAX_PROP_BYTES: FileSize = FileSize::kib(4 * 1024);
pub const MAX_PROP_TRIANGLES: usize = 16_000;
pub const MAX_PROP_MATERIALS: usize = 2;
pub const MIN_POLYGON_POINTS: usize = 3;
pub const MAX_POLYGON_POINTS: usize = 64;
/// How many props a cell of an area stands at most, scattered ones included: a draw budget.
pub const MAX_PROPS: usize = 2000;
pub const MAX_SCATTERED: u32 = 500;
pub const MAX_DENSITY: f32 = 2.0;
pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 8.0;
/// How deep a prop may be sunk into the ground.
pub const MAX_SUNK: Meters = Meters(8.0);

pub const PROP_GLB: PartSpec = PartSpec::File {
    name: "model",
    exts: &["glb"],
};

/// The images the model draws with, each a PNG the model names by its path from the bundle.
pub const PROP_TEXTURES: PartSpec = PartSpec::Keyed {
    folder: "textures",
    exts: &["png"],
};

/// A thing standing in the world: a glTF model, the ground it takes up, how it is drawn when it
/// comes between the camera and a player, and the game's own materials drawing those of its
/// model's materials it names.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropDef {
    #[serde(skip)]
    pub model: ModelRef,
    pub footprint: Footprint,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub fade: bool,
    #[serde(default = "serial::yes", skip_serializing_if = "serial::is_true")]
    pub shadow: bool,
    /// Keyed by the name of a material of the model.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<String, PropMaterialId>,
}

/// The ground a prop takes up, in its own metres around its origin; nobody walks there.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Footprint {
    None,
    Circle {
        radius: Meters,
    },
    Box {
        width: Meters,
        depth: Meters,
    },
    Polygon {
        #[schemars(with = "Vec<serial::PosRow>")]
        #[serde(
            serialize_with = "serial::points",
            deserialize_with = "serial::points_from"
        )]
        points: Vec<Pos<Meters>>,
        /// The hash of the model the points were fitted to, when the editor fitted them: a model
        /// changed since is reported, so a footprint never silently stops matching its mesh.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fitted_to: Option<ModelHash>,
    },
}

/// A model file's hash as the loader hashes every file.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelHash(pub String);

impl Bundle for PropDef {
    const PARTS: &'static [PartSpec] = &[PROP_GLB, PROP_TEXTURES];

    fn read_parts(&mut self, parts: &mut Parts) {
        if let Some(glb) = parts.required(PROP_GLB) {
            self.model = ModelRef::new(parts.path(glb));
            if let Some(info) = parts.gltf_info(glb) {
                let mut cx = RowCheck::default();
                check_mesh(&info, parts.weight(glb), &mut cx);
                for name in self.materials.keys() {
                    if !info.materials.contains(name) {
                        cx.error(
                            format!("/materials/{name}"),
                            "no such material",
                            name,
                            format!("one of the model's: {}", info.materials.join(", ")),
                        );
                    }
                }
                parts.config_errors(cx);
            }
        }
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentFile for PropDef {
    const TABLE: &'static str = "prop";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(_: &Nearby) -> Result<PropDef, String> {
        Ok(PropDef {
            model: ModelRef::default(),
            footprint: Footprint::None,
            fade: false,
            shadow: true,
            materials: BTreeMap::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        match &self.footprint {
            Footprint::None => {}
            Footprint::Circle { radius } => {
                cx.range("/footprint/radius", radius.0, 0.05, 32.0, "m");
            }
            Footprint::Box { width, depth } => {
                cx.range("/footprint/width", width.0, 0.05, 64.0, "m");
                cx.range("/footprint/depth", depth.0, 0.05, 64.0, "m");
            }
            Footprint::Polygon { points, .. } => {
                cx.count(
                    "/footprint/points",
                    points.len(),
                    MIN_POLYGON_POINTS,
                    MAX_POLYGON_POINTS,
                    "points",
                );
                if points.len() >= MIN_POLYGON_POINTS && !is_simple(points) {
                    cx.error(
                        "/footprint/points",
                        "polygon crosses itself",
                        "edges that cross",
                        "a simple polygon, its edges meeting only at their ends",
                    );
                }
            }
        }
    }
}

impl Footprint {
    pub fn is_none(&self) -> bool {
        matches!(self, Footprint::None)
    }

    /// Whether `point` lies on the footprint of a prop standing at `at`, turned by `yaw` and
    /// `scale` times its size.
    pub fn covers(&self, at: Pos<Meters>, yaw: Degrees, scale: f32, point: Pos<Meters>) -> bool {
        let local = Degrees(-yaw.0).rotate(point - at) / scale.max(f32::EPSILON);
        match self {
            Footprint::None => false,
            Footprint::Circle { radius } => local.length() <= radius.0,
            Footprint::Box { width, depth } => {
                local.x.abs() <= width.0 / 2.0 && local.y.abs() <= depth.0 / 2.0
            }
            Footprint::Polygon { points, .. } => inside_polygon(points, Pos::origin() + local),
        }
    }

    /// The corners of the footprint as placed, going round it; none for a prop taking no ground.
    pub fn outline(&self, at: Pos<Meters>, yaw: Degrees, scale: f32) -> Vec<Pos<Meters>> {
        let place = |local: Pos<Meters>| at + yaw.rotate(local.to_vector() * scale);
        match self {
            Footprint::None => Vec::new(),
            Footprint::Circle { radius } => (0..24)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / 24.0;
                    place(Pos::new(angle.cos() * radius.0, angle.sin() * radius.0))
                })
                .collect(),
            Footprint::Box { width, depth } => {
                let (w, d) = (width.0 / 2.0, depth.0 / 2.0);
                [(-w, -d), (w, -d), (w, d), (-w, d)]
                    .into_iter()
                    .map(|(x, y)| place(Pos::new(x, y)))
                    .collect()
            }
            Footprint::Polygon { points, .. } => points.iter().map(|point| place(*point)).collect(),
        }
    }

    /// The box around the footprint as placed; none for a prop taking no ground.
    pub fn bounds(&self, at: Pos<Meters>, yaw: Degrees, scale: f32) -> Option<Rect<Meters>> {
        let outline = self.outline(at, yaw, scale);
        let first = *outline.first()?;
        Some(
            outline
                .iter()
                .fold(Rect::new(first, euclid::Size2D::zero()), |bounds, point| {
                    let min = bounds.min().min(*point);
                    let max = bounds.max().max(*point);
                    Rect::from_points([min, max])
                }),
        )
    }
}

/// A prop standing in an area, `sunk` that far into the ground under it, as a boulder bedded in
/// a hillside is.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropPlacement {
    pub prop: PropId,
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    #[serde(default, skip_serializing_if = "Degrees::is_zero")]
    pub yaw: Degrees,
    #[schemars(extend("default" = serial::one_f32()))]
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub scale: f32,
    #[serde(default, skip_serializing_if = "Meters::is_zero")]
    pub sunk: Meters,
}

impl PropPlacement {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/scale"),
            self.scale,
            MIN_SCALE,
            MAX_SCALE,
            "times the model's size,",
        );
        cx.range(
            format!("{pointer}/sunk"),
            self.sunk.0,
            0.0,
            MAX_SUNK.0,
            "m into the ground",
        );
    }
}

/// Props strewn over a region: `per_square_meter` of them, each one of `props`, turned any way and
/// sized within `scale`, laid out the same from `seed` on every machine.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scatter {
    pub props: Vec<PropId>,
    #[schemars(with = "serial::RectRow")]
    #[serde(
        serialize_with = "serial::rect",
        deserialize_with = "serial::rect_from"
    )]
    pub region: Rect<Meters>,
    pub per_square_meter: f32,
    pub seed: u64,
    #[serde(default, skip_serializing_if = "ScaleRange::is_default")]
    pub scale: ScaleRange,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleRange {
    pub min: f32,
    pub max: f32,
}

impl Default for ScaleRange {
    fn default() -> ScaleRange {
        ScaleRange { min: 1.0, max: 1.0 }
    }
}

impl ScaleRange {
    pub fn is_default(&self) -> bool {
        *self == ScaleRange::default()
    }
}

impl Scatter {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.count(format!("{pointer}/props"), self.props.len(), 1, 16, "props");
        cx.range(
            format!("{pointer}/per_square_meter"),
            self.per_square_meter,
            0.01,
            MAX_DENSITY,
            "props per square metre,",
        );
        cx.range(
            format!("{pointer}/scale/min"),
            self.scale.min,
            MIN_SCALE,
            self.scale.max.min(MAX_SCALE),
            "times the model's size,",
        );
        cx.range(
            format!("{pointer}/scale/max"),
            self.scale.max,
            self.scale.min.max(MIN_SCALE),
            MAX_SCALE,
            "times the model's size,",
        );
        if self.count() > MAX_SCATTERED {
            cx.error(
                format!("{pointer}/per_square_meter"),
                "too many props",
                format!("{} props over {} m²", self.count(), self.region.area()),
                format!("at most {MAX_SCATTERED} props from one scatter"),
            );
        }
    }

    pub fn count(&self) -> u32 {
        (self.region.area() * self.per_square_meter).ceil().max(0.0) as u32
    }

    /// The props the scatter strews, the same for the same seed on every machine.
    pub fn expand(&self) -> Vec<PropPlacement> {
        let mut rng = Rng::new(self.seed);
        (0..self.count())
            .filter_map(|_| {
                let prop = *rng.choose(&self.props)?;
                let at = self.region.origin
                    + Offset::new(
                        rng.rand_float() * self.region.size.width,
                        rng.rand_float() * self.region.size.height,
                    );
                let yaw = Degrees(rng.rand_float() * 360.0);
                let scale = self.scale.min + rng.rand_float() * (self.scale.max - self.scale.min);
                Some(PropPlacement {
                    prop,
                    at,
                    yaw,
                    scale,
                    sunk: Meters::default(),
                })
            })
            .collect()
    }
}

/// A polygon fitted to a model that has changed since is reported at the prop: its points no longer
/// follow the mesh.
pub async fn check_fitted(loading: &mut Loading) {
    let content = loading.content();
    let hashes = content.built::<FileHashes>();
    for (id, def) in loading.rows::<PropDef>(&content) {
        let Footprint::Polygon {
            fitted_to: Some(fitted),
            ..
        } = &def.footprint
        else {
            continue;
        };
        let current = hashes
            .0
            .get(def.model.as_str())
            .map(|hash| hash.hash.as_str());
        if current != Some(fitted.0.as_str()) {
            let mut check = RowCheck::default();
            check.error(
                "/footprint/fitted_to",
                "stale footprint",
                "a footprint fitted to an older model",
                "a footprint fitted to the model as it is now: refit it in the editor",
            );
            loading.row_errors(id, check);
        }
    }
}

fn check_mesh(info: &GltfInfo, weight: Option<FileSize>, cx: &mut RowCheck) {
    if let Some(weight) = weight
        && weight > MAX_PROP_BYTES
    {
        cx.error(
            "",
            "model too heavy",
            weight,
            format!("at most {MAX_PROP_BYTES}"),
        );
    }
    cx.range("", info.triangles, 1, MAX_PROP_TRIANGLES, "triangles");
    cx.range("", info.materials.len(), 0, MAX_PROP_MATERIALS, "materials");
}

fn inside_polygon(points: &[Pos<Meters>], point: Pos<Meters>) -> bool {
    let mut inside = false;
    let mut previous = match points.last() {
        Some(last) => *last,
        None => return false,
    };
    for &current in points {
        if (current.y > point.y) != (previous.y > point.y) {
            let cross = (previous.x - current.x) * (point.y - current.y) / (previous.y - current.y)
                + current.x;
            if point.x < cross {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Whether no two edges of the polygon cross, edges meeting only where they share an end.
fn is_simple(points: &[Pos<Meters>]) -> bool {
    let count = points.len();
    let edge = |index: usize| (points[index], points[(index + 1) % count]);
    for a in 0..count {
        for b in a + 1..count {
            let adjacent = b == a + 1 || (a == 0 && b == count - 1);
            if adjacent {
                continue;
            }
            let (p1, p2) = edge(a);
            let (q1, q2) = edge(b);
            if segments_cross(p1, p2, q1, q2) {
                return false;
            }
        }
    }
    true
}

fn segments_cross(p1: Pos<Meters>, p2: Pos<Meters>, q1: Pos<Meters>, q2: Pos<Meters>) -> bool {
    let side = |a: Pos<Meters>, b: Pos<Meters>, c: Pos<Meters>| {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    };
    let (d1, d2) = (side(q1, q2, p1), side(q1, q2, p2));
    let (d3, d4) = (side(p1, p2, q1), side(p1, p2, q2));
    (d1 * d2 < 0.0) && (d3 * d4 < 0.0)
}
