use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use arc_swap::{ArcSwap, Guard};
use rayon::prelude::*;

use super::Area;
use super::prop::PropPlacement;
use super::terrain::{TerrainId, Walk};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::geometry::corners;
use crate::core::ground::{TILE_SIDE, TileAt};
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{CellPos, GridSize, GroundCell, MeterSize, Meters};
use crate::systems::movement::{NavMesh, TileLayer};

/// What a terrain is underfoot: whether actors walk on it, and what a step on it sounds like.
#[derive(Clone, Debug)]
pub struct Underfoot {
    pub walkable: bool,
    pub footstep: SfxId,
}

/// Where an area walks and flies, for everyone and for the wild, over the tiles of ground held so
/// far: each tile triangulated once as it is first held, and the four meshes joined anew over
/// every held tile whenever more come in. A process moves on the ground near its players alone,
/// however wide the area. A mesh is read without a lock or a count, so every core reads it at
/// once, and swapped whole when it is laid anew.
pub struct Meshes {
    tiles: RwLock<BTreeMap<TileAt, Arc<[TileLayer; KINDS]>>>,
    patches: RwLock<BTreeMap<TileAt, Arc<[Rect<Meters>]>>>,
    meshes: [ArcSwap<NavMesh>; KINDS],
}

/// A mesh as it is read: the one laid when the read began, kept whole until the read ends.
pub type HeldMesh = Guard<Arc<NavMesh>>;

impl Meshes {
    /// The meshes over no ground yet, within `bounds`.
    pub(super) fn empty(bounds: Rect<Meters>) -> Meshes {
        Meshes {
            tiles: RwLock::new(BTreeMap::new()),
            patches: RwLock::new(BTreeMap::new()),
            meshes: std::array::from_fn(|_| {
                ArcSwap::from_pointee(NavMesh::join(bounds, std::iter::empty()))
            }),
        }
    }
}

/// The meshes of an area: the ground and the air, each for everyone and for the wild.
const KINDS: usize = 4;

/// The tiles of `area` held: those its meshes cover.
pub(super) fn held(area: &Area) -> BTreeSet<TileAt> {
    area.meshes
        .tiles
        .read()
        .expect("the tiles lock")
        .keys()
        .copied()
        .collect()
}

/// Whether every one of `tiles` of `area` is held.
pub(super) fn holds(area: &Area, tiles: impl IntoIterator<Item = TileAt>) -> bool {
    let held = area.meshes.tiles.read().expect("the tiles lock");
    tiles.into_iter().all(|at| held.contains_key(&at))
}

/// Holds `tiles` of `area` besides those held, so its meshes cover them: loads their ground,
/// triangulates each new one on a core of its own and joins the meshes anew.
pub(super) fn hold(area: &Area, tiles: impl IntoIterator<Item = TileAt>) {
    let wanted: BTreeSet<TileAt> = tiles.into_iter().collect();
    let missing: Vec<TileAt> = {
        let held = area.meshes.tiles.read().expect("the tiles lock");
        wanted
            .into_iter()
            .filter(|at| !held.contains_key(at))
            .collect()
    };
    if missing.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    let count = missing.len();
    let laid: Vec<(TileAt, Arc<[TileLayer; KINDS]>)> = missing
        .into_par_iter()
        .map(|at| (at, Arc::new(triangulate(area, at))))
        .collect();
    loop {
        let tiles: Vec<Arc<[TileLayer; KINDS]>> = {
            let mut held = area.meshes.tiles.write().expect("the tiles lock");
            held.extend(laid.iter().cloned());
            held.values().cloned().collect()
        };
        let bounds = area.size.bounds();
        let meshes: Vec<Arc<NavMesh>> = (0..KINDS)
            .into_par_iter()
            .map(|kind| Arc::new(NavMesh::join(bounds, tiles.iter().map(|tile| &tile[kind]))))
            .collect();
        let held = area.meshes.tiles.write().expect("the tiles lock");
        if held.len() == tiles.len() {
            for (kind, mesh) in meshes.into_iter().enumerate() {
                area.meshes.meshes[kind].store(mesh);
            }
            bevy::log::debug!(
                "held {count} more ground tiles, meshes over {} in {:?}",
                tiles.len(),
                started.elapsed()
            );
            return;
        }
    }
}

