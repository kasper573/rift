//! How the maps draw an area: as a survey map does. Each terrain is a smooth shape in the colour
//! its ground shows from above, the relief is shaded from the north-west and traced in contour
//! lines, each water is tinted by its depth within its shore line, and the props are drawn from
//! their own plans from above: every prop large enough to tell apart in the picture as its
//! outline, and the smaller ones gathered into stands wherever they grow close together. Every
//! shape is traced from the ground's grids and drawn with antialiasing at the picture's own scale,
//! so the pictures of every level are as sharp as their pixels allow.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::color::{ColorToPacked, LinearRgba, Oklcha, Srgba};
use bevy::math::Vec2;
use contour::ContourBuilder;
use geo::{
    Area as _, Centroid, Coord, LineString, MultiLineString, MultiPolygon, Polygon, Simplify,
};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use super::baked::MapTileKey;
use super::pictures::{MAP_PICTURE_PIXELS, MapDetail, MapTile, Pixels};
use crate::core::content::media::GltfInfos;
use crate::core::content::{Content, ContentProblem, FileHash, FileHashes};
use crate::core::ground::{
    GroundSize, Heightfield, Level, OVERVIEW_STRIDE, PaintMap, SAMPLES_PER_METER, TileAt, WaterMap,
    WaterSurface,
};
use crate::core::math::{Pos, Rect};
use crate::core::render::world::gpu::impostor::{PLAN_PIXELS, PlanFromAbove};
use crate::core::units::{Degrees, MeterSize, Meters};
use crate::systems::area::prop::{Footprint, PropDef};
use crate::systems::area::terrain::TerrainDef;
use crate::systems::area::{Area, PropId};
use crate::systems::water::WaterDef;

/// Every prop's plan from above, as far as the impostors have come.
pub type PropPlans = HashMap<PropId, Arc<PlanFromAbove>>;

/// Everything the maps draw of an area, ready to draw any of its squares from on any thread.
pub struct Survey {
    detail: MapDetail,
    size: GroundSize,
    ground: SurveyGround,
    terrain_fills: Vec<[u8; 3]>,
    water_looks: Vec<WaterLook>,
    /// The plan each kind of prop the area stands that takes up ground is drawn from, each once;
    /// none for a prop whose plan never came.
    shapes: Vec<Option<PropShape>>,
    group_fills: Vec<[u8; 3]>,
    placed: Vec<Placed>,
    cells: PropCells,
    /// The files of the area that every square of it is drawn from, and the style it is drawn in.
    whole: blake3::Hash,
    /// The files of each kind of prop among `shapes`.
    prop_files: Vec<blake3::Hash>,
    /// The file of each tile of each of the ground's grids, by the tile's place.
    tile_files: HashMap<TileAt, blake3::Hash>,
}

impl Survey {
    /// The survey of `area` of `content`, its maps drawn at `detail`, its props from `plans`.
    pub fn new(content: &Content, area: &Area, detail: MapDetail, plans: &PropPlans) -> Survey {
        let placements: Vec<_> = area
            .props
            .iter()
            .chain(&area.stood)
            .filter(|placed| !matches!(placed.prop.get(content).footprint, Footprint::None))
            .collect();
        let mut props: Vec<PropId> = placements.iter().map(|placed| placed.prop).collect();
        props.sort_unstable_by(|one, other| one.path(content).cmp(other.path(content)));
        props.dedup();
        let index: HashMap<PropId, u32> = props
            .iter()
            .enumerate()
            .map(|(at, prop)| (*prop, at as u32))
            .collect();
        let reaches: Vec<f32> = props.iter().map(|prop| reach_of(content, *prop)).collect();
        let mut shapes: Vec<Option<PropShape>> = props
            .iter()
            .map(|prop| {
                let plan = plans.get(prop)?;
                PropShape::of(plan, &prop.get(content).footprint)
            })
            .collect();
        let group_fills = group(&mut shapes);
        let placed: Vec<Placed> = placements
            .iter()
            .map(|placed| {
                let prop = index[&placed.prop];
                Placed {
                    at: placed.at,
                    yaw: placed.yaw,
                    scale: placed.scale,
                    prop,
                    reach: reaches[prop as usize] * placed.scale,
                }
            })
            .collect();
        let cells = PropCells::of(&placed, area.size.bounds());
        let hashes = &content.built::<FileHashes>().0;
        let hash_of = |path: &str| hashes.get(path).map_or("", |hash| hash.hash.as_str());
        let mut whole = blake3::Hasher::new();
        whole.update(&SURVEY_STYLE.to_le_bytes());
        whole.update(&detail.0.to_le_bytes());
        whole.update(&MAP_PICTURE_PIXELS.to_le_bytes());
        for terrain in &area.terrains {
            whole.update(hash_of(terrain.path(content)).as_bytes());
        }
        for water in &area.waters {
            whole.update(hash_of(water.path(content)).as_bytes());
        }
        let prop_files: Vec<blake3::Hash> = props
            .iter()
            .map(|prop| {
                let def = prop.get(content);
                let mut files = blake3::Hasher::new();
                files.update(hash_of(prop.path(content)).as_bytes());
                files.update(hash_of(def.model.as_str()).as_bytes());
                for material in def.materials.values() {
                    files.update(hash_of(material.path(content)).as_bytes());
                }
                files.finalize()
            })
            .collect();
        for files in &prop_files {
            whole.update(files.as_bytes());
        }
        let ground = SurveyGround {
            heights: area.heights.clone(),
            paint: area.paint.clone(),
            water: area.water.clone(),
            waters: area.water_map.clone(),
        };
        let size = area.ground_size();
        let tile_files = size
            .each_tile()
            .map(|at| (at, ground.files_of(at, hashes)))
            .collect();
        Survey {
            detail,
            size,
            ground,
            terrain_fills: area
                .terrains
                .iter()
                .map(|terrain| terrain_fill(terrain.get(content)))
                .collect(),
            water_looks: area
                .waters
                .iter()
                .map(|water| WaterLook::of(water.get(content)))
                .collect(),
            shapes,
            group_fills,
            placed,
            cells,
            whole: whole.finalize(),
            prop_files,
            tile_files,
        }
    }

    /// The detail the area's maps are drawn in.
    pub fn detail(&self) -> MapDetail {
        self.detail
    }

