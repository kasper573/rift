use glam::Vec2;
use polyanya::{Coords, Layer, Mesh, Polygon, Triangulation, Vertex};
use rayon::prelude::*;

use crate::core::geometry::corners;
use crate::core::math::{Offset, Pos, Rect, Rng, Size};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterPos, Meters};

/// The ground every walker's body takes, so it keeps this far from what blocks and from the others.
pub const AGENT_RADIUS: Meters = Meters(0.35);

/// Where an area walks: its bounds less everything that blocks, each grown by the walker's radius,
/// cut into convex polygons a straight line crosses freely and strung together where they share an
/// edge. A point is walkable when a polygon holds it; two points connect when their polygons do.
pub struct NavMesh {
    bounds: Rect<Meters>,
    mesh: Mesh,
    polygons: Vec<Vec<Pos<Meters>>>,
    islands: Vec<Island>,
    triangles: Vec<Triangle>,
    /// The triangles by the ground they cover, so a clip passes over only those near it.
    index: SpatialIndex,
}

/// A stretch of ground connected within itself and cut off from the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Island(usize);

impl NavMesh {
    /// `bounds` less each obstacle, every outline grown by `radius` and the bounds shrunk by it, so
    /// a point on the mesh is a place a body of that radius fits, give or take a tenth of the
    /// radius: a grown outline keeps only the points that each add more ground than a sixteenth
    /// of the radius squared, which lets a body a tenth of its radius in at most between the
    /// points kept round an outline its own size, and spares the mesh most of the points a
    /// rounded outline or a run of blocked cells brings.
    pub fn build(
        bounds: Rect<Meters>,
        obstacles: impl IntoIterator<Item = Vec<Pos<Meters>>>,
        radius: Meters,
    ) -> NavMesh {
        let obstacles: Vec<Vec<Pos<Meters>>> = obstacles
            .into_iter()
            .filter(|outline| outline.len() >= 3)
            .collect();
        let ground = bounds.inflate(-radius.0, -radius.0);
        let mut mesh = Mesh {
            layers: vec![triangulate(ground, &obstacles, radius)],
            ..Default::default()
        };
        mesh.reorder_neighbors_ccw_and_fix_corners();
        mesh.update_is_one_way();
        mesh.merge_polygons();
        mesh.search_delta = LOCATE_FUZZ / (LOCATE_RINGS - 1) as f32;
        mesh.search_steps = LOCATE_RINGS;
        mesh.bake();
        let layer = &mesh.layers[0];
        let polygons: Vec<Vec<Pos<Meters>>> = layer
            .polygons
            .iter()
            .map(|polygon| {
                polygon
                    .vertices
                    .iter()
                    .map(|&vertex| pos(layer.vertices[vertex as usize].coords))
                    .collect()
            })
            .collect();
        let islands = islands(layer);
        let triangles: Vec<Triangle> = polygons
            .iter()
            .enumerate()
            .flat_map(|(index, outline)| {
                fan(outline).map(move |corners| Triangle {
                    corners,
                    bounds: Rect::from_points(corners),
                    polygon: index,
                })
            })
            .collect();
        let mut index = SpatialIndex::new(TRIANGLE_CELL);
        index.refile(triangles.iter().map(|triangle| triangle.bounds));
        NavMesh {
            bounds,
            mesh,
            polygons,
            islands,
            triangles,
            index,
        }
    }

    pub fn bounds(&self) -> Rect<Meters> {
        self.bounds
    }

    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    /// The convex polygons the mesh is made of, each going round counter-clockwise.
    pub fn polygons(&self) -> impl Iterator<Item = &[Pos<Meters>]> {
        self.polygons.iter().map(Vec::as_slice)
    }

    pub fn walkable(&self, at: Pos<Meters>) -> bool {
        self.locate(at).is_some()
    }

    pub fn island(&self, at: Pos<Meters>) -> Option<Island> {
        self.locate(at).map(|polygon| self.islands[polygon])
    }

