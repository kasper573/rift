use super::ground::{GroundDef, Walk};
use super::prop::PropPlacement;
use super::terrain::Terrain;
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::geometry::corners;
use crate::core::math::{Pos, Rect};
use crate::core::terrain::{Heightfield, Paint};
use crate::core::units::{Cell, CellPos, GridDims, GridSize, Meters};
use crate::systems::movement::{AGENT_RADIUS, NavMesh};

/// Where an area walks and flies, for everyone and for the wild, and what it sounds like underfoot.
pub struct WalkMeshes {
    pub walk: NavMesh,
    pub wild: NavMesh,
    pub airspace: NavMesh,
    pub wild_airspace: NavMesh,
    pub footsteps: Vec<Option<SfxId>>,
}

/// The ground blocks where it is painted mostly with a material that blocks, where it is steeper
/// than the terrain allows, and under every prop's footprint; a safe zone keeps the wild out of it.
/// The air knows only the bounds and, for the wild, the safe zones.
pub fn meshes(
    content: &Content,
    terrain: &Terrain,
    heights: &Heightfield,
    paint: &Paint,
    props: &[PropPlacement],
    safe_zones: &[Rect<Meters>],
) -> WalkMeshes {
    let size = terrain.size.grid();
    let bounds = terrain.size.bounds();
    let materials: Vec<&GroundDef> = terrain
        .materials
        .iter()
        .map(|ground| ground.get(content))
        .collect();
    let cells: Vec<CellPos> = size.cells().collect();
    let dominant: Vec<usize> = cells.iter().map(|cell| paint.dominant(*cell)).collect();
    let blocked: Vec<bool> = cells
        .iter()
        .zip(&dominant)
        .map(|(cell, material)| {
            materials[*material].walk != Walk::Walkable
                || heights.steepest(*cell) > terrain.max_slope
        })
        .collect();
    let footprints = props.iter().filter_map(|placed| {
        let def = placed.prop.get(content);
        (!def.footprint.is_none())
            .then(|| def.footprint.outline(placed.at, placed.yaw, placed.scale))
    });
    let ground: Vec<Vec<Pos<Meters>>> = footprints
        .chain(
            patches(size, &blocked)
                .into_iter()
                .map(|patch| corners(patch).to_vec()),
        )
        .collect();
    let zones: Vec<Vec<Pos<Meters>>> = safe_zones
        .iter()
        .map(|zone| corners(*zone).to_vec())
        .collect();
    let footsteps = dominant
        .iter()
        .map(|material| Some(materials[*material].surface.get(content).footstep))
        .collect();
    WalkMeshes {
        walk: NavMesh::build(bounds, ground.iter().cloned(), AGENT_RADIUS),
        wild: NavMesh::build(bounds, ground.iter().chain(&zones).cloned(), AGENT_RADIUS),
        airspace: NavMesh::build(bounds, [], AGENT_RADIUS),
        wild_airspace: NavMesh::build(bounds, zones.iter().cloned(), AGENT_RADIUS),
        footsteps,
    }
}

/// The blocked cells as rectangles: each row's runs, and runs alike on consecutive rows joined.
fn patches(size: GridSize, blocked: &[bool]) -> Vec<Rect<Meters>> {
    let (width, height) = (size.width as usize, size.height as usize);
    let mut patches: Vec<Rect<Meters>> = Vec::new();
    for y in 0..height {
        let mut x = 0;
        while x < width {
            if !blocked[y * width + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < width && blocked[y * width + x] {
                x += 1;
            }
            let run = CellPos::new(start as i32, y as i32)
                .bounds()
                .union(&CellPos::new(x as i32 - 1, y as i32).bounds());
            match patches.iter_mut().find(|patch| {
                patch.min_x() == run.min_x()
                    && patch.max_x() == run.max_x()
                    && patch.max_y() == run.min_y()
            }) {
                Some(above) => *above = above.union(&run),
                None => patches.push(run),
            }
        }
    }
    patches
}