    /// The key of the picture of `tile`: what it shows of the area, and the style it is drawn in.
    pub fn key(&self, tile: MapTile) -> MapTileKey {
        let rect = tile.rect(self.detail);
        let reached = reached(rect);
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.whole.as_bytes());
        hasher.update(&[tile.level]);
        hasher.update(&tile.column.to_le_bytes());
        hasher.update(&tile.row.to_le_bytes());
        for at in self.size.tiles_within(reached) {
            hasher.update(self.tile_files[&at].as_bytes());
        }
        for placed in self.cells.near(&self.placed, reached) {
            hasher.update(self.prop_files[placed.prop as usize].as_bytes());
            for value in [placed.at.x, placed.at.y, placed.yaw.0, placed.scale] {
                hasher.update(&value.to_le_bytes());
            }
        }
        MapTileKey(hasher.finalize().to_hex().to_string())
    }

    /// The picture of `tile`, its pixels opaque; refused where a tile of the ground it shows
    /// doesn't read.
    pub fn draw(&self, tile: MapTile) -> Result<Pixels, ContentProblem> {
        let rect = tile.rect(self.detail);
        let canvas = Canvas {
            origin: rect.origin,
            pixel: rect.width() / MAP_PICTURE_PIXELS as f32,
        };
        let finer_than_overview = canvas.pixel < OVERVIEW_STRIDE as f32 / SAMPLES_PER_METER as f32;
        let ground = if finer_than_overview {
            self.ground
                .holding(self.size.tiles_within(reached(rect)).collect())?
        } else {
            self.ground.unloaded()
        };
        let grids = Grids::sample(&canvas, &ground);
        let mut picture =
            Pixmap::new(MAP_PICTURE_PIXELS, MAP_PICTURE_PIXELS).expect("a picture's pixels");
        self.fill_terrains(&mut picture, &grids);
        shade_relief(&mut picture, &canvas, &grids.heights);
        trace_contours(&mut picture, &canvas, &grids.heights);
        self.fill_waters(&mut picture, &canvas, &grids);
        self.draw_props(&mut picture, &canvas, rect);
        Ok(picture.take())
    }

    /// The fill of the terrain most of the picture shows, then each other terrain's shape smoothed
    /// over it, so the ground's pixels never show at any scale.
    fn fill_terrains(&self, picture: &mut Pixmap, grids: &Grids) {
        let fill_of = |terrain: u8| {
            self.terrain_fills
                .get(usize::from(terrain))
                .copied()
                .unwrap_or(PAPER)
        };
        let mut counts = [0usize; 256];
        for terrain in &grids.terrains {
            counts[usize::from(*terrain)] += 1;
        }
        let most = (0..counts.len())
            .max_by_key(|terrain| counts[*terrain])
            .unwrap_or_default();
        let [r, g, b] = fill_of(most as u8);
        picture.fill(tiny_skia::Color::from_rgba8(r, g, b, u8::MAX));
        for (terrain, count) in counts.iter().enumerate() {
            if *count == 0 || terrain == most {
                continue;
            }
            let painted = |at: usize| usize::from(grids.terrains[at]) == terrain;
            let Some(mut shape) = Field::around(painted, TERRAIN_SMOOTHING, |at| {
                f32::from(u8::from(painted(at)))
            }) else {
                continue;
            };
            shape.blur(TERRAIN_SMOOTHING);
            fill(picture, &shape.traced(0.5), fill_of(terrain as u8));
        }
    }

    /// Each water tinted by its depth, deeper in darker bands, within its shore line.
    fn fill_waters(&self, picture: &mut Pixmap, canvas: &Canvas, grids: &Grids) {
        let mut present: Vec<u8> = grids
            .depths
            .iter()
            .zip(&grids.waters)
            .filter(|(depth, _)| **depth > 0.0)
            .map(|(_, water)| *water)
            .collect();
        present.sort_unstable();
        present.dedup();
        for water in present {
            let Some(look) = self.water_looks.get(usize::from(water)) else {
                continue;
            };
            let of_it = |at: usize| grids.waters[at] == water && grids.depths[at] > DRY.0;
            let Some(mut depths) = Field::around(of_it, WATER_SMOOTHING, |at| {
                if of_it(at) { grids.depths[at] } else { DRY.0 }
            }) else {
                continue;
            };
            depths.blur(WATER_SMOOTHING);
            for (band, fill_color) in WATER_BANDS.iter().zip(look.bands) {
                fill(picture, &depths.traced(band.0), fill_color);
            }
            let shore = depths.lines(&[0.0]);
            let width = by_scale(canvas.pixel, SHORE_WIDTH);
            for line in shore {
                stroke(picture, &line, look.shore, u8::MAX, width);
            }
        }
    }

    /// The props reaching into `rect`: every one large enough to tell apart as its outline, the
    /// smaller ones gathered into stands under them where they grow close together.
    fn draw_props(&self, picture: &mut Pixmap, canvas: &Canvas, rect: Rect<Meters>) {
        let mut outlined: Vec<(&Placed, &PropShape)> = Vec::new();
        let mut stands: Vec<Option<Vec<f32>>> = vec![None; self.group_fills.len()];
        for placed in self.cells.near(&self.placed, reached(rect)) {
            let Some(shape) = &self.shapes[placed.prop as usize] else {
                continue;
            };
            if shape.diameter * placed.scale / canvas.pixel >= shape.outlined_from() {
                outlined.push((placed, shape));
                continue;
            }
            let (column, row) = canvas.grid_at(placed.at);
            if column >= GRID || row >= GRID {
                continue;
            }
            let cover = stands[shape.group].get_or_insert_with(|| vec![0.0; GRID * GRID]);
            cover[grid_index(column, row)] +=
                shape.area * placed.scale * placed.scale / (canvas.pixel * canvas.pixel);
        }
        self.fill_stands(picture, canvas, &stands);
        outlined.sort_by(|(_, one), (_, other)| one.kept.total_cmp(&other.kept));
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        for (placed, shape) in outlined {
            let wide = placed.scale / canvas.pixel;
            let (sin, cos) = placed.yaw.radians().0.sin_cos();
            let at = canvas.picture_at(placed.at);
            let placing =
                Transform::from_row(cos * wide, sin * wide, -sin * wide, cos * wide, at.x, at.y);
            let diameter = shape.diameter * wide;
            let disc;
            let (path, transform, unit) = if diameter < SHAPED_PIXELS {
                let mut middle = tiny_skia::Point::from_xy(shape.middle.x, shape.middle.y);
                placing.map_point(&mut middle);
                disc = PathBuilder::from_circle(middle.x, middle.y, diameter / 2.0);
                let Some(disc) = &disc else {
                    continue;
                };
                (disc, Transform::identity(), 1.0)
            } else {
                (&shape.path, placing, wide)
            };
            let [r, g, b] = shape.fill;
            paint.set_color_rgba8(r, g, b, u8::MAX);
            picture.fill_path(path, &paint, FillRule::EvenOdd, transform, None);
            let [r, g, b] = INK;
            paint.set_color_rgba8(r, g, b, u8::MAX);
            let stroke = Stroke {
                width: (diameter * OUTLINE_SHARE).clamp(OUTLINE_THINNEST, OUTLINE_WIDEST) / unit,
                ..Stroke::default()
            };
            picture.stroke_path(path, &paint, &stroke, transform, None);
        }
    }

    /// The stands the props too small to outline make where they grow close enough together, the
    /// gaps between them closed, each part of a stand in the colour of the kind of prop most of
    /// it is, and edged.
    fn fill_stands(&self, picture: &mut Pixmap, canvas: &Canvas, stands: &[Option<Vec<f32>>]) {
        if stands.iter().all(Option::is_none) {
            return;
        }
        let mut total = Field::whole(|at| stands.iter().flatten().map(|cover| cover[at]).sum());
        total.blur(STAND_GATHERING);
        let mut stood = Field::whole(|at| f32::from(u8::from(total.values[at] > STAND_COVER)));
        let closing = (STAND_CLOSING.0 / canvas.pixel).min(STAND_CLOSING_PIXELS);
        if closing >= 1.0 {
            for threshold in [CLOSING_EDGE, 1.0 - CLOSING_EDGE] {
                stood.blur(closing / CLOSING_SIGMAS);
                for stood in &mut stood.values {
                    *stood = f32::from(u8::from(*stood > threshold));
                }
            }
        }
        stood.blur(STAND_SMOOTHING);
        let shares: Vec<Option<Field>> = stands
            .iter()
            .map(|cover| {
                cover.as_ref().map(|cover| {
                    let mut share = Field::whole(|at| cover[at]);
                    share.blur(STAND_GATHERING);
                    share
                })
            })
            .collect();
        let most: Vec<usize> = (0..GRID * GRID)
            .map(|at| {
                shares
                    .iter()
                    .enumerate()
                    .filter_map(|(group, share)| Some((group, share.as_ref()?.values[at])))
                    .max_by(|(_, one), (_, other)| one.total_cmp(other))
                    .map_or(0, |(group, _)| group)
            })
            .collect();
        for (group, share) in shares.iter().enumerate() {
            if share.is_none() {
                continue;
            }
            let in_part = |at: usize| stood.values[at] >= 0.5 && most[at] == group;
            let Some(mut part) = Field::around(in_part, STAND_SMOOTHING, |at| {
                f32::from(u8::from(in_part(at)))
            }) else {
                continue;
            };
            part.blur(STAND_SMOOTHING);
            fill(picture, &part.traced(0.5), self.group_fills[group]);
        }
        for line in stood.lines(&[0.5]) {
            stroke(picture, &line, STAND_EDGE, u8::MAX, STAND_EDGE_WIDTH);
        }
    }
}