    /// Whether a walk leads from `from` to `to`.
    pub fn connects(&self, from: Pos<Meters>, to: Pos<Meters>) -> bool {
        matches!((self.island(from), self.island(to)), (Some(a), Some(b)) if a == b)
    }

    /// Whether some of the mesh lies within `bounds`.
    pub fn covers(&self, bounds: Rect<Meters>) -> bool {
        self.pieces_within(bounds).next().is_some()
    }

    /// A point on each piece of the mesh within `bounds`, one per polygon the bounds cut.
    pub fn ground_within(&self, bounds: Rect<Meters>) -> Vec<Pos<Meters>> {
        self.pieces_within(bounds)
            .map(|piece| centroid(&piece))
            .collect()
    }

    /// Each polygon's part within `bounds`, of those the bounds cut.
    fn pieces_within(&self, bounds: Rect<Meters>) -> impl Iterator<Item = Vec<Pos<Meters>>> + '_ {
        let clip = corners(bounds);
        self.polygons_within(bounds).filter_map(move |polygon| {
            let piece = clip_polygon(&self.polygons[polygon], &clip);
            (area(&piece) > AREA_EPSILON).then_some(piece)
        })
    }

    /// The polygons with a triangle filed within `reach`, each once.
    fn polygons_within(&self, reach: Rect<Meters>) -> impl Iterator<Item = usize> + '_ {
        let mut polygons: Vec<usize> = self
            .index
            .within(reach)
            .map(|triangle| self.triangles[triangle].polygon)
            .collect();
        polygons.sort_unstable();
        polygons.dedup();
        polygons.into_iter()
    }

    /// The corners of the shortest walk from `from` to `to`, `to` last, or none when no walk leads
    /// there. Each corner is a point to walk straight to from the one before.
    pub fn route(&self, from: Pos<Meters>, to: Pos<Meters>) -> Option<Vec<Pos<Meters>>> {
        let path = self.mesh.path(self.place(from)?, self.place(to)?)?;
        let mut corners: Vec<Pos<Meters>> = path.path.into_iter().map(pos).collect();
        if corners.last() != Some(&to) {
            corners.push(to);
        }
        Some(corners)
    }

    /// The point on the mesh nearest `at`: `at` itself when the mesh holds it.
    pub fn nearest(&self, at: Pos<Meters>) -> Option<Pos<Meters>> {
        if self.walkable(at) {
            return Some(at);
        }
        let mut within = TRIANGLE_CELL;
        loop {
            let reach = Rect::from_points([
                at - Offset::new(within.0, within.0),
                at + Offset::new(within.0, within.0),
            ]);
            let found = self
                .polygons_within(reach)
                .flat_map(|polygon| edges(&self.polygons[polygon]))
                .map(|(a, b)| closest_on_segment(a, b, at))
                .min_by(|a, b| a.distance(at).0.total_cmp(&b.distance(at).0));
            match found {
                Some(point) if point.distance(at) <= within => return Some(point),
                _ if reach.contains_rect(&self.bounds) => return found,
                _ => within = Meters(within.0 * 2.0),
            }
        }
    }

    /// The nearest point on the mesh that `fits`, searched along rings `step` apart around `at`, so
    /// a body of radius `step / 2` finds a place that fits between those already taken.
    pub fn nearest_where(
        &self,
        at: Pos<Meters>,
        step: Meters,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        if let Some(nearest) = self.nearest(at)
            && fits(nearest)
        {
            return Some(nearest);
        }
        let reach = self.bounds.size.to_vector().length();
        let mut ring = 1;
        loop {
            let radius = step.0 * ring as f32;
            if radius > reach {
                return None;
            }
            let count = ((std::f32::consts::TAU * radius / step.0).ceil() as usize).max(8);
            let found = (0..count)
                .map(|index| {
                    let angle = std::f32::consts::TAU * index as f32 / count as f32;
                    at + Offset::new(angle.cos() * radius, angle.sin() * radius)
                })
                .find(|&point| self.walkable(point) && fits(point));
            if found.is_some() {
                return found;
            }
            ring += 1;
        }
    }

    /// A point anywhere on the mesh that `fits`, each spot as likely as any other of the same size.
    pub fn random_point(
        &self,
        rng: &mut Rng,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        draw(
            rng,
            self.triangles.iter().map(|triangle| triangle.corners),
            fits,
        )
    }

    pub fn random_point_within(
        &self,
        rng: &mut Rng,
        bounds: Rect<Meters>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        draw(rng, self.clipped(&corners(bounds), |_| true), fits)
    }

    /// A point that `fits` on the island `from` stands on.
    pub fn random_reachable(
        &self,
        rng: &mut Rng,
        from: Pos<Meters>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let island = self.island(from)?;
        draw(
            rng,
            self.triangles
                .iter()
                .filter(|triangle| self.islands[triangle.polygon] == island)
                .map(|triangle| triangle.corners),
            fits,
        )
    }

    pub fn random_reachable_within(
        &self,
        rng: &mut Rng,
        from: Pos<Meters>,
        bounds: Rect<Meters>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let island = self.island(from)?;
        draw(
            rng,
            self.clipped(&corners(bounds), |polygon| self.islands[polygon] == island),
            fits,
        )
    }

    pub fn random_reachable_near(
        &self,
        rng: &mut Rng,
        from: Pos<Meters>,
        center: Pos<Meters>,
        radius: Meters,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let island = self.island(from)?;
        draw(
            rng,
            self.clipped(&circle_outline(center, radius), |polygon| {
                self.islands[polygon] == island
            }),
            fits,
        )
    }

    /// The polygon holding `at`, or the nearest within the fuzz: a point on a polygon's edge, as a
    /// route's corners are, lands a pixel or two off it through the window.
    fn locate(&self, at: Pos<Meters>) -> Option<usize> {
        self.place(at)
            .map(|coords| (coords.polygon() & POLYGON_BITS) as usize)
    }

    /// `at` placed on the mesh with the polygon that holds it, which a route is given outright: left
    /// to find the polygon itself, the search misses a point lying on a polygon's corner, and a
    /// walk to a corner the mesh holds would never be found.
    fn place(&self, at: Pos<Meters>) -> Option<Coords> {
        if !self.bounds.contains(at) {
            return None;
        }
        let at = vec2(at);
        self.mesh
            .get_point_layer(at)
            .first()
            .copied()
            .or_else(|| self.mesh.get_closest_point(at))
    }

    /// The triangles of the polygons `kept`, cut down to the convex `clip`.
    fn clipped(&self, clip: &[Pos<Meters>], kept: impl Fn(usize) -> bool) -> Vec<[Pos<Meters>; 3]> {
        let reach = Rect::from_points(clip);
        self.index
            .within(reach)
            .map(|index| &self.triangles[index])
            .filter(|triangle| triangle.bounds.intersects(&reach) && kept(triangle.polygon))
            .flat_map(|triangle| {
                let piece = clip_polygon(&triangle.corners, clip);
                fan(&piece).collect::<Vec<_>>()
            })
            .collect()
    }
}

