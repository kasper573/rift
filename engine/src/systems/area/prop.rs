use std::collections::{BTreeMap, HashMap};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::gltf::{self, GltfInfo, Triangle};
use crate::core::content::media::GltfInfos;
use crate::core::content::{AssetKind, FileSize, ModelRef};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, FileHashes, FileShape, FileWriter, Loading, Nearby,
    PartSpec, Parts, RowCheck, serial,
};
use crate::core::ground::TILE_SIDE;
use crate::core::math::{Offset, Pos, Rect, Rng};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{Degrees, Meters};
use crate::core::vfx::VfxId;
use crate::systems::area::prop_material::PropMaterialId;
use crate::systems::water::ripples::RippleSource;

pub type PropId = ContentId<PropDef>;

pub const MAX_PROP_BYTES: FileSize = FileSize::kib(4 * 1024);
pub const MAX_PROP_TRIANGLES: usize = 96_000;
pub const MAX_PROP_MATERIALS: usize = 8;
/// How many vfx a prop or a body plays of its own accord at most.
pub const MAX_EFFECTS: usize = 4;
pub const MIN_POLYGON_POINTS: usize = 3;
pub const MAX_POLYGON_POINTS: usize = 64;
pub const MAX_POLYGON_PARTS: usize = 32;
/// How many props an area stands at most on a square of its ground, scattered ones included: a
/// draw budget.
pub const MAX_PROPS: usize = 2000;
pub const MAX_SCATTERED: u32 = 500;
pub const MAX_DENSITY: f32 = 2.0;
pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 8.0;
/// How deep a prop may be sunk into the ground.
pub const MAX_SUNK: Meters = Meters(8.0);

pub const PROP_GLB: PartSpec = PartSpec::File {
    name: "model",
    exts: AssetKind::Model.extensions(),
};