/// A prop's plan as the maps draw it: its outline in its own metres about its origin, at its
/// size, filled in its colour from above.
#[derive(Clone, Debug)]
struct PropShape {
    /// The outline, holes among its rings.
    path: Path,
    /// The middle of the plan.
    middle: Vec2,
    fill: [u8; 3],
    /// The ground the plan covers, in square metres.
    area: f32,
    /// The width of a disc as large as the plan.
    diameter: f32,
    /// How much of the ground under its plan the prop keeps anyone off: low for a tree over its
    /// trunk, high for a house or a rock. Those that keep less are drawn first, under the rest.
    kept: f32,
    /// The stand it gathers into.
    group: usize,
}

impl PropShape {
    /// How wide the prop must show, in pixels, to be drawn as its outline rather than gathered
    /// into a stand: a canopy only once it shows large, as a map shows woods until it is drawn
    /// large enough to show their trees.
    fn outlined_from(&self) -> f32 {
        if self.kept < CANOPY_KEPT {
            CANOPY_OUTLINE_PIXELS
        } else {
            OUTLINE_PIXELS
        }
    }

    fn of(plan: &PlanFromAbove, footprint: &Footprint) -> Option<PropShape> {
        let side = PLAN_PIXELS as usize;
        let padded = side + 2;
        let mut cover = vec![0.0; padded * padded];
        for row in 0..side {
            for column in 0..side {
                cover[(row + 1) * padded + column + 1] = plan.cover[row * side + column];
            }
        }
        blur_grid(&mut cover, padded, padded, PLAN_SMOOTHING);
        let reach = plan.reach.0;
        let to_model = |at: Coord<f64>| Coord {
            x: f64::from(plan.middle.x) + (at.x - 1.0) / side as f64 * 2.0 * f64::from(reach)
                - f64::from(reach),
            y: f64::from(plan.middle.y) + (at.y - 1.0) / side as f64 * 2.0 * f64::from(reach)
                - f64::from(reach),
        };
        let least = f64::from(2.0 * reach).powi(2) * LEAST_PLAN_PART;
        let traced = ContourBuilder::new(padded, padded, true)
            .contours(
                &cover
                    .iter()
                    .map(|&value| f64::from(value))
                    .collect::<Vec<_>>(),
                &[0.5],
            )
            .ok()?;
        let polygons: Vec<Polygon<f64>> = traced
            .into_iter()
            .flat_map(|contour| contour.into_inner().0.0)
            .map(|polygon| {
                let (exterior, interiors) = polygon.into_inner();
                let model = |ring: LineString<f64>| ring.0.into_iter().map(to_model).collect();
                Polygon::new(
                    LineString(model(exterior)),
                    interiors
                        .into_iter()
                        .map(|ring| LineString(model(ring)))
                        .collect(),
                )
                .simplify(f64::from(reach) * PLAN_SIMPLIFYING)
            })
            .filter(|polygon| polygon.unsigned_area() >= least)
            .collect();
        let area = polygons.iter().map(geo::Area::unsigned_area).sum::<f64>() as f32;
        if area <= 0.0 {
            return None;
        }
        let mut path = PathBuilder::new();
        for polygon in &polygons {
            for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
                let mut corners = ring.0.iter().map(|at| (at.x as f32, at.y as f32));
                let Some((x, y)) = corners.next() else {
                    continue;
                };
                path.move_to(x, y);
                for (x, y) in corners {
                    path.line_to(x, y);
                }
                path.close();
            }
        }
        let path = path.finish()?;
        let middle = MultiPolygon(polygons).centroid()?;
        let kept: f32 = footprint
            .outlines(Pos::origin(), Default::default(), 1.0)
            .into_iter()
            .map(|corners| {
                Polygon::new(
                    corners
                        .iter()
                        .map(|corner| Coord {
                            x: f64::from(corner.x),
                            y: f64::from(corner.y),
                        })
                        .collect(),
                    Vec::new(),
                )
                .unsigned_area() as f32
            })
            .sum();
        Some(PropShape {
            path,
            middle: Vec2::new(middle.x() as f32, middle.y() as f32),
            fill: srgb_of(plan.color),
            area,
            diameter: 2.0 * (area / std::f32::consts::PI).sqrt(),
            kept: kept / area,
            group: 0,
        })
    }
}

