use super::ground::{GroundDef, Walk};
use super::prop::{PropDef, PropPlacement};
use super::terrain::{Heightfield, Paint, Terrain};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::math::Rect;
use crate::core::units::{Cell, GridDims, Meters};

/// Where an area walks, flies, is safe and sounds underfoot, one answer per metre cell.
pub struct WalkGrids {
    pub walk: Vec<bool>,
    pub wild: Vec<bool>,
    pub airspace: Vec<bool>,
    pub wild_airspace: Vec<bool>,
    pub safe_cells: Vec<bool>,
    pub footsteps: Vec<Option<SfxId>>,
}

/// A cell blocks where its ground is painted mostly with a material that blocks, where the ground
/// is steeper than the terrain allows, or where a prop's footprint covers its centre; a safe zone
/// keeps the wild out of it.
pub fn grids(
    content: &Content,
    terrain: &Terrain,
    heights: &Heightfield,
    paint: &Paint,
    props: &[PropPlacement],
    safe_zones: &[Rect<Meters>],
) -> WalkGrids {
    let size = terrain.size.grid();
    let materials: Vec<&GroundDef> = terrain
        .materials
        .iter()
        .map(|ground| ground.get(content))
        .collect();
    let footprints: Vec<(&PropDef, &PropPlacement)> = props
        .iter()
        .map(|placed| (placed.prop.get(content), placed))
        .filter(|(def, _)| !def.footprint.is_none())
        .collect();
    let cells: Vec<_> = size.cells().collect();
    let dominant: Vec<usize> = cells.iter().map(|cell| paint.dominant(*cell)).collect();
    let walk: Vec<bool> = cells
        .iter()
        .zip(&dominant)
        .map(|(cell, material)| {
            let center = cell.center();
            materials[*material].walk == Walk::Walkable
                && heights.steepest(*cell) <= terrain.max_slope
                && !footprints.iter().any(|(def, placed)| {
                    def.footprint
                        .covers(placed.at, placed.yaw, placed.scale, center)
                })
        })
        .collect();
    let safe_cells: Vec<bool> = cells
        .iter()
        .map(|cell| safe_zones.iter().any(|zone| zone.contains(cell.center())))
        .collect();
    let wild = walk
        .iter()
        .zip(&safe_cells)
        .map(|(&walkable, &safe)| walkable && !safe)
        .collect();
    let airspace = vec![true; walk.len()];
    let wild_airspace = safe_cells.iter().map(|safe| !safe).collect();
    let footsteps = dominant
        .iter()
        .map(|material| Some(materials[*material].surface.get(content).footstep))
        .collect();
    WalkGrids {
        walk,
        wild,
        airspace,
        wild_airspace,
        safe_cells,
        footsteps,
    }
}
