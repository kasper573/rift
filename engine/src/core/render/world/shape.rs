//! The shape of a piece of a model at every level a view may draw it at, finest first: its own
//! triangles, what the simplifier brings them to, and where it is cut out of its texture, its
//! islands thinned once too small to tell apart; so a crowd of copies far off the eye draws no
//! more triangles than its pixels show.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::cover::PieceVertices;
use super::lod::{self, GRAIN_PIXELS, STRAY_PIXELS};

/// Geometry ready to draw at the level a view needs: its vertices, and the triangles it is drawn
/// with at every level it comes down to, finest first, its own at the head.
pub(super) struct Shape {
    pub(super) vertices: PieceVertices,
    pub(super) levels: Vec<PieceLevel>,
    /// How many of the levels the simplifier made, the shape's own first; any after them thin its
    /// islands.
    simplified: usize,
}

impl Shape {
    /// The shape of `vertices` at its own triangles, then coarser and coarser: what the simplifier
    /// brings them to, each with how far its shape strays, and its islands thinned on from there
    /// once they are too small to tell apart, as a shape cut out of its texture may be.
    pub(super) fn new(vertices: PieceVertices) -> Shape {
        let positions: Vec<[f32; 3]> = vertices
            .positions
            .iter()
            .map(|position| position.to_array())
            .collect();
        let normals: Vec<[f32; 3]> = vertices
            .normals
            .iter()
            .map(|normal| normal.to_array())
            .collect();
        let mut levels: Vec<PieceLevel> =
            std::iter::once(PieceLevel::of(&vertices, &vertices.indices, f32::INFINITY))
                .chain(
                    lod::simplify(
                        &vertices.indices,
                        &positions,
                        lod::Surface {
                            uvs: Some(&vertices.uvs),
                            normals: Some(&normals),
                        },
                    )
                    .iter()
                    .map(|(kept, strays)| PieceLevel::of(&vertices, kept, STRAY_PIXELS / strays)),
                )
                .collect();
        let simplified = levels.len();
        if let Some(coarsest) = levels.last() {
            let thinned = thinned(coarsest);
            levels.extend(thinned);
        }
        Shape {
            vertices,
            levels,
            simplified,
        }
    }

    pub(super) fn triangles(&self) -> usize {
        self.levels[0].triangles()
    }

    /// The level to draw the shape at, seen at `pixels_per_unit` over `covered` pixels: the
    /// coarsest that looks as the shape does there, and of those the simplifier made, coarser
    /// still while it takes more triangles than the pixels it covers, as no picture shows more of
    /// a shape than a triangle a pixel. Thinned islands stand in only where they look as the
    /// shape does.
    pub(super) fn level(&self, pixels_per_unit: f32, covered: f32) -> &PieceLevel {
        let by_look = self
            .levels
            .iter()
            .rposition(|level| pixels_per_unit <= level.shown_up_to)
            .unwrap_or(0);
        let simplified = &self.levels[..self.simplified];
        let by_pixels = simplified
            .iter()
            .position(|level| level.triangles() as f32 <= covered)
            .unwrap_or(simplified.len() - 1);
        &self.levels[by_look.max(by_pixels)]
    }
}

/// The most vertices a merged mesh holds: as many as two-byte indices reach, so a region's covers
/// go to the GPU as several small meshes, a few a frame, rather than one large one.
pub(super) const MOST_MERGED_VERTICES: usize = u16::MAX as usize;

/// A shape's triangles at a level: the vertices of the shape they use, where each lies, and the
/// triangles over them, with the most pixels a unit of the shape may span for the level to look
/// as the shape does.
pub(super) struct PieceLevel {
    shown_up_to: f32,
    pub(super) vertices: Vec<u32>,
    pub(super) positions: Vec<Vec3>,
    pub(super) indices: Vec<u32>,
}

impl PieceLevel {
    fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The level drawing the triangles `kept` index over the vertices, looking as the shape does
    /// up to `shown_up_to` pixels a unit.
    fn of(vertices: &PieceVertices, kept: &[u32], shown_up_to: f32) -> PieceLevel {
        PieceLevel::drawing(kept.iter().copied(), shown_up_to, |vertex| {
            (vertex, vertices.positions[vertex as usize])
        })
    }