/// A prop as the survey stands it.
struct Placed {
    at: Pos<Meters>,
    yaw: Degrees,
    scale: f32,
    prop: u32,
    /// How far from where it stands its model reaches over the ground.
    reach: f32,
}

/// The area's props by the cell of the ground they stand in.
struct PropCells {
    columns: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
    /// The furthest any prop reaches past its cell.
    reach: f32,
}

impl PropCells {
    fn of(placed: &[Placed], bounds: Rect<Meters>) -> PropCells {
        let columns = (bounds.width() / PROP_CELL.0).ceil().max(1.0) as usize;
        let rows = (bounds.height() / PROP_CELL.0).ceil().max(1.0) as usize;
        let mut cells = vec![Vec::new(); columns * rows];
        for (index, placed) in placed.iter().enumerate() {
            let (column, row) = cell_of(placed.at, columns, rows);
            cells[row * columns + column].push(index as u32);
        }
        PropCells {
            columns,
            rows,
            cells,
            reach: placed.iter().map(|placed| placed.reach).fold(0.0, f32::max),
        }
    }

    /// The props of `placed` whose models reach into `rect`.
    fn near<'a>(
        &'a self,
        placed: &'a [Placed],
        rect: Rect<Meters>,
    ) -> impl Iterator<Item = &'a Placed> {
        let wide = rect.inflate(self.reach, self.reach);
        let (low_column, low_row) = cell_of(wide.min(), self.columns, self.rows);
        let (high_column, high_row) = cell_of(wide.max(), self.columns, self.rows);
        (low_row..=high_row)
            .flat_map(move |row| {
                (low_column..=high_column)
                    .flat_map(move |column| &self.cells[row * self.columns + column])
            })
            .map(move |&index| &placed[index as usize])
            .filter(move |placed| rect.inflate(placed.reach, placed.reach).contains(placed.at))
    }
}

fn cell_of(at: Pos<Meters>, columns: usize, rows: usize) -> (usize, usize) {
    let along =
        |at: f32, count: usize| ((at / PROP_CELL.0).floor().max(0.0) as usize).min(count - 1);
    (along(at.x, columns), along(at.y, rows))
}

/// Where a picture lies over the ground: the corner of its first pixel, and how far a pixel
/// spans. The grids it is drawn from reach `MARGIN` pixels further each way, for what a shape
/// past the picture's edge smooths into it.
struct Canvas {
    origin: Pos<Meters>,
    pixel: f32,
}

impl Canvas {
    /// The middle of the grids' cell `(column, row)` on the ground.
    fn ground_at(&self, column: usize, row: usize) -> Pos<Meters> {
        Pos::new(
            self.origin.x + (column as f32 - MARGIN as f32 + 0.5) * self.pixel,
            self.origin.y + (row as f32 - MARGIN as f32 + 0.5) * self.pixel,
        )
    }

    /// The grids' cell `at` lies in, which may lie past them.
    fn grid_at(&self, at: Pos<Meters>) -> (usize, usize) {
        let along = |at: f32, origin: f32| {
            ((at - origin) / self.pixel + MARGIN as f32)
                .floor()
                .max(0.0) as usize
        };
        (along(at.x, self.origin.x), along(at.y, self.origin.y))
    }

    /// Where on the picture `at` lies, in its pixels.
    fn picture_at(&self, at: Pos<Meters>) -> Vec2 {
        Vec2::new(
            (at.x - self.origin.x) / self.pixel,
            (at.y - self.origin.y) / self.pixel,
        )
    }
}

/// Values over a window of the picture's grids, traced into the shapes and lines they make: the
/// window's first cell, how many cells it is across and down, and its values row by row.
struct Field {
    column: usize,
    row: usize,
    columns: usize,
    rows: usize,
    values: Vec<f32>,
}

impl Field {
    /// `value` of every cell of the grids.
    fn whole(value: impl Fn(usize) -> f32) -> Field {
        Field::over(0, GRID, value)
    }

    /// `value` of every cell of the square of the grids `side` cells along each way from cell
    /// `(first, first)`.
    fn over(first: usize, side: usize, value: impl Fn(usize) -> f32) -> Field {
        Field::window(first, first, side, side, value)
    }

    /// `value` of every cell of the grids as far as a blur `sigma` cells wide reaches from those
    /// `inside` holds for, and a cell past it; none where it holds for none.
    fn around(
        inside: impl Fn(usize) -> bool,
        sigma: f32,
        value: impl Fn(usize) -> f32,
    ) -> Option<Field> {
        let (mut low, mut high) = ((GRID, GRID), (0, 0));
        for row in 0..GRID {
            for column in 0..GRID {
                if inside(grid_index(column, row)) {
                    low = (low.0.min(column), low.1.min(row));
                    high = (high.0.max(column), high.1.max(row));
                }
            }
        }
        if low.0 > high.0 {
            return None;
        }
        let reach = blur_reach(sigma) + 1;
        let (column, row) = (low.0.saturating_sub(reach), low.1.saturating_sub(reach));
        let (last_column, last_row) = (
            (high.0 + reach).min(GRID - 1),
            (high.1 + reach).min(GRID - 1),
        );
        Some(Field::window(
            column,
            row,
            last_column - column + 1,
            last_row - row + 1,
            value,
        ))
    }

    fn window(
        column: usize,
        row: usize,
        columns: usize,
        rows: usize,
        value: impl Fn(usize) -> f32,
    ) -> Field {
        Field {
            column,
            row,
            columns,
            rows,
            values: (row..row + rows)
                .flat_map(|row| (column..column + columns).map(move |column| (column, row)))
                .map(|(column, row)| value(grid_index(column, row)))
                .collect(),
        }
    }

    fn blur(&mut self, sigma: f32) {
        blur_grid(&mut self.values, self.columns, self.rows, sigma);
    }

