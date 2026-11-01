use std::ops::Range;

use crate::core::geometry::corners;
use crate::core::math::{Offset, Pos, Rect, Rng};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterPos, Meters};
use glam::Vec2;
use polyanya::{Coords, Layer, Mesh, Polygon, Triangulation, Vertex};

/// Where an area walks: its bounds less everything that blocks, each grown by the walker's radius,
/// cut into convex polygons a straight line crosses freely and strung together where they share an
/// edge. A point is walkable when a polygon holds it; two points connect when their polygons do.
pub struct NavMesh {
    bounds: Rect<Meters>,
    mesh: Mesh,
    /// Every polygon's corners one after another, each going round counter-clockwise: polygon `i`
    /// holds those from `starts[i]` up to `starts[i + 1]`.
    corners: Vec<Pos<Meters>>,
    starts: Vec<u32>,
    islands: Vec<Island>,
    /// The polygons by the ground they cover, so a clip or a search passes over only those near it.
    index: SpatialIndex,
    by_area: ByArea,
}

/// One tile of ground triangulated on its own, to join with its neighbours into a mesh.
pub struct TileLayer {
    rect: Rect<Meters>,
    layer: Layer,
}

/// The polygons, each with the ground of every one up to it, so a draw by area halves its way to
/// a polygon rather than weighing them all.
struct ByArea {
    polygons: Vec<u32>,
    reach: Vec<f64>,
}

impl ByArea {
    fn of<'a>(count: usize, outline: impl Fn(usize) -> &'a [Pos<Meters>]) -> ByArea {
        let polygons: Vec<u32> = (0..count as u32).collect();
        let mut total = 0.0;
        let reach: Vec<f64> = polygons
            .iter()
            .map(|&polygon| {
                total += f64::from(area(outline(polygon as usize)));
                total
            })
            .collect();
        ByArea { polygons, reach }
    }
}

/// A stretch of ground connected within itself and cut off from the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Island(usize);

impl NavMesh {
    /// The ground of `tile`, within `bounds` shrunk by `radius`, less every obstacle reaching into
    /// it grown by `radius`, triangulated on its own: a point on the layer is a place a body of
    /// that radius fits, give or take a tenth of the radius. A grown outline keeps only the points
    /// that each add more ground than a sixteenth of the radius squared, which lets a body a tenth
    /// of its radius in at most between the points kept round an outline its own size, and spares
    /// the mesh most of the points a rounded outline or a run of blocked cells brings.
    pub fn triangulate(
        tile: Rect<Meters>,
        bounds: Rect<Meters>,
        obstacles: &[Vec<Pos<Meters>>],
        radius: Meters,
    ) -> TileLayer {
        let ground = bounds.inflate(-radius.0, -radius.0);
        let Some(rect) = tile.intersection(&ground).filter(|rect| !rect.is_empty()) else {
            return TileLayer {
                rect: Rect::zero(),
                layer: Layer::default(),
            };
        };
        let mut triangulation = Triangulation::from_outer_edges(&corners(rect).map(vec2));
        triangulation.set_agent_radius(radius.0);
        triangulation.set_agent_radius_simplification(radius.0 * radius.0 / 16.0);
        triangulation.agent_radius_on_outer_edge(false);
        triangulation.add_obstacles(
            obstacles
                .iter()
                .filter(|outline| outline.len() >= 3)
                .filter(|outline| {
                    Rect::from_points(outline.iter().copied())
                        .inflate(radius.0, radius.0)
                        .intersects(&rect)
                })
                .map(|outline| outline.iter().copied().map(vec2).collect::<Vec<_>>()),
        );
        TileLayer {
            rect,
            layer: triangulation.as_layer(),
        }
    }