/// The images the model draws with, each a PNG the model names by its path from the bundle.
pub const PROP_TEXTURES: PartSpec = PartSpec::Keyed {
    folder: "textures",
    exts: AssetKind::Image.extensions(),
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
    /// Whether it keeps the rain off the ground under it, as a roof or a dense canopy does.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub shelter: bool,
    /// Whether it rests on what lies under it, as a rug, a doormat or the shine marking a warp
    /// does: lifted out of a floor placed where it stands onto the floor's top, let down onto the
    /// floor or ground under it, and tilted along a slope, its shape as it is. Without it, its foot
    /// stands at the height of the ground under its middle.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub snap_to_ground: bool,
    /// Keyed by the name of a material of the model.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<String, PropMaterialId>,
    /// The vfx it plays over and over wherever it is seen, standing in an area, lying on the
    /// ground or held in a hand: the flame of a lantern, the glow of a crystal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<VfxId>,
    /// How it breaks the water it stands in, as a shoal of fish or a spring does. Without it,
    /// it leaves the water as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripples: Option<RippleSource>,
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
        /// The simple polygons that together are the footprint, each going round its corners:
        /// an arch takes the ground under each leg and not between them.
        #[schemars(with = "Vec<Vec<serial::PosRow>>")]
        #[serde(
            serialize_with = "serial::parts",
            deserialize_with = "serial::parts_from"
        )]
        parts: Vec<Vec<Pos<Meters>>>,
        /// The hash of the model the parts were fitted to, when the editor fitted them: a model
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
    const SHAPE: FileShape = FileShape::Bundle;

    fn fresh(_: &Nearby) -> Result<PropDef, String> {
        Ok(PropDef {
            model: ModelRef::default(),
            footprint: Footprint::None,
            fade: false,
            shadow: true,
            shelter: false,
            snap_to_ground: false,
            materials: BTreeMap::new(),
            effects: Vec::new(),
            ripples: None,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.count("/effects", self.effects.len(), 0, MAX_EFFECTS, "vfx");
        if let Some(ripples) = &self.ripples {
            ripples.check("/ripples", cx);
        }
        match &self.footprint {
            Footprint::None => {}
            Footprint::Circle { radius } => {
                cx.range("/footprint/radius", radius.0, 0.05, 32.0, "m");
            }
            Footprint::Box { width, depth } => {
                cx.range("/footprint/width", width.0, 0.05, 64.0, "m");
                cx.range("/footprint/depth", depth.0, 0.05, 64.0, "m");
            }
            Footprint::Polygon { parts, .. } => {
                cx.count(
                    "/footprint/parts",
                    parts.len(),
                    1,
                    MAX_POLYGON_PARTS,
                    "parts",
                );
                for (index, points) in parts.iter().enumerate() {
                    let pointer = format!("/footprint/parts/{index}");
                    cx.count(
                        &pointer,
                        points.len(),
                        MIN_POLYGON_POINTS,
                        MAX_POLYGON_POINTS,
                        "points",
                    );
                    if points.len() >= MIN_POLYGON_POINTS && !is_simple(points) {
                        cx.error(
                            pointer,
                            "polygon crosses itself",
                            "edges that cross",
                            "a simple polygon, its edges meeting only at their ends",
                        );
                    }
                }
            }
        }
    }
}

impl Footprint {
    pub fn is_none(&self) -> bool {
        matches!(self, Footprint::None)
    }

    /// The ground a model of `triangles` takes from a walker who stands `tall`: a polygon round
    /// every connected piece of it the walker would meet, however little of it touches the ground,
    /// that remembers the model it was `fitted_to`. A model of more pieces than a footprint holds
    /// keeps the widest. None for a model with no ground to stand on.
    pub fn fitted(triangles: &[Triangle], tall: Meters, fitted_to: ModelHash) -> Option<Footprint> {
        let mut parts: Vec<Vec<Pos<Meters>>> = gltf::footing(triangles, tall)
            .into_iter()
            .map(|points| {
                let step = points.len().div_ceil(MAX_POLYGON_POINTS);
                points.into_iter().step_by(step).collect()
            })
            .collect();
        parts.sort_by(|a, b| area_of(b).total_cmp(&area_of(a)));
        parts.truncate(MAX_POLYGON_PARTS);
        (!parts.is_empty()).then_some(Footprint::Polygon {
            parts,
            fitted_to: Some(fitted_to),
        })
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
            Footprint::Polygon { parts, .. } => parts
                .iter()
                .any(|points| inside_polygon(points, Pos::origin() + local)),
        }
    }

    /// The corners of each part of the footprint as placed, going round it; none for a prop taking
    /// no ground.
    pub fn outlines(&self, at: Pos<Meters>, yaw: Degrees, scale: f32) -> Vec<Vec<Pos<Meters>>> {
        let place = |local: Pos<Meters>| at + yaw.rotate(local.to_vector() * scale);
        match self {
            Footprint::None => Vec::new(),
            Footprint::Circle { radius } => vec![
                (0..24)
                    .map(|step| {
                        let angle = std::f32::consts::TAU * step as f32 / 24.0;
                        place(Pos::new(angle.cos() * radius.0, angle.sin() * radius.0))
                    })
                    .collect(),
            ],
            Footprint::Box { width, depth } => {
                let (w, d) = (width.0 / 2.0, depth.0 / 2.0);
                vec![
                    [(-w, -d), (w, -d), (w, d), (-w, d)]
                        .into_iter()
                        .map(|(x, y)| place(Pos::new(x, y)))
                        .collect(),
                ]
            }
            Footprint::Polygon { parts, .. } => parts
                .iter()
                .map(|points| points.iter().map(|point| place(*point)).collect())
                .collect(),
        }
    }

    /// How far the footprint reaches from the prop's origin; none for a prop taking no ground.
    pub fn reach(&self) -> Option<Meters> {
        self.outlines(Pos::origin(), Degrees(0.0), 1.0)
            .into_iter()
            .flatten()
            .map(|corner| corner.to_vector().length())
            .reduce(f32::max)
            .map(Meters)
    }

    /// The box around the footprint as placed; none for a prop taking no ground.
    pub fn bounds(&self, at: Pos<Meters>, yaw: Degrees, scale: f32) -> Option<Rect<Meters>> {
        let outline: Vec<Pos<Meters>> = self
            .outlines(at, yaw, scale)
            .into_iter()
            .flatten()
            .collect();
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
        cx.above_up_to(
            format!("{pointer}/per_square_meter"),
            self.per_square_meter,
            0.0,
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

    /// The props the scatter strews, the same for the same seed on every machine, but for those
    /// `clearing` leaves out: each is drawn all the same, so leaving one out moves no other.
    pub fn expand<'a>(
        &'a self,
        clearing: &'a ScatterClearing,
    ) -> impl Iterator<Item = PropPlacement> + 'a {
        let mut rng = Rng::new(self.seed);
        let drawn = (0..self.count()).filter_map(move |_| {
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
        });
        drawn.filter(|placed| clearing.keeps_clear(placed))
    }
}

/// The ground an area's placed props and its objects stand on, which its scatters strew nothing
/// on: a scattered prop is left out where it would stand, or its own footprint reach, within
/// theirs.
pub struct ScatterClearing {
    taken: Vec<Vec<Pos<Meters>>>,
    index: SpatialIndex,
    strewn: HashMap<PropId, Footprint>,
}

impl Default for ScatterClearing {
    fn default() -> ScatterClearing {
        ScatterClearing {
            taken: Vec::new(),
            index: SpatialIndex::new(TILE_SIDE),
            strewn: HashMap::new(),
        }
    }
}