    /// The shapes where the values are at least `threshold`, in the grids' cells.
    fn traced(&self, threshold: f32) -> MultiPolygon<f64> {
        self.builder()
            .contours(&self.wide(), &[f64::from(threshold)])
            .ok()
            .and_then(|contours| contours.into_iter().next())
            .map_or_else(
                || MultiPolygon(Vec::new()),
                |contour| contour.into_inner().0,
            )
    }

    /// The lines along which the values cross each of `thresholds`, in the grids' cells.
    fn lines(&self, thresholds: &[f32]) -> Vec<MultiLineString<f64>> {
        let thresholds: Vec<f64> = thresholds.iter().map(|value| f64::from(*value)).collect();
        self.builder()
            .lines(&self.wide(), &thresholds)
            .map(|lines| lines.into_iter().map(|line| line.into_inner().0).collect())
            .unwrap_or_default()
    }

    fn builder(&self) -> ContourBuilder {
        ContourBuilder::new(self.columns, self.rows, true)
            .x_origin(self.column as f64)
            .y_origin(self.row as f64)
    }

    fn wide(&self) -> Vec<f64> {
        self.values.iter().map(|value| f64::from(*value)).collect()
    }
}

/// The grids of an area's ground the survey draws from.
struct SurveyGround {
    heights: Heightfield,
    paint: PaintMap,
    water: WaterSurface,
    waters: WaterMap,
}

impl SurveyGround {
    /// This ground with every one of `tiles` read, for this copy alone.
    fn holding(&self, tiles: Vec<TileAt>) -> Result<SurveyGround, ContentProblem> {
        Ok(SurveyGround {
            heights: self.heights.holding(tiles.iter().copied())?,
            paint: self.paint.holding(tiles.iter().copied())?,
            water: self.water.holding(tiles.iter().copied())?,
            waters: self.waters.holding(tiles)?,
        })
    }

    /// This ground as its overviews hold it.
    fn unloaded(&self) -> SurveyGround {
        SurveyGround {
            heights: self.heights.unloaded(),
            paint: self.paint.unloaded(),
            water: self.water.unloaded(),
            waters: self.waters.unloaded(),
        }
    }

    /// The files of the tile at `at` of every grid: the hash `hashes` holds of the file a tile
    /// still holds its values as, else of the file it would be written as, so the files and the
    /// same values made in memory hash alike.
    fn files_of(&self, at: TileAt, hashes: &BTreeMap<String, FileHash>) -> blake3::Hash {
        let file = |stored: Option<&str>, encoded: &dyn Fn() -> Option<Arc<[u8]>>| match stored
            .and_then(|path| hashes.get(path))
        {
            Some(hash) => hash.hash.clone(),
            None => FileHash::of(encoded().as_deref().unwrap_or_default()).hash,
        };
        let mut files = blake3::Hasher::new();
        for hash in [
            file(self.heights.stored_at(at), &|| self.heights.encode_tile(at)),
            file(self.paint.stored_at(at), &|| self.paint.encode_tile(at)),
            file(self.water.stored_at(at), &|| self.water.encode_tile(at)),
            file(self.waters.stored_at(at), &|| self.waters.encode_tile(at)),
        ] {
            files.update(hash.as_bytes());
        }
        files.finalize()
    }
}

/// The ground under every cell of a picture's grids: its height, its terrain, how deep the water
/// stands over it (how far under the water's level it lies where it is dry beside the water,
/// `DRY` further off) and which water lies there.
struct Grids {
    heights: Vec<f32>,
    terrains: Vec<u8>,
    depths: Vec<f32>,
    waters: Vec<u8>,
}

impl Grids {
    fn sample(canvas: &Canvas, ground: &SurveyGround) -> Grids {
        let SurveyGround {
            heights,
            paint,
            water,
            waters,
        } = ground;
        let origin = Heightfield::sample_pos(0, 0);
        let per_meter = SAMPLES_PER_METER as f32;
        let (mut ground, mut level) = (heights.reader(), water.reader());
        let (mut painted, mut which) = (paint.reader(), waters.reader());
        let cells = GRID * GRID;
        let mut grids = Grids {
            heights: Vec::with_capacity(cells),
            terrains: Vec::with_capacity(cells),
            depths: Vec::with_capacity(cells),
            waters: Vec::with_capacity(cells),
        };
        let mut levels: Vec<Option<f32>> = Vec::with_capacity(cells);
        for row in 0..GRID {
            for column in 0..GRID {
                let at = canvas.ground_at(column, row);
                let x = ((at.x - origin.x) * per_meter).max(0.0);
                let y = ((at.y - origin.y) * per_meter).max(0.0);
                let (left, top) = (x.floor() as u32, y.floor() as u32);
                let (across, down) = (x.fract(), y.fract());
                let mut height = |column: u32, row: u32| ground.sample(column, row).0;
                let upper = height(left, top) * (1.0 - across) + height(left + 1, top) * across;
                let lower =
                    height(left, top + 1) * (1.0 - across) + height(left + 1, top + 1) * across;
                grids.heights.push(upper * (1.0 - down) + lower * down);
                let (nearest_column, nearest_row) = (x.round() as u32, y.round() as u32);
                let surface = level.index(nearest_column, nearest_row);
                let wet = surface.level() > ground.index(nearest_column, nearest_row).level();
                levels.push(wet.then(|| level.sample(nearest_column, nearest_row).0));
                let (column, row) = paint.pixel_at(at);
                grids.terrains.push(painted.index(column, row));
                let (column, row) = waters.pixel_at(at);
                grids.waters.push(which.index(column, row));
            }
        }
        for _ in 0..LEVEL_RUNS_ON {
            let reached = levels.clone();
            for (at, level) in levels.iter_mut().enumerate() {
                if level.is_some() {
                    continue;
                }
                *level = around(at)
                    .filter_map(|there| reached[there])
                    .reduce(f32::max);
            }
        }
        grids.depths = levels
            .iter()
            .zip(&grids.heights)
            .map(|(level, height)| level.map_or(DRY.0, |level| (level - height).max(DRY.0)))
            .collect();
        grids
    }
}

/// How a water is tinted on the maps: a band for each of `WATER_BANDS` deeper, and its shore line.
#[derive(Clone, Copy, Debug)]
struct WaterLook {
    bands: [[u8; 3]; 4],
    shore: [u8; 3],
}

