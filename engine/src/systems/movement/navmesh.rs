use crate::core::geometry::corners;
use crate::core::math::{Offset, Pos, Rect, Rng};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterPos, Meters};
use geo::TriangulateEarcut;
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
    reach: Vec<f64>,
}

impl ByArea {
    fn of<'a>(count: usize, outline: impl Fn(usize) -> &'a [Pos<Meters>]) -> ByArea {
        let mut total = 0.0;
        let reach: Vec<f64> = (0..count)
            .map(|polygon| {
                total += f64::from(area(outline(polygon)));
                total
            })
            .collect();
        ByArea { reach }
    }
}

/// A stretch of ground connected within itself and cut off from the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Island(usize);

impl Island {
    /// The island's number among its mesh's, counted from 0.
    pub fn index(self) -> usize {
        self.0
    }
}

impl TileLayer {
    /// The ground the layer covers: its tile, within the area's walkable bounds.
    pub fn rect(&self) -> Rect<Meters> {
        self.rect
    }
}

impl NavMesh {
    /// The ground of `tile`, within `bounds` shrunk by `radius`, less every obstacle reaching into
    /// it grown by `radius`, triangulated on its own: a point on the layer is a place a body of
    /// that radius fits, kept off an obstacle by at most a tenth of the radius more.
    pub fn triangulate(
        tile: Rect<Meters>,
        bounds: Rect<Meters>,
        obstacles: &[Vec<Pos<Meters>>],
        radius: Meters,
    ) -> TileLayer {
        let [layer] = NavMesh::triangulate_sharing(tile, bounds, radius, obstacles, [&[]]);
        layer
    }

    /// Layers of `tile` as [`NavMesh::triangulate`] lays them, each less the `shared` obstacles
    /// and its own besides: the shared ones are grown once for them all.
    pub fn triangulate_sharing<const LAYERS: usize>(
        tile: Rect<Meters>,
        bounds: Rect<Meters>,
        radius: Meters,
        shared: &[Vec<Pos<Meters>>],
        own: [&[Vec<Pos<Meters>>]; LAYERS],
    ) -> [TileLayer; LAYERS] {
        let ground = bounds.inflate(-radius.0, -radius.0);
        let Some(rect) = tile.intersection(&ground).filter(|rect| !rect.is_empty()) else {
            return std::array::from_fn(|_| TileLayer {
                rect: Rect::zero(),
                layer: Layer::default(),
            });
        };
        let mut triangulation = Triangulation::from_outer_edges(&corners(rect).map(vec2));
        triangulation.add_obstacles(reaching(shared, rect, radius));
        triangulation.prebuild();
        own.map(|obstacles| {
            let mut triangulation = triangulation.clone();
            triangulation.add_obstacles(reaching(obstacles, rect, radius));
            TileLayer {
                rect,
                layer: triangulation.as_layer(),
            }
        })
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
        self.draw_by_area(rng, fits)
    }

