use std::sync::OnceLock;

use super::Area;
use super::prop::PropPlacement;
use super::terrain::{TerrainId, Walk};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::geometry::corners;
use crate::core::math::{Pos, Rect};
use crate::core::units::{Cell, CellPos, GridDims, GridSize, MeterSize, Meters};
use crate::systems::movement::{AGENT_RADIUS, NavMesh};

/// What a terrain is underfoot: whether actors walk on it, and what a step on it sounds like.
#[derive(Clone, Debug)]
pub struct Underfoot {
    pub walkable: bool,
    pub footstep: SfxId,
}

/// Where an area walks and flies, for everyone and for the wild, each built the first time it's
/// asked for: a process moves on few of a game's areas, and a wide area's meshes are large.
#[derive(Default)]
pub struct Meshes {
    walk: OnceLock<NavMesh>,
    wild: OnceLock<NavMesh>,
    airspace: OnceLock<NavMesh>,
    wild_airspace: OnceLock<NavMesh>,
}

/// The mesh `area`'s actors move on, built on first use. The ground blocks where it is painted
/// mostly with a terrain that blocks, where it is steeper than the area's `max_slope`, and under
/// every prop's footprint; a safe zone keeps the wild out of it. The air knows only the bounds
/// and, for the wild, the safe zones.
pub(super) fn mesh(area: &Area, wild: bool, flies: bool) -> &NavMesh {
    let meshes = &area.meshes;
    let build = |obstacles: Vec<Vec<Pos<Meters>>>| {
        NavMesh::build(area.size.bounds(), obstacles, AGENT_RADIUS)
    };
    let zones = || area.safe_zones.iter().map(|zone| corners(*zone).to_vec());
    match (wild, flies) {
        (false, false) => meshes.walk.get_or_init(|| build(blocked(area))),
        (true, false) => meshes
            .wild
            .get_or_init(|| build(blocked(area).into_iter().chain(zones()).collect())),
        (false, true) => meshes.airspace.get_or_init(|| build(Vec::new())),
        (true, true) => meshes
            .wild_airspace
            .get_or_init(|| build(zones().collect())),
    }
}

/// What each of `terrains` is underfoot, by its index in the area.
pub(super) fn underfoot(content: &Content, terrains: &[TerrainId]) -> Vec<Underfoot> {
    terrains
        .iter()
        .map(|terrain| {
            let def = terrain.get(content);
            Underfoot {
                walkable: def.walk == Walk::Walkable,
                footstep: def.surface.get(content).footstep,
            }
        })
        .collect()
}

/// The outline of every prop's footprint, where one blocks the ground.
pub(super) fn footprints(content: &Content, props: &[PropPlacement]) -> Vec<Vec<Pos<Meters>>> {
    props
        .iter()
        .filter_map(|placed| {
            let def = placed.prop.get(content);
            (!def.footprint.is_none())
                .then(|| def.footprint.outline(placed.at, placed.yaw, placed.scale))
        })
        .collect()
}

fn blocked(area: &Area) -> Vec<Vec<Pos<Meters>>> {
    let size = area.size.grid();
    let blocked: Vec<bool> = size
        .cells()
        .map(|cell| {
            !area.underfoot_of(area.paint.dominant(cell)).walkable
                || area.heights.steepest(cell) > area.max_slope
        })
        .collect();
    area.footprints
        .iter()
        .cloned()
        .chain(
            patches(size, &blocked)
                .into_iter()
                .map(|patch| corners(patch).to_vec()),
        )
        .collect()
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