impl WaterLook {
    /// The tints of a water with its own hue, as light as a map shows shallows and growing
    /// darker and richer with the depth.
    fn of(def: &WaterDef) -> WaterLook {
        let [r, g, b] = def.depths.0;
        let depths = Oklcha::from(Srgba::rgb_u8(r, g, b));
        let tint = |(lightness, chroma): (f32, f32)| {
            srgb_of(LinearRgba::from(Oklcha::new(
                lightness,
                depths.chroma * chroma,
                depths.hue,
                1.0,
            )))
        };
        WaterLook {
            bands: WATER_TINTS.map(tint),
            shore: tint(SHORE_TINT),
        }
    }
}

/// The fill of `def`'s ground: its colour lightened toward the paper.
fn terrain_fill(def: &TerrainDef) -> [u8; 3] {
    let [r, g, b, _] = def.color.0.to_be_bytes();
    mixed([r, g, b], PAPER, TERRAIN_PAPER)
}

/// How far `prop`'s model reaches over the ground from where it stands, at its size, past it as
/// far as its shaders draw.
fn reach_of(content: &Content, prop: PropId) -> f32 {
    let def: &PropDef = prop.get(content);
    let overhang = def
        .materials
        .values()
        .map(|material| material.get(content).overhang.0)
        .fold(0.0, f32::max);
    let model = content
        .built::<GltfInfos>()
        .0
        .get(def.model.as_str())
        .map(|info| {
            let (low, high) = (info.bounds.min, info.bounds.max);
            [low.x, high.x]
                .into_iter()
                .flat_map(|x| [low.z, high.z].map(|z| Vec2::new(x, z).length()))
                .fold(0.0, f32::max)
        });
    let footprint = def.footprint.reach().map(|reach| reach.0);
    model.or(footprint).unwrap_or(0.0) + overhang
}

/// Sorts the props into the kinds their stands are coloured by, by their colours from above, and
/// the fill of each kind: as many kinds as `STAND_GROUPS` at the most, each the mean colour of
/// its props, each weighed by the ground one of it covers. How many of each the area stands
/// counts for nothing, so standing one more changes no square but those it stands in.
fn group(shapes: &mut [Option<PropShape>]) -> Vec<[u8; 3]> {
    let colors: Vec<(bevy::math::Vec3, f32, usize)> = shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| {
            let shape = shape.as_ref()?;
            let [r, g, b] = shape.fill;
            Some((
                bevy::math::Vec3::new(f32::from(r), f32::from(g), f32::from(b)),
                shape.area,
                index,
            ))
        })
        .collect();
    let Some(heaviest) = colors
        .iter()
        .max_by(|(_, one, _), (_, other, _)| one.total_cmp(other))
    else {
        return vec![PAPER];
    };
    let mut middles = vec![heaviest.0];
    while middles.len() < STAND_GROUPS.min(colors.len()) {
        let farthest = colors
            .iter()
            .map(|(color, _, _)| {
                let near = middles
                    .iter()
                    .map(|middle| middle.distance(*color))
                    .fold(f32::INFINITY, f32::min);
                (*color, near)
            })
            .max_by(|(_, one), (_, other)| one.total_cmp(other))
            .expect("a colour");
        if farthest.1 <= 0.0 {
            break;
        }
        middles.push(farthest.0);
    }
    let nearest = |middles: &[bevy::math::Vec3], color: bevy::math::Vec3| {
        middles
            .iter()
            .enumerate()
            .min_by(|(_, one), (_, other)| one.distance(color).total_cmp(&other.distance(color)))
            .map_or(0, |(group, _)| group)
    };
    for _ in 0..GROUPING_ROUNDS {
        let mut sums = vec![(bevy::math::Vec3::ZERO, 0.0); middles.len()];
        for (color, weight, _) in &colors {
            let group = nearest(&middles, *color);
            sums[group].0 += *color * weight.max(f32::EPSILON);
            sums[group].1 += weight.max(f32::EPSILON);
        }
        for (middle, (sum, weight)) in middles.iter_mut().zip(sums) {
            if weight > 0.0 {
                *middle = sum / weight;
            }
        }
    }
    for (color, _, index) in &colors {
        if let Some(shape) = &mut shapes[*index] {
            shape.group = nearest(&middles, *color);
        }
    }
    middles
        .iter()
        .map(|middle| {
            let color = [middle.x, middle.y, middle.z].map(|channel| channel.round() as u8);
            mixed(color, PAPER, STAND_PAPER)
        })
        .collect()
}

/// Shades `picture` by the slope of `heights` under the light from the north-west, as a survey
/// map's relief is: darker on the slopes facing away, lighter on those facing it, flat ground as
/// it is. The relief is drawn steeper the more ground a pixel spans, so hills still read when a
/// map is zoomed out.
fn shade_relief(picture: &mut Pixmap, canvas: &Canvas, heights: &[f32]) {
    let sharpest = 1.0 / SAMPLES_PER_METER as f32;
    let zoomed_out = ((canvas.pixel / sharpest).log2() / RELIEF_LEVELS).clamp(0.0, 1.0);
    let steepened = RELIEF_STEEPENED.0 + (RELIEF_STEEPENED.1 - RELIEF_STEEPENED.0) * zoomed_out;
    let (azimuth, altitude) = (LIGHT_AZIMUTH.to_radians(), LIGHT_ALTITUDE.to_radians());
    let data = picture.data_mut();
    for row in 0..MAP_PICTURE_PIXELS as usize {
        for column in 0..MAP_PICTURE_PIXELS as usize {
            let (x, y) = (column + MARGIN, row + MARGIN);
            let east = (heights[grid_index(x + 1, y)] - heights[grid_index(x - 1, y)])
                / (2.0 * canvas.pixel)
                * steepened;
            let south = (heights[grid_index(x, y + 1)] - heights[grid_index(x, y - 1)])
                / (2.0 * canvas.pixel)
                * steepened;
            let slope = east.hypot(south).atan();
            let aspect = (-east).atan2(south);
            let lit = (altitude.sin() * slope.cos()
                + altitude.cos() * slope.sin() * (azimuth - aspect).cos())
            .clamp(0.0, 1.0);
            let dark = ((RELIEF_DARK_FROM - lit) * RELIEF_DARK_RISE).clamp(0.0, 1.0) * RELIEF_DARK;
            let light =
                ((lit - RELIEF_LIGHT_FROM) * RELIEF_LIGHT_RISE).clamp(0.0, 1.0) * RELIEF_LIGHT;
            let (tone, share) = if dark > light {
                (SHADE, dark)
            } else {
                (LIGHT, light)
            };
            let at = (row * MAP_PICTURE_PIXELS as usize + column) * 4;
            for (channel, tone) in data[at..at + 3].iter_mut().zip(tone) {
                *channel =
                    (f32::from(*channel) * (1.0 - share) + f32::from(tone) * share).round() as u8;
            }
        }
    }
}