    /// A point within `bounds` that `fits`, each spot as likely as any other of the same size:
    /// drawn over the bounds and kept where the mesh walks, which costs next to nothing where the
    /// mesh covers much of them, else drawn over the mesh's ground cut to them.
    pub fn random_point_within(
        &self,
        rng: &mut Rng,
        bounds: Rect<Meters>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        (0..DRAWS)
            .map(|_| {
                bounds.origin
                    + Offset::new(
                        rng.rand_float() * bounds.width(),
                        rng.rand_float() * bounds.height(),
                    )
            })
            .find(|&at| self.walkable(at) && fits(at))
            .or_else(|| Self::draw_on(rng, self.clipped(&corners(bounds), |_| true), fits))
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

    /// A spot that `fits` on `ground`, each as likely as any other of the same size; none when the
    /// draws keep missing or there is no ground to draw from.
    pub fn draw_on(
        rng: &mut Rng,
        ground: impl IntoIterator<Item = [Pos<Meters>; 3]>,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let mut total = 0.0;
        let weighted: Vec<([Pos<Meters>; 3], f32)> = ground
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

    /// How many islands the mesh holds.
    pub fn island_count(&self) -> usize {
        self.islands
            .iter()
            .map(|island| island.0 + 1)
            .max()
            .unwrap_or(0)
    }

    /// Every corner of every polygon, with the island the polygon lies on.
    pub fn corners_by_island(&self) -> impl Iterator<Item = (Pos<Meters>, Island)> + '_ {
        (0..self.polygon_count()).flat_map(move |polygon| {
            self.outline(polygon)
                .iter()
                .map(move |corner| (*corner, self.islands[polygon]))
        })
    }

    fn polygon_count(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    fn outline(&self, polygon: usize) -> &[Pos<Meters>] {
        &self.corners[self.starts[polygon] as usize..self.starts[polygon + 1] as usize]
    }

    /// A point that `fits` on the mesh, each spot as likely as any other of the same size.
    fn draw_by_area(
        &self,
        rng: &mut Rng,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        let reach = &self.by_area.reach;
        let ground = *reach.last()?;
        if ground <= f64::from(AREA_EPSILON) {
            return None;
        }
        for _ in 0..DRAWS {
            let target = f64::from(rng.rand_float()) * ground;
            let polygon = reach
                .partition_point(|&reach| reach < target)
                .min(reach.len() - 1);
            if let Some(point) = point_in_outline(rng, self.outline(polygon))
                && fits(point)
            {
                return Some(point);
            }
        }
        None
    }
}

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
pub const SEAM_SNAP: f32 = 1e-3;
/// How far past the walker's radius a grown obstacle may reach, as a share of the radius.
const OVERREACH: f32 = 0.1;

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

/// The ground walkers `radius` round keep clear of around an obstacle of `outline`, as the outlines
/// that together hold it.
pub fn kept_clear(outline: &[Pos<Meters>], radius: Meters) -> Vec<Vec<Pos<Meters>>> {
    grown(outline, radius)
        .into_iter()
        .map(|grown| grown.into_iter().map(|at| Pos::new(at.x, at.y)).collect())
        .collect()
}

/// The outlines of `obstacles` that reach into `rect` once grown by `radius`, grown.
fn reaching<'a>(
    obstacles: &'a [Vec<Pos<Meters>>],
    rect: Rect<Meters>,
    radius: Meters,
) -> impl Iterator<Item = Vec<Vec2>> + 'a {
    obstacles
        .iter()
        .filter(|outline| outline.len() >= 3)
        .filter(move |outline| {
            Rect::from_points(outline.iter().copied())
                .inflate(radius.0, radius.0)
                .intersects(&rect)
        })
        .flat_map(move |outline| grown(outline, radius))
}

/// `outline` grown by `radius` all round, as outlines whose ground together holds every point
/// within `radius` of it and reaches at most a tenth of `radius` further. A convex outline grows
/// into one: its edges pushed out, its corners rounded by edges touching the rounding. Any other
/// grows as the triangles it splits into, the ground of their growths overlapping as theirs does.
fn grown(outline: &[Pos<Meters>], radius: Meters) -> Vec<Vec<Vec2>> {
    let mut points: Vec<Vec2> = outline.iter().copied().map(vec2).collect();
    points.dedup_by(|a, b| a.distance_squared(*b) < AREA_EPSILON);
    if points.len() > 1 && points[0].distance_squared(points[points.len() - 1]) < AREA_EPSILON {
        points.pop();
    }
    if points.len() < 3 {
        return Vec::new();
    }
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    if is_convex(&points) {
        return vec![grown_convex(&points, radius.0)];
    }
    let polygon = geo::Polygon::new(
        points.iter().map(|point| (point.x, point.y)).collect(),
        Vec::new(),
    );
    polygon
        .earcut_triangles_iter()
        .filter_map(|triangle| {
            let mut corners = triangle
                .to_array()
                .map(|corner| Vec2::new(corner.x, corner.y))
                .to_vec();
            let area = signed_area(&corners);
            if area.abs() < AREA_EPSILON {
                return None;
            }
            if area < 0.0 {
                corners.reverse();
            }
            Some(grown_convex(&corners, radius.0))
        })
        .collect()
}