struct Triangle {
    corners: [Pos<Meters>; 3],
    bounds: Rect<Meters>,
    polygon: usize,
}

/// How far off a point may lie and still be found on the mesh, so a walk ending on a polygon's edge
/// stands on it.
/// How far off the mesh a point may lie and still count as on it, and be routed from or to, searched
/// along rings out to there.
const LOCATE_FUZZ: f32 = 0.05;
const LOCATE_RINGS: u32 = 3;
const POLYGON_BITS: u32 = 0x00FF_FFFF;
const AREA_EPSILON: f32 = 1e-6;
const DRAWS: usize = 32;
const TRIANGLE_CELL: Meters = Meters(16.0);
const CIRCLE_SIDES: usize = 24;
/// The ground is triangulated in tiles no wider than this, each on a core of its own: growing an
/// outline and triangulating cost more per outline the more there are, and a wide area holds
/// outlines by the hundred thousand, while a tile this size holds a few hundred, enough work to be
/// worth a core.
const TILE_SIDE: Meters = Meters(200.0);
/// How far off a seam a vertex may land through the arithmetic and still be the seam's: both tiles
/// cut the same outline at the same seam, so their vertices there differ by the rounding alone,
/// far under a millimetre.
const SEAM_SNAP: f32 = 1e-3;