/// The contour lines of `heights`, as far apart as the picture's scale reads them, every fifth
/// one bolder.
fn trace_contours(picture: &mut Pixmap, canvas: &Canvas, heights: &[f32]) {
    let interval = CONTOUR_INTERVALS
        .iter()
        .copied()
        .find(|interval| *interval >= canvas.pixel * CONTOUR_PIXELS)
        .unwrap_or(CONTOUR_INTERVALS[CONTOUR_INTERVALS.len() - 1]);
    let heights = Field::over(
        MARGIN - CONTOUR_MARGIN,
        MAP_PICTURE_PIXELS as usize + 2 * CONTOUR_MARGIN,
        |at| heights[at],
    );
    let (low, high) = heights
        .values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), height| {
            (low.min(*height), high.max(*height))
        });
    let steps: Vec<i64> =
        ((low / interval).ceil() as i64..=(high / interval).floor() as i64).collect();
    let thresholds: Vec<f32> = steps.iter().map(|step| *step as f32 * interval).collect();
    for (step, line) in steps.iter().zip(heights.lines(&thresholds)) {
        let (width, alpha) = if step.rem_euclid(CONTOUR_MAJOR) == 0 {
            CONTOUR_MAJOR_LINE
        } else {
            CONTOUR_MINOR_LINE
        };
        stroke(picture, &line, CONTOUR, alpha, width);
    }
}

fn fill(picture: &mut Pixmap, shapes: &MultiPolygon<f64>, color: [u8; 3]) {
    let mut path = PathBuilder::new();
    for polygon in &shapes.0 {
        for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
            ring_onto(&mut path, &ring.0, true);
        }
    }
    let Some(path) = path.finish() else {
        return;
    };
    let [r, g, b] = color;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color_rgba8(r, g, b, u8::MAX);
    picture.fill_path(
        &path,
        &paint,
        FillRule::EvenOdd,
        Transform::identity(),
        None,
    );
}

fn stroke(
    picture: &mut Pixmap,
    lines: &MultiLineString<f64>,
    color: [u8; 3],
    alpha: u8,
    width: f32,
) {
    let mut path = PathBuilder::new();
    for line in &lines.0 {
        ring_onto(&mut path, &line.0, false);
    }
    let Some(path): Option<Path> = path.finish() else {
        return;
    };
    let [r, g, b] = color;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color_rgba8(r, g, b, alpha);
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    picture.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// Adds `corners`, traced over the picture's grids, to `path` in the picture's pixels.
fn ring_onto(path: &mut PathBuilder, corners: &[Coord<f64>], closed: bool) {
    let mut corners = corners
        .iter()
        .map(|at| (at.x as f32 - MARGIN as f32, at.y as f32 - MARGIN as f32));
    let Some((x, y)) = corners.next() else {
        return;
    };
    path.move_to(x, y);
    for (x, y) in corners {
        path.line_to(x, y);
    }
    if closed {
        path.close();
    }
}

/// `values`, `columns` × `rows` cells, blurred as by a gaussian `sigma` cells wide, the cells
/// past the edges as those at them.
fn blur_grid(values: &mut [f32], columns: usize, rows: usize, sigma: f32) {
    let reach = blur_reach(sigma) as isize;
    let weights: Vec<f32> = (-reach..=reach)
        .map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let weights: Vec<f32> = weights.iter().map(|weight| weight / total).collect();
    let mut passed = vec![0.0; values.len()];
    let (last_column, last_row) = (columns as isize - 1, rows as isize - 1);
    for row in 0..rows {
        for column in 0..columns {
            passed[row * columns + column] = weights
                .iter()
                .enumerate()
                .map(|(index, weight)| {
                    let along =
                        (column as isize + index as isize - reach).clamp(0, last_column) as usize;
                    values[row * columns + along] * weight
                })
                .sum();
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            values[row * columns + column] = weights
                .iter()
                .enumerate()
                .map(|(index, weight)| {
                    let along = (row as isize + index as isize - reach).clamp(0, last_row) as usize;
                    passed[along * columns + column] * weight
                })
                .sum();
        }
    }
}

/// How many cells a blur `sigma` cells wide reaches each way.
fn blur_reach(sigma: f32) -> usize {
    (3.0 * sigma).ceil() as usize
}

/// The cells beside the grids' cell `at`, each way and across.
fn around(at: usize) -> impl Iterator<Item = usize> {
    let (column, row) = ((at % GRID) as isize, (at / GRID) as isize);
    (-1..=1)
        .flat_map(move |down| (-1..=1).map(move |across| (column + across, row + down)))
        .filter(move |&(there_column, there_row)| {
            (there_column, there_row) != (column, row)
                && there_column >= 0
                && there_row >= 0
                && (there_column as usize) < GRID
                && (there_row as usize) < GRID
        })
        .map(|(column, row)| grid_index(column as usize, row as usize))
}

fn grid_index(column: usize, row: usize) -> usize {
    row * GRID + column
}

/// The ground a picture of `rect` is drawn from: as far past it as its grids reach.
fn reached(rect: Rect<Meters>) -> Rect<Meters> {
    let margin = MARGIN as f32 * rect.width() / MAP_PICTURE_PIXELS as f32;
    rect.inflate(margin, margin)
}

/// `widths`, the widest drawn on the sharpest pictures and the thinnest on the coarsest, as wide
/// as a picture of pixels spanning `pixel` draws it.
fn by_scale(pixel: f32, (sharpest, coarsest): (f32, f32)) -> f32 {
    let sharpest_pixel = 1.0 / SAMPLES_PER_METER as f32;
    let zoomed_out = ((pixel / sharpest_pixel).log2() / RELIEF_LEVELS).clamp(0.0, 1.0);
    sharpest + (coarsest - sharpest) * zoomed_out
}

fn srgb_of(color: LinearRgba) -> [u8; 3] {
    let [r, g, b, _] = Srgba::from(color).to_u8_array();
    [r, g, b]
}

/// `color` taken `share` of the way to `toward`.
fn mixed(color: [u8; 3], toward: [u8; 3], share: f32) -> [u8; 3] {
    let mut out = color;
    for (channel, toward) in out.iter_mut().zip(toward) {
        *channel = (f32::from(*channel) * (1.0 - share) + f32::from(toward) * share).round() as u8;
    }
    out
}

/// The style the maps are drawn in, part of every picture's key: a change to how they are drawn
/// changes it, so every baked map is out of date until it is baked again.
const SURVEY_STYLE: u32 = 1;
/// How many pixels the picture's grids reach past it each way: past everything that a pixel's
/// look is worked out from, so the pictures of neighbouring squares meet without a seam. A stand
/// reaches furthest: its gathering, its closing both ways and its smoothing.
const MARGIN: usize = 24;
/// How many cells the picture's grids are across, and down.
const GRID: usize = MAP_PICTURE_PIXELS as usize + 2 * MARGIN;
/// How far apart the cells the props are found by lie.
const PROP_CELL: Meters = Meters(32.0);

/// The colour of the paper the maps are drawn on, which their fills are lightened toward.
const PAPER: [u8; 3] = [246, 242, 228];
/// The colour outlines are drawn in.
const INK: [u8; 3] = [40, 35, 30];
/// How much of the way toward the paper a terrain's fill is lightened.
const TERRAIN_PAPER: f32 = 0.3;
/// How widely a terrain's shape is smoothed, in pixels.
const TERRAIN_SMOOTHING: f32 = 1.2;

/// The way the relief is lit from, clockwise from the north, and how high, in degrees.
const LIGHT_AZIMUTH: f32 = 315.0;
const LIGHT_ALTITUDE: f32 = 40.0;
/// How much steeper the relief is shaded than it stands, on the sharpest pictures and on those
/// `RELIEF_LEVELS` levels coarser and beyond.
const RELIEF_STEEPENED: (f32, f32) = (1.2, 2.5);
const RELIEF_LEVELS: f32 = 5.0;
/// The tone a slope facing away from the light is shaded toward, from how little light it takes,
/// how fast it darkens and how dark at the most; and the same for a slope facing the light.
const SHADE: [u8; 3] = [70, 80, 110];
const RELIEF_DARK_FROM: f32 = 0.75;
const RELIEF_DARK_RISE: f32 = 1.3;
const RELIEF_DARK: f32 = 0.55;
const LIGHT: [u8; 3] = [255, 245, 215];
const RELIEF_LIGHT_FROM: f32 = 0.85;
const RELIEF_LIGHT_RISE: f32 = 3.0;
const RELIEF_LIGHT: f32 = 0.3;

/// The colour contour lines are drawn in.
const CONTOUR: [u8; 3] = [153, 102, 61];
/// The heights apart contour lines may be drawn, in metres: the first at least as many pixels
/// apart on a slope of one in one as `CONTOUR_PIXELS`.
const CONTOUR_INTERVALS: [f32; 10] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0];
const CONTOUR_PIXELS: f32 = 4.0;
/// How many cells past the picture its contour lines are traced, for their ends to reach its edge.
const CONTOUR_MARGIN: usize = 2;
/// Every how many contour lines one is drawn bolder, and how wide and opaque each kind is.
const CONTOUR_MAJOR: i64 = 5;
const CONTOUR_MAJOR_LINE: (f32, u8) = (0.9, 225);
const CONTOUR_MINOR_LINE: (f32, u8) = (0.35, 140);