/// The convex, counter-clockwise `corners` grown by `radius`: every edge of the growth lies on a
/// line touching the rounded outline `radius` out, and every corner of it lies at most a tenth of
/// `radius` further, with as few corners as that allows.
fn grown_convex(corners: &[Vec2], radius: f32) -> Vec<Vec2> {
    let farthest = radius * (1.0 + OVERREACH);
    let widest_step = 2.0 * (1.0 / (1.0 + OVERREACH)).acos();
    let outward = |from: Vec2, to: Vec2| {
        let along = (to - from).normalize_or_zero();
        Vec2::new(along.y, -along.x)
    };
    let count = corners.len();
    let mut touching: Vec<Line> = Vec::with_capacity(count * 2);
    for at in 0..count {
        let before = corners[(at + count - 1) % count];
        let corner = corners[at];
        let after = corners[(at + 1) % count];
        let (from, to) = (outward(before, corner), outward(corner, after));
        let turn = from.perp_dot(to).atan2(from.dot(to)).max(0.0);
        let steps = (turn / widest_step).floor() as usize;
        let start = from.y.atan2(from.x);
        touching.extend((1..=steps).map(|index| {
            let angle = start + turn * index as f32 / (steps + 1) as f32;
            let normal = Vec2::new(angle.cos(), angle.sin());
            Line {
                through: corner + normal * radius,
                normal,
            }
        }));
        touching.push(Line {
            through: corner + to * radius,
            normal: to,
        });
    }
    let reaches =
        |point: Option<Vec2>| point.is_some_and(|point| beyond(corners, point) <= farthest);
    let mut at = 0;
    while touching.len() > 3 && at < touching.len() {
        let next = (at + 2) % touching.len();
        if reaches(touching[at].meeting(&touching[next])) {
            let dropped = (at + 1) % touching.len();
            touching.remove(dropped);
            if dropped < at {
                at -= 1;
            }
        } else {
            at += 1;
        }
    }
    (0..touching.len())
        .filter_map(|at| touching[at].meeting(&touching[(at + 1) % touching.len()]))
        .collect()
}

/// A straight line, by a point on it and the way it faces.
struct Line {
    through: Vec2,
    normal: Vec2,
}

impl Line {
    /// Where the line crosses `other`; none when they run alike or turn back on each other.
    fn meeting(&self, other: &Line) -> Option<Vec2> {
        let turn = self.normal.perp_dot(other.normal);
        if turn <= AREA_EPSILON {
            return None;
        }
        let along = self.normal.perp();
        let t = (other.through - self.through).dot(other.normal) / along.dot(other.normal);
        Some(self.through + along * t)
    }
}

/// How far `point` lies outside the convex, counter-clockwise `corners`.
fn beyond(corners: &[Vec2], point: Vec2) -> f32 {
    let count = corners.len();
    (0..count)
        .map(|at| {
            let (a, b) = (corners[at], corners[(at + 1) % count]);
            let along = b - a;
            let t = ((point - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(a + along * t)
        })
        .fold(f32::INFINITY, f32::min)
}

fn signed_area(points: &[Vec2]) -> f32 {
    (0..points.len())
        .map(|at| points[at].perp_dot(points[(at + 1) % points.len()]))
        .sum::<f32>()
        / 2.0
}

fn is_convex(points: &[Vec2]) -> bool {
    let count = points.len();
    (0..count).all(|at| {
        let (a, b, c) = (
            points[at],
            points[(at + 1) % count],
            points[(at + 2) % count],
        );
        (b - a).perp_dot(c - b) >= -AREA_EPSILON
    })
}