/// `ground` less the obstacles grown by `radius`, as one layer of convex polygons: triangulated
/// tile by tile, in parallel, each tile given every obstacle reaching into it, then joined along
/// the seams, where both tiles hold the same vertices.
fn triangulate(ground: Rect<Meters>, obstacles: &[Vec<Pos<Meters>>], radius: Meters) -> Layer {
    if ground.is_empty() {
        return Layer::default();
    }
    let columns = (ground.size.width / TILE_SIDE.0).ceil().max(1.0) as usize;
    let rows = (ground.size.height / TILE_SIDE.0).ceil().max(1.0) as usize;
    let tile: Size<Meters> = Size::new(
        ground.size.width / columns as f32,
        ground.size.height / rows as f32,
    );
    let seam_x = |column: usize| {
        if column == columns {
            ground.max().x
        } else {
            ground.min().x + tile.width * column as f32
        }
    };
    let seam_y = |row: usize| {
        if row == rows {
            ground.max().y
        } else {
            ground.min().y + tile.height * row as f32
        }
    };
    let reaches: Vec<Rect<Meters>> = obstacles
        .iter()
        .map(|outline| Rect::from_points(outline).inflate(radius.0, radius.0))
        .collect();
    let tiles: Vec<Layer> = (0..rows * columns)
        .into_par_iter()
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            let rect = Rect::from_points([
                Pos::new(seam_x(column), seam_y(row)),
                Pos::new(seam_x(column + 1), seam_y(row + 1)),
            ]);
            let mut triangulation = Triangulation::from_outer_edges(&corners(rect).map(vec2));
            triangulation.set_agent_radius(radius.0);
            triangulation.set_agent_radius_simplification(radius.0 * radius.0 / 16.0);
            triangulation.agent_radius_on_outer_edge(false);
            triangulation.add_obstacles(
                obstacles
                    .iter()
                    .zip(&reaches)
                    .filter(|(_, reach)| reach.intersects(&rect))
                    .map(|(outline, _)| outline.iter().copied().map(vec2).collect::<Vec<_>>()),
            );
            triangulation.as_layer()
        })
        .collect();
    let seams_x: Vec<f32> = (1..columns).map(seam_x).collect();
    let seams_y: Vec<f32> = (1..rows).map(seam_y).collect();
    joined(tiles, &seams_x, &seams_y)
}