/// How far under the water's level dry ground beside the water is taken to lie at the most, and
/// how many cells past the water its level is carried over the ground, so the shore line runs
/// where the ground rises out of the water rather than between the cells.
const DRY: Meters = Meters(-5.0);
const LEVEL_RUNS_ON: usize = 2;
/// How widely a water's depths are smoothed, in pixels.
const WATER_SMOOTHING: f32 = 0.8;
/// The depths each band of a water's tint begins at.
const WATER_BANDS: [Meters; 4] = [Meters(0.0), Meters(2.0), Meters(5.0), Meters(10.0)];
/// Each band's lightness, and its share of the water's own chroma, in Oklch.
const WATER_TINTS: [(f32, f32); 4] = [(0.86, 0.6), (0.8, 0.75), (0.73, 0.9), (0.66, 1.0)];
const SHORE_TINT: (f32, f32) = (0.46, 1.2);
/// How wide the shore line is on the sharpest pictures and the coarsest.
const SHORE_WIDTH: (f32, f32) = (1.4, 0.9);

/// How wide a prop must show, in pixels, to be drawn as its outline rather than gathered into a
/// stand; and how wide a canopy must, a prop keeping anyone off less of the ground under its plan
/// than `CANOPY_KEPT`.
const OUTLINE_PIXELS: f32 = 1.5;
const CANOPY_OUTLINE_PIXELS: f32 = 6.0;
const CANOPY_KEPT: f32 = 0.25;
/// How wide a prop's outline is drawn, as a share of how wide the prop shows, at the least and
/// the most, in pixels.
const OUTLINE_SHARE: f32 = 1.0 / 12.0;
/// How wide a prop must show, in pixels, for its outline to be drawn in its own shape rather than
/// as a disc as large as its plan, which looks the same until then.
const SHAPED_PIXELS: f32 = 6.0;
const OUTLINE_THINNEST: f32 = 0.3;
const OUTLINE_WIDEST: f32 = 1.0;
/// How widely the plan from above is smoothed before its outline is traced, in its texels, how
/// far the outline may stray from it as a share of the plan's reach, and the least part of the
/// plan's square a piece of it must cover to be kept.
const PLAN_SMOOTHING: f32 = 0.8;
const PLAN_SIMPLIFYING: f64 = 0.02;
const LEAST_PLAN_PART: f64 = 0.002;

/// How many kinds of stand there are at the most, and how many rounds sort the props into them.
const STAND_GROUPS: usize = 5;
const GROUPING_ROUNDS: usize = 8;
/// How widely the ground props too small to outline cover is gathered, in pixels, how much of
/// it they must cover there to make a stand, and how widely a stand's edge is smoothed.
const STAND_GATHERING: f32 = 2.5;
const STAND_COVER: f32 = 0.18;
/// How wide a gap between stands is closed, in metres and at the most in pixels, and how a
/// closing is drawn: the stands grown by blurring them `CLOSING_SIGMAS` times less widely and
/// keeping what the blur reaches with more than `CLOSING_EDGE` of a stand, then shrunk back alike.
const STAND_CLOSING: Meters = Meters(24.0);
const STAND_CLOSING_PIXELS: f32 = 4.0;
const CLOSING_SIGMAS: f32 = 2.0;
const CLOSING_EDGE: f32 = 0.02;
const STAND_SMOOTHING: f32 = 1.0;
/// How much of the way toward the paper a stand's fill is lightened, and its edge.
const STAND_PAPER: f32 = 0.1;
const STAND_EDGE: [u8; 3] = [60, 70, 40];
const STAND_EDGE_WIDTH: f32 = 0.4;