    /// The level drawing the triangles `kept` index, each vertex standing for the piece's vertex
    /// and at the place `placed` gives it.
    fn drawing(
        kept: impl IntoIterator<Item = u32>,
        shown_up_to: f32,
        placed: impl Fn(u32) -> (u32, Vec3),
    ) -> PieceLevel {
        let mut level = PieceLevel {
            shown_up_to,
            vertices: Vec::new(),
            positions: Vec::new(),
            indices: Vec::new(),
        };
        let mut used: HashMap<u32, u32> = HashMap::default();
        for vertex in kept {
            let index = *used.entry(vertex).or_insert_with(|| {
                let (stands_for, at) = placed(vertex);
                level.vertices.push(stands_for);
                level.positions.push(at);
                level.vertices.len() as u32 - 1
            });
            level.indices.push(index);
        }
        level
    }
}

/// How many pixels a shape `across` pixels wide covers, taken round.
pub(super) fn covered_pixels(across: f32) -> f32 {
    std::f32::consts::FRAC_PI_4 * across * across
}

/// How far across `shapes` measure together.
pub(super) fn extent<'a>(shapes: impl Iterator<Item = &'a Shape>) -> f32 {
    let (low, high) = shapes
        .flat_map(|shape| shape.vertices.positions.iter())
        .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(low, high), &at| {
            (low.min(at), high.max(at))
        });
    (high - low).max_element().max(0.0)
}

/// A level's islands, triangles joined by the vertices they share, thinned to every second one,
/// then every fourth, while more than one is left: each island kept grown about its middle to make
/// up the area of those dropped, so what the islands cover stays as thick. A thinner level looks
/// as the shape does only while its grown islands span no more than [`GRAIN_PIXELS`], too few for
/// the view to tell them apart from the islands they stand in for.
fn thinned(level: &PieceLevel) -> Vec<PieceLevel> {
    let triangles = level.indices.as_chunks::<3>().0;
    let mut joined: Vec<u32> = (0..level.vertices.len() as u32).collect();
    let root = |joined: &mut Vec<u32>, mut at: u32| {
        while joined[at as usize] != at {
            joined[at as usize] = joined[joined[at as usize] as usize];
            at = joined[at as usize];
        }
        at
    };
    for triangle in triangles {
        let first = root(&mut joined, triangle[0]);
        for &vertex in &triangle[1..] {
            let other = root(&mut joined, vertex);
            joined[other as usize] = first;
        }
    }
    let mut islands: HashMap<u32, usize> = HashMap::default();
    let island_of_vertex: Vec<usize> = (0..level.vertices.len() as u32)
        .map(|vertex| {
            let found = root(&mut joined, vertex);
            let count = islands.len();
            *islands.entry(found).or_insert(count)
        })
        .collect();
    let mut middles = vec![(Vec3::ZERO, 0.0f32); islands.len()];
    let mut bounds = vec![(Vec3::INFINITY, Vec3::NEG_INFINITY); islands.len()];
    for (vertex, &island) in island_of_vertex.iter().enumerate() {
        let at = level.positions[vertex];
        middles[island].0 += at;
        middles[island].1 += 1.0;
        bounds[island] = (bounds[island].0.min(at), bounds[island].1.max(at));
    }
    let widest = bounds
        .iter()
        .map(|(low, high)| (*high - *low).max_element())
        .fold(0.0f32, f32::max);
    let mut levels = Vec::new();
    let mut every = 2;
    while islands.len() / every >= 1 {
        let grown = (every as f32).sqrt();
        let kept = triangles
            .iter()
            .filter(|triangle| island_of_vertex[triangle[0] as usize].is_multiple_of(every))
            .flat_map(|triangle| triangle.iter().copied());
        let shown_up_to = level
            .shown_up_to
            .min(GRAIN_PIXELS / (widest * grown).max(f32::EPSILON));
        levels.push(PieceLevel::drawing(kept, shown_up_to, |vertex| {
            let (sum, count) = middles[island_of_vertex[vertex as usize]];
            let middle = sum / count;
            let at = middle + (level.positions[vertex as usize] - middle) * grown;
            (level.vertices[vertex as usize], at)
        }));
        every *= 2;
    }
    levels
}