/// The tiles as one layer: every vertex on a seam snapped onto it and shared by the tiles meeting
/// there, so the polygons on both sides neighbour each other through it.
fn joined(tiles: Vec<Layer>, seams_x: &[f32], seams_y: &[f32]) -> Layer {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut polygons: Vec<Polygon> = Vec::new();
    for layer in tiles {
        let (first_vertex, first_polygon) = (vertices.len() as u32, polygons.len() as u32);
        polygons.extend(layer.polygons.into_iter().map(|polygon| {
            Polygon::new(
                polygon.vertices.iter().map(|v| v + first_vertex).collect(),
                polygon.is_one_way,
            )
        }));
        vertices.extend(layer.vertices.into_iter().map(|vertex| {
            let mut coords = vertex.coords;
            if let Some(seam) = seams_x.iter().find(|x| (coords.x - *x).abs() <= SEAM_SNAP) {
                coords.x = *seam;
            }
            if let Some(seam) = seams_y.iter().find(|y| (coords.y - *y).abs() <= SEAM_SNAP) {
                coords.y = *seam;
            }
            Vertex::new(
                coords,
                vertex
                    .polygons
                    .iter()
                    .map(|p| {
                        if *p == u32::MAX {
                            *p
                        } else {
                            p + first_polygon
                        }
                    })
                    .collect(),
            )
        }));
    }
    let mut parent: Vec<usize> = (0..vertices.len()).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let same_along = |seams: &[f32], across: fn(&Vertex) -> f32, along: fn(&Vertex) -> f32| {
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for seam in seams {
            let mut on_it: Vec<usize> = (0..vertices.len())
                .filter(|&index| across(&vertices[index]) == *seam)
                .collect();
            on_it.sort_by(|a, b| along(&vertices[*a]).total_cmp(&along(&vertices[*b])));
            pairs.extend(
                on_it
                    .windows(2)
                    .filter(|pair| {
                        along(&vertices[pair[1]]) - along(&vertices[pair[0]]) <= SEAM_SNAP
                    })
                    .map(|pair| (pair[0], pair[1])),
            );
        }
        pairs
    };
    let mut pairs = same_along(seams_x, |v| v.coords.x, |v| v.coords.y);
    pairs.extend(same_along(seams_y, |v| v.coords.y, |v| v.coords.x));
    for (a, b) in pairs {
        let (a, b) = (root(&mut parent, a), root(&mut parent, b));
        if a != b {
            parent[a] = b;
        }
    }
    let mut renumbered: Vec<Option<u32>> = vec![None; vertices.len()];
    let mut shared: Vec<Vertex> = Vec::new();
    for (index, vertex) in vertices.into_iter().enumerate() {
        let root = root(&mut parent, index);
        match renumbered[root] {
            Some(kept) => shared[kept as usize]
                .polygons
                .extend(vertex.polygons.into_iter().filter(|p| *p != u32::MAX)),
            None => {
                renumbered[root] = Some(shared.len() as u32);
                shared.push(vertex);
            }
        }
    }
    for polygon in &mut polygons {
        for vertex in &mut polygon.vertices {
            *vertex = renumbered[root(&mut parent, *vertex as usize)].expect("a renumbered vertex");
        }
    }
    let mut layer = Layer::default();
    layer.vertices = shared;
    layer.polygons = polygons;
    layer
}

fn vec2(at: Pos<Meters>) -> Vec2 {
    Vec2::new(at.x, at.y)
}

fn pos(at: Vec2) -> Pos<Meters> {
    Pos::new(at.x, at.y)
}

/// Each polygon's island, polygons sharing a vertex sharing an island.
fn islands(layer: &polyanya::Layer) -> Vec<Island> {
    let count = layer.polygons.len();
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    for vertex in &layer.vertices {
        let mut shared = vertex.polygons.iter().filter(|&&p| p != u32::MAX);
        if let Some(&first) = shared.next() {
            for &other in shared {
                let (a, b) = (
                    root(&mut parent, first as usize),
                    root(&mut parent, other as usize),
                );
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut numbered: Vec<Option<usize>> = vec![None; count];
    let mut next = 0;
    (0..count)
        .map(|index| {
            let root = root(&mut parent, index);
            let island = *numbered[root].get_or_insert_with(|| {
                next += 1;
                next - 1
            });
            Island(island)
        })
        .collect()
}

/// A convex outline as triangles fanned from its first corner.
fn fan(outline: &[Pos<Meters>]) -> impl Iterator<Item = [Pos<Meters>; 3]> + '_ {
    let first = outline.first().copied();
    outline
        .windows(2)
        .skip(1)
        .filter_map(move |pair| Some([first?, pair[0], pair[1]]))
}

fn edges(outline: &[Pos<Meters>]) -> impl Iterator<Item = (Pos<Meters>, Pos<Meters>)> + '_ {
    outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(&a, &b)| (a, b))
}