/// The mesh `area`'s actors move on, over the tiles held. The ground blocks where it is painted
/// mostly with a terrain that blocks, where it is steeper than the area's `max_slope`, and under
/// every prop's footprint; a safe zone keeps the wild out of it. The air knows only the bounds
/// and, for the wild, the safe zones.
pub(super) fn mesh(area: &Area, wild: bool, flies: bool) -> HeldMesh {
    area.meshes.meshes[kind(wild, flies)].load()
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

/// Which of the four meshes: the ground's for everyone, the wild's, the air's, the wild air's.
fn kind(wild: bool, flies: bool) -> usize {
    usize::from(wild) | usize::from(flies) << 1
}

/// The four layers of the tile at `at`.
fn triangulate(area: &Area, at: TileAt) -> [TileLayer; KINDS] {
    let size = area.heights.size();
    let tile = at.rect(size);
    let reach = tile.inflate(area.bodies.radius.0, area.bodies.radius.0);
    let bounds = area.size.bounds();
    let mut ground: Vec<Vec<Pos<Meters>>> = area
        .footprint_index
        .within(reach)
        .map(|footprint| area.footprints[footprint].clone())
        .collect();
    ground.extend(
        size.tiles_within(tile.inflate(TILE_SIDE.0, TILE_SIDE.0))
            .flat_map(|near| blocked(area, near).to_vec())
            .filter(|patch| patch.intersects(&reach))
            .map(|patch| corners(patch).to_vec()),
    );
    let zones: Vec<Vec<Pos<Meters>>> = area
        .safe_zones
        .iter()
        .map(|zone| corners(*zone).to_vec())
        .collect();
    let radius = area.bodies.radius;
    let [walks, wild] =
        NavMesh::triangulate_sharing(tile, bounds, radius, &ground, [&[], &zones[..]]);
    [
        walks,
        wild,
        NavMesh::triangulate(tile, bounds, &[], radius),
        NavMesh::triangulate(tile, bounds, &zones, radius),
    ]
}

/// The patches of ground blocked on tile `at`, computed once: terrain that doesn't walk, and
/// ground steeper than the area allows. A cell's steepness reads the samples on its far edges,
/// which on the tile's last row and column lie on the next tiles, so those are loaded first: a
/// patch must come out the same whenever it is asked for, as the tiles meeting at a seam each
/// triangulate the patches reaching across it.
fn blocked(area: &Area, at: TileAt) -> Arc<[Rect<Meters>]> {
    if let Some(patches) = area
        .meshes
        .patches
        .read()
        .expect("the patches lock")
        .get(&at)
    {
        return patches.clone();
    }
    let size = area.heights.size();
    let tile = at.rect(size);
    for near in size.tiles_within(Rect::new(tile.origin, tile.size + Size::new(1.0, 1.0))) {
        if let Err(problem) = area.heights.load(near) {
            bevy::log::warn!("ground tile {} doesn't load: {problem}", near.key());
        }
    }
    if let Err(problem) = area.paint.load(at) {
        bevy::log::warn!("paint tile {} doesn't load: {problem}", at.key());
    }
    let grid = area.size.grid();
    let side = TILE_SIDE.0 as u32;
    let origin = CellPos::new((at.column * side) as i32, (at.row * side) as i32);
    let dims = GridSize::new(
        side.min(grid.width.saturating_sub(at.column * side)),
        side.min(grid.height.saturating_sub(at.row * side)),
    );
    let cells: Vec<bool> = area
        .paint
        .dominant_in(origin, dims)
        .into_iter()
        .zip(area.heights.steepest_in(origin, dims))
        .map(|(terrain, slope)| !area.underfoot_of(terrain).walkable || slope > area.max_slope)
        .collect();
    let patches: Arc<[Rect<Meters>]> = patches(origin, dims, &cells).into();
    area.meshes
        .patches
        .write()
        .expect("the patches lock")
        .entry(at)
        .or_insert(patches)
        .clone()
}

/// The blocked cells of a tile of `size` cells from `origin` as rectangles: each row's runs, and
/// runs alike on consecutive rows joined.
fn patches(origin: CellPos, size: GridSize, blocked: &[bool]) -> Vec<Rect<Meters>> {
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
            let run = (origin + CellPos::new(start as i32, y as i32).to_vector())
                .bounds()
                .union(&(origin + CellPos::new(x as i32 - 1, y as i32).to_vector()).bounds());
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