    /// The mesh the tiles make, joined along the seams where they meet: a polygon's neighbours
    /// across a seam are the other tile's, and a seam no tile lies beyond is the mesh's edge.
    pub fn join<'a>(
        bounds: Rect<Meters>,
        tiles: impl IntoIterator<Item = &'a TileLayer>,
    ) -> NavMesh {
        let tiles: Vec<&TileLayer> = tiles.into_iter().collect();
        let seams = |along: fn(&Rect<Meters>) -> [f32; 2]| {
            let mut seams: Vec<f32> = tiles
                .iter()
                .filter(|tile| !tile.rect.is_empty())
                .flat_map(|tile| along(&tile.rect))
                .collect();
            seams.sort_by(f32::total_cmp);
            seams.dedup();
            seams
        };
        let seams_x = seams(|rect| [rect.min_x(), rect.max_x()]);
        let seams_y = seams(|rect| [rect.min_y(), rect.max_y()]);
        let mut mesh = Mesh {
            layers: vec![joined(
                tiles.iter().map(|tile| &tile.layer),
                &seams_x,
                &seams_y,
            )],
            ..Default::default()
        };
        mesh.reorder_neighbors_ccw_and_fix_corners();
        mesh.update_is_one_way();
        mesh.merge_polygons();
        fit(&mut mesh.layers[0]);
        mesh.search_delta = LOCATE_FUZZ / (LOCATE_RINGS - 1) as f32;
        mesh.search_steps = LOCATE_RINGS;
        mesh.bake();
        let layer = &mesh.layers[0];
        let mut corners: Vec<Pos<Meters>> = Vec::new();
        let mut starts: Vec<u32> = Vec::with_capacity(layer.polygons.len() + 1);
        for polygon in &layer.polygons {
            starts.push(corners.len() as u32);
            corners.extend(
                polygon
                    .vertices
                    .iter()
                    .map(|&vertex| pos(layer.vertices[vertex as usize].coords)),
            );
        }
        starts.push(corners.len() as u32);
        let islands = islands(layer);
        let outline =
            |polygon: usize| &corners[starts[polygon] as usize..starts[polygon + 1] as usize];
        let count = starts.len() - 1;
        let mut index = SpatialIndex::new(POLYGON_CELL);
        index.refile((0..count).map(|polygon| Rect::from_points(outline(polygon))));
        let by_area = ByArea::of(islands.len(), outline);
        NavMesh {
            bounds,
            mesh,
            corners,
            starts,
            islands,
            index,
            by_area,
        }
    }

    pub fn bounds(&self) -> Rect<Meters> {
        self.bounds
    }

    pub fn is_empty(&self) -> bool {
        self.polygon_count() == 0
    }

    /// The convex polygons the mesh is made of, each going round counter-clockwise.
    pub fn polygons(&self) -> impl Iterator<Item = &[Pos<Meters>]> {
        (0..self.polygon_count()).map(|polygon| self.outline(polygon))
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
            let piece = clip_polygon(self.outline(polygon), &clip);
            (area(&piece) > AREA_EPSILON).then_some(piece)
        })
    }

    /// The polygons that may reach into `reach`, each once.
    fn polygons_within(&self, reach: Rect<Meters>) -> impl Iterator<Item = usize> + '_ {
        self.index.within(reach)
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
        let mut within = POLYGON_CELL;
        loop {
            let reach = Rect::from_points([
                at - Offset::new(within.0, within.0),
                at + Offset::new(within.0, within.0),
            ]);
            let found = self
                .polygons_within(reach)
                .flat_map(|polygon| edges(self.outline(polygon)))
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
        self.draw_by_area(rng, 0..self.by_area.polygons.len(), fits)
    }

    pub fn random_point_within(
        &self,
        rng: &mut Rng,
        bounds: Rect<Meters>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        draw(rng, self.clipped(&corners(bounds), |_| true), fits)
    }

    /// The ground within `radius` of `center` an actor at `from` can reach, as triangles to draw
    /// a spot from.
    pub fn reachable_near(
        &self,
        from: Pos<Meters>,
        center: Pos<Meters>,
        radius: Meters,
    ) -> Vec<[Pos<Meters>; 3]> {
        let Some(island) = self.island(from) else {
            return Vec::new();
        };
        self.clipped(&circle_outline(center, radius), |polygon| {
            self.islands[polygon] == island
        })
    }

    /// The ground within `bounds` an actor at `from` can reach, as triangles to draw a spot from.
    pub fn reachable_within(
        &self,
        from: Pos<Meters>,
        bounds: Rect<Meters>,
    ) -> Vec<[Pos<Meters>; 3]> {
        let Some(island) = self.island(from) else {
            return Vec::new();
        };
        self.clipped(&corners(bounds), |polygon| self.islands[polygon] == island)
    }

    /// A spot that `fits` on `ground`, each as likely as any other of the same size.
    pub fn draw_on(
        rng: &mut Rng,
        ground: impl IntoIterator<Item = [Pos<Meters>; 3]>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        draw(rng, ground, fits)
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

    /// The polygons `kept`, cut down to the convex `clip`, as triangles.
    fn clipped(&self, clip: &[Pos<Meters>], kept: impl Fn(usize) -> bool) -> Vec<[Pos<Meters>; 3]> {
        let reach = Rect::from_points(clip);
        self.polygons_within(reach)
            .filter(|&polygon| kept(polygon))
            .flat_map(|polygon| {
                let piece = clip_polygon(self.outline(polygon), clip);
                fan(&piece).collect::<Vec<_>>()
            })
            .collect()
    }

    fn polygon_count(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    fn outline(&self, polygon: usize) -> &[Pos<Meters>] {
        &self.corners[self.starts[polygon] as usize..self.starts[polygon + 1] as usize]
    }

    /// A point that `fits` on the polygons at `range` of those by area, each spot as likely as any
    /// other of the same size.
    fn draw_by_area(
        &self,
        rng: &mut Rng,
        range: Range<usize>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let by_area = &self.by_area;
        let (start, end) = (range.start, range.end);
        if start >= end {
            return None;
        }
        let floor = if start == 0 {
            0.0
        } else {
            by_area.reach[start - 1]
        };
        let ceiling = by_area.reach[end - 1];
        if ceiling - floor <= f64::from(AREA_EPSILON) {
            return None;
        }
        for _ in 0..DRAWS {
            let target = floor + f64::from(rng.rand_float()) * (ceiling - floor);
            let index = (start
                + by_area.reach[start..end].partition_point(|&reach| reach < target))
            .min(end - 1);
            let polygon = by_area.polygons[index] as usize;
            if let Some(point) = point_in_outline(rng, self.outline(polygon))
                && fits(point)
            {
                return Some(point);
            }
        }
        None
    }
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
const POLYGON_CELL: Meters = Meters(16.0);
const CIRCLE_SIDES: usize = 24;
/// How far off a seam a vertex may land through the arithmetic and still be the seam's: both tiles
/// cut the same outline at the same seam, so their vertices there differ by the rounding alone,
/// far under a millimetre.
const SEAM_SNAP: f32 = 1e-3;

/// The layer's lists trimmed to what they hold: built by pushing, they keep room for as much again.
fn fit(layer: &mut Layer) {
    for vertex in &mut layer.vertices {
        vertex.polygons.shrink_to_fit();
    }
    for polygon in &mut layer.polygons {
        polygon.vertices.shrink_to_fit();
    }
    layer.vertices.shrink_to_fit();
    layer.polygons.shrink_to_fit();
}

/// The tiles as one layer: every vertex on a seam snapped onto it and shared by the tiles meeting
/// there, so the polygons on both sides neighbour each other through it.
fn joined<'a>(tiles: impl Iterator<Item = &'a Layer>, seams_x: &[f32], seams_y: &[f32]) -> Layer {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut polygons: Vec<Polygon> = Vec::new();
    for layer in tiles {
        let (first_vertex, first_polygon) = (vertices.len() as u32, polygons.len() as u32);
        polygons.extend(layer.polygons.iter().map(|polygon| {
            Polygon::new(
                polygon.vertices.iter().map(|v| v + first_vertex).collect(),
                polygon.is_one_way,
            )
        }));
        vertices.extend(layer.vertices.iter().map(|vertex| {
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
        let point = point_in(rng, weighted[index].0);
        if fits(point) {
            return Some(point);
        }
    }
    None
}

/// A point in the convex outline, each spot as likely as any other of the same size.
fn point_in_outline(rng: &mut Rng, outline: &[Pos<Meters>]) -> Option<Pos<Meters>> {
    let total = area(outline);
    let target = rng.rand_float() * total;
    let mut covered = 0.0;
    let mut last = None;
    for corners in fan(outline) {
        last = Some(corners);
        covered += area(&corners);
        if covered >= target {
            break;
        }
    }
    last.map(|corners| point_in(rng, corners))
}

fn point_in(rng: &mut Rng, [a, b, c]: [Pos<Meters>; 3]) -> Pos<Meters> {
    let (r1, r2) = (rng.rand_float().sqrt(), rng.rand_float());
    a + (b - a) * (r1 * (1.0 - r2)) + (c - a) * (r1 * r2)
}