fn area(outline: &[Pos<Meters>]) -> f32 {
    edges(outline)
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        .abs()
        / 2.0
}

fn centroid(outline: &[Pos<Meters>]) -> Pos<Meters> {
    let sum = outline
        .iter()
        .fold(Offset::zero(), |sum, point| sum + point.to_vector());
    Pos::origin() + sum / outline.len().max(1) as f32
}

fn closest_on_segment(a: Pos<Meters>, b: Pos<Meters>, at: Pos<Meters>) -> Pos<Meters> {
    let along = b - a;
    let length2 = along.square_length();
    if length2 < f32::EPSILON {
        return a;
    }
    let t = ((at - a).dot(along) / length2).clamp(0.0, 1.0);
    a + along * t
}

fn circle_outline(center: Pos<Meters>, radius: Meters) -> Vec<Pos<Meters>> {
    (0..CIRCLE_SIDES)
        .map(|side| {
            let angle = std::f32::consts::TAU * side as f32 / CIRCLE_SIDES as f32;
            center + Offset::new(angle.cos() * radius.0, angle.sin() * radius.0)
        })
        .collect()
}

/// `subject` cut down to the convex `clip`, by Sutherland–Hodgman.
fn clip_polygon(subject: &[Pos<Meters>], clip: &[Pos<Meters>]) -> Vec<Pos<Meters>> {
    let orientation = edges(clip)
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        .signum();
    let inside = |a: Pos<Meters>, b: Pos<Meters>, p: Pos<Meters>| {
        ((b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)) * orientation >= 0.0
    };
    let mut output = subject.to_vec();
    for (a, b) in edges(clip) {
        let input = std::mem::take(&mut output);
        if input.is_empty() {
            break;
        }
        for (previous, current) in edges(&input) {
            let (current_in, previous_in) = (inside(a, b, current), inside(a, b, previous));
            if current_in {
                if !previous_in {
                    output.push(intersection(previous, current, a, b));
                }
                output.push(current);
            } else if previous_in {
                output.push(intersection(previous, current, a, b));
            }
        }
    }
    output
}

fn intersection(p: Pos<Meters>, q: Pos<Meters>, a: Pos<Meters>, b: Pos<Meters>) -> Pos<Meters> {
    let (d1, d2) = (q - p, b - a);
    let cross = d1.x * d2.y - d1.y * d2.x;
    if cross.abs() < f32::EPSILON {
        return p;
    }
    let t = ((a.x - p.x) * d2.y - (a.y - p.y) * d2.x) / cross;
    p + d1 * t
}

/// A point that `fits` in one of the triangles, each spot as likely as any other of the same size;
/// none when the draws keep missing or there is no ground to draw from.
fn draw(
    rng: &mut Rng,
    triangles: impl IntoIterator<Item = [Pos<Meters>; 3]>,
    fits: impl Fn(Pos<Meters>) -> bool,
) -> Option<Pos<Meters>> {
    let mut total = 0.0;
    let weighted: Vec<([Pos<Meters>; 3], f32)> = triangles
        .into_iter()
        .map(|corners| (corners, area(&corners)))
        .filter(|(_, area)| *area > AREA_EPSILON)
        .map(|(corners, area)| {
            total += area;
            (corners, total)
        })
        .collect();
    if weighted.is_empty() {
        return None;
    }
    for _ in 0..DRAWS {
        let target = rng.rand_float() * total;
        let index = weighted
            .partition_point(|(_, cumulative)| *cumulative < target)
            .min(weighted.len() - 1);
        let [a, b, c] = weighted[index].0;
        let (r1, r2) = (rng.rand_float().sqrt(), rng.rand_float());
        let point = a + (b - a) * (r1 * (1.0 - r2)) + (c - a) * (r1 * r2);
        if fits(point) {
            return Some(point);
        }
    }
    None
}