impl ScatterClearing {
    /// The ground `placed` takes, clear of what `scatters` strew.
    pub fn of<'a>(
        content: &Content,
        placed: impl IntoIterator<Item = &'a PropPlacement>,
        scatters: impl IntoIterator<Item = &'a Scatter>,
    ) -> ScatterClearing {
        let taken: Vec<Vec<Pos<Meters>>> = placed
            .into_iter()
            .flat_map(|placed| {
                let def = placed.prop.get(content);
                def.footprint.outlines(placed.at, placed.yaw, placed.scale)
            })
            .collect();
        let mut index = SpatialIndex::new(TILE_SIDE);
        index.refile(
            taken
                .iter()
                .map(|outline| Rect::from_points(outline.iter().copied())),
        );
        let strewn = scatters
            .into_iter()
            .flat_map(|scatter| &scatter.props)
            .map(|&prop| (prop, prop.get(content).footprint.clone()))
            .collect();
        ScatterClearing {
            taken,
            index,
            strewn,
        }
    }

    /// Whether a prop a scatter strews at `placed` keeps clear of the ground taken: neither where
    /// it stands nor any of its own footprint lies within it.
    pub fn keeps_clear(&self, placed: &PropPlacement) -> bool {
        let own = self
            .strewn
            .get(&placed.prop)
            .map_or_else(Vec::new, |footprint| {
                footprint.outlines(placed.at, placed.yaw, placed.scale)
            });
        let reach = Rect::from_points(own.iter().flatten().copied().chain([placed.at]));
        self.index.within(reach).all(|taken| {
            let taken = &self.taken[taken];
            !inside_polygon(taken, placed.at) && own.iter().all(|part| !outlines_meet(part, taken))
        })
    }

    /// Where the ground taken differs from `before`'s: each outline one holds and the other
    /// doesn't, by the box around it.
    pub fn changes_from<'a>(
        &'a self,
        before: &'a ScatterClearing,
    ) -> impl Iterator<Item = Rect<Meters>> + 'a {
        let gone = before
            .taken
            .iter()
            .filter(|outline| !self.taken.contains(outline));
        let come = self
            .taken
            .iter()
            .filter(|outline| !before.taken.contains(outline));
        gone.chain(come)
            .map(|outline| Rect::from_points(outline.iter().copied()))
    }
}

/// A polygon fitted to a model that has changed since is reported at the prop: its parts no longer
/// follow the mesh.
pub fn check_fitted(loading: &mut Loading) {
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

/// A material drawn in pictures of the game's lays them by its model's texture coordinates, and
/// turns a relief by its tangents: a model that draws the material without them is reported at
/// the prop.
pub fn check_pictured(loading: &mut Loading) {
    let content = loading.content();
    let infos = content.built::<GltfInfos>();
    for (id, def) in loading.rows::<PropDef>(&content) {
        let Some(info) = infos.0.get(def.model.as_str()) else {
            continue;
        };
        let mut check = RowCheck::default();
        for (name, material) in &def.materials {
            let material = material.get(&content);
            let pictured = material.color.is_some() || material.normal.is_some();
            if pictured && info.unmapped.contains(name) {
                check.error(
                    format!("/materials/{name}"),
                    "nothing to lay a picture by",
                    "a mesh drawn in the material without texture coordinates",
                    "a model whose meshes of the material all carry texture coordinates",
                );
            }
            if material.normal.is_some() && info.untangented.contains(name) {
                check.error(
                    format!("/materials/{name}"),
                    "nothing to turn a relief by",
                    "a mesh drawn in the material without tangents",
                    "a model whose meshes of the material all carry tangents",
                );
            }
        }
        loading.row_errors(id, check);
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

fn area_of(points: &[Pos<Meters>]) -> f32 {
    let count = points.len();
    (0..count)
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % count]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f32>()
        .abs()
        / 2.0
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

/// Whether two outlines share any ground: a corner of one lies within the other, or their edges
/// cross.
fn outlines_meet(a: &[Pos<Meters>], b: &[Pos<Meters>]) -> bool {
    a.iter().any(|&corner| inside_polygon(b, corner))
        || b.iter().any(|&corner| inside_polygon(a, corner))
        || edges(a).any(|(p1, p2)| edges(b).any(|(q1, q2)| segments_cross(p1, p2, q1, q2)))
}

/// Each edge of a polygon, from each corner to the next round it.
fn edges(points: &[Pos<Meters>]) -> impl Iterator<Item = (Pos<Meters>, Pos<Meters>)> + '_ {
    let count = points.len();
    (0..count).map(move |index| (points[index], points[(index + 1) % count]))
}

fn segments_cross(p1: Pos<Meters>, p2: Pos<Meters>, q1: Pos<Meters>, q2: Pos<Meters>) -> bool {
    let side = |a: Pos<Meters>, b: Pos<Meters>, c: Pos<Meters>| {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    };
    let (d1, d2) = (side(q1, q2, p1), side(q1, q2, p2));
    let (d3, d4) = (side(p1, p2, q1), side(p1, p2, q2));
    (d1 * d2 < 0.0) && (d3 * d4 < 0.0)
}
