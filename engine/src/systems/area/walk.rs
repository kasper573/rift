use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

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
use crate::systems::movement::{NavMesh, SEAM_SNAP, TileLayer};

/// What a terrain is underfoot: whether actors walk on it, and what a step on it sounds like.
#[derive(Clone, Debug)]
pub struct Underfoot {
    pub walkable: bool,
    pub footstep: SfxId,
}

/// Where an area walks and flies, for everyone and for the wild, over the tiles of ground held:
/// each tile triangulated as it is first held, and the four meshes joined anew over the held tiles
/// whenever the tiles held change. A tile is held for as long as a [`GroundHold`] wants it, so a
/// process holds the ground near its players or in its view alone, however wide the area and
/// however far they have been. A mesh is read without a lock or a count, so every core reads it at
/// once, and swapped whole when it is laid anew.
pub struct Meshes {
    bounds: Rect<Meters>,
    held: RwLock<Held>,
    patches: RwLock<BTreeMap<TileAt, Arc<[Rect<Meters>]>>>,
    meshes: [ArcSwap<NavMesh>; KINDS],
    /// Which change of the tiles held the meshes were last joined over. One join runs at a time,
    /// so whoever asks for one while another is under way waits for meshes that cover its tiles.
    joined: Mutex<u64>,
}

/// A mesh as it is read: the one laid when the read began, kept whole until the read ends.
pub type HeldMesh = Guard<Arc<NavMesh>>;

/// Ground of an area held for as long as this lives: the area's meshes cover its tiles until it is
/// dropped or moved onto others, and a tile no hold wants any longer is let go.
pub struct GroundHold {
    meshes: Arc<Meshes>,
    tiles: BTreeSet<TileAt>,
}

impl GroundHold {
    pub fn tiles(&self) -> &BTreeSet<TileAt> {
        &self.tiles
    }

    /// Whether the ground held is `area`'s.
    pub fn holds_on(&self, area: &Area) -> bool {
        Arc::ptr_eq(&self.meshes, &area.meshes)
    }
}

impl Drop for GroundHold {
    fn drop(&mut self) {
        if release(&self.meshes, &std::mem::take(&mut self.tiles)) {
            join(&self.meshes);
        }
    }
}

impl Meshes {
    /// The meshes over no ground yet, within `bounds`.
    pub(super) fn empty(bounds: Rect<Meters>) -> Arc<Meshes> {
        Arc::new(Meshes {
            bounds,
            held: RwLock::new(Held::default()),
            patches: RwLock::new(BTreeMap::new()),
            meshes: std::array::from_fn(|_| {
                ArcSwap::from_pointee(NavMesh::join(bounds, std::iter::empty()))
            }),
            joined: Mutex::new(0),
        })
    }
}

/// The tiles held, each with its layers and how many holds want it, and how many times they have
/// changed, so meshes joined over tiles that changed meanwhile are joined anew.
#[derive(Default)]
struct Held {
    tiles: BTreeMap<TileAt, HeldTile>,
    changes: u64,
}

struct HeldTile {
    layers: Arc<[TileLayer; KINDS]>,
    holds: usize,
}

/// The meshes of an area: the ground and the air, each for everyone and for the wild.
const KINDS: usize = 4;

/// The tiles of `area` held: those its meshes cover.
pub(super) fn held(area: &Area) -> BTreeSet<TileAt> {
    area.meshes
        .held
        .read()
        .expect("the held lock")
        .tiles
        .keys()
        .copied()
        .collect()
}

/// Whether every one of `tiles` of `area` is held.
pub(super) fn holds(area: &Area, tiles: impl IntoIterator<Item = TileAt>) -> bool {
    let held = area.meshes.held.read().expect("the held lock");
    tiles.into_iter().all(|at| held.tiles.contains_key(&at))
}

/// Holds `tiles` of `area` for as long as the hold lives, letting go of those `previous` held
/// that it no longer wants: loads the ground of tiles not held yet, triangulates each on a core of
/// its own and joins the meshes anew once.
pub(super) fn hold(
    area: &Area,
    previous: Option<GroundHold>,
    tiles: impl IntoIterator<Item = TileAt>,
) -> GroundHold {
    let wanted: BTreeSet<TileAt> = tiles.into_iter().collect();
    let missing: Vec<TileAt> = {
        let held = area.meshes.held.read().expect("the held lock");
        wanted
            .iter()
            .filter(|at| !held.tiles.contains_key(at))
            .copied()
            .collect()
    };
    let laid: BTreeMap<TileAt, Arc<[TileLayer; KINDS]>> = missing
        .into_par_iter()
        .map(|at| (at, Arc::new(triangulate(area, at))))
        .collect();
    let mut changed = false;
    {
        let mut held = area.meshes.held.write().expect("the held lock");
        for at in &wanted {
            match held.tiles.get_mut(at) {
                Some(tile) => tile.holds += 1,
                None => {
                    let layers = laid
                        .get(at)
                        .cloned()
                        .unwrap_or_else(|| Arc::new(triangulate(area, *at)));
                    held.tiles.insert(*at, HeldTile { layers, holds: 1 });
                    changed = true;
                }
            }
        }
        held.changes += u64::from(changed);
    }
    match previous {
        Some(mut previous) if Arc::ptr_eq(&previous.meshes, &area.meshes) => {
            release(&area.meshes, &std::mem::take(&mut previous.tiles));
        }
        other => drop(other),
    }
    join(&area.meshes);
    GroundHold {
        meshes: area.meshes.clone(),
        tiles: wanted,
    }
}

/// The mesh `area`'s actors move on, over the tiles held. The ground blocks where it is painted
/// mostly with a terrain that blocks, where it is steeper than the area's `max_slope`, and under
/// every prop's footprint; a safe zone keeps the wild out of it. The air knows only the bounds
/// and, for the wild, the safe zones.
pub(super) fn mesh(area: &Area, wild: bool, flies: bool) -> HeldMesh {
    area.meshes.meshes[kind(wild, flies)].load()
}

/// An area's ground looked over a tile at a time, for a check of all of it, however wide: each
/// question triangulates only the tiles it looks at, a few kept at hand for the next, and which
/// ground connects to which across the whole area is worked out once, a tile at a time, keeping
/// no more of each than its islands. The area's held ground is left as it is.
pub struct Survey<'a> {
    area: &'a Area,
    at_hand: Mutex<VecDeque<(TileAt, Arc<SurveyedTile>)>>,
    islands: OnceLock<SurveyIslands>,
}

/// A stretch of an area's ground connected within itself and cut off from the rest, as a
/// [`Survey`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurveyIsland(u32);

/// What a check asks a [`Survey`] of the ground.
#[derive(Clone, Copy, Debug)]
pub enum SurveyQuestion {
    /// Whether everyone walks at the point.
    Walkable(Pos<Meters>),
    /// Whether some ground within the bounds is walked: by the wild, or by everyone.
    Covers { bounds: Rect<Meters>, wild: bool },
    /// The islands of the ground everyone walks within the bounds.
    Islands(Rect<Meters>),
    /// The island of the ground everyone walks at the point, none where nobody walks there.
    IslandAt(Pos<Meters>),
}

impl SurveyQuestion {
    fn bounds(&self) -> Rect<Meters> {
        match *self {
            SurveyQuestion::Walkable(at) | SurveyQuestion::IslandAt(at) => {
                Rect::new(at, Size::zero())
            }
            SurveyQuestion::Covers { bounds, .. } | SurveyQuestion::Islands(bounds) => bounds,
        }
    }

    fn islands(&self) -> bool {
        matches!(
            self,
            SurveyQuestion::Islands(_) | SurveyQuestion::IslandAt(_)
        )
    }
}

/// A [`Survey`]'s answer to a [`SurveyQuestion`]: yes or no, or the islands asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum SurveyAnswer {
    Yes(bool),
    Islands(BTreeSet<SurveyIsland>),
}

impl SurveyAnswer {
    pub fn yes(&self) -> bool {
        matches!(self, SurveyAnswer::Yes(true))
    }

    pub fn islands(&self) -> BTreeSet<SurveyIsland> {
        match self {
            SurveyAnswer::Islands(islands) => islands.clone(),
            SurveyAnswer::Yes(_) => BTreeSet::new(),
        }
    }
}

impl<'a> Survey<'a> {
    pub(super) fn of(area: &'a Area) -> Survey<'a> {
        Survey {
            area,
            at_hand: Mutex::new(VecDeque::new()),
            islands: OnceLock::new(),
        }
    }

    /// Whether everyone walks at `at`.
    pub fn walkable(&self, at: Pos<Meters>) -> bool {
        self.tile(self.area.ground_size().tile_at(at))
            .walks
            .walkable(at)
    }

    /// The island `at` stands on, none where nobody walks.
    pub fn island(&self, at: Pos<Meters>) -> Option<SurveyIsland> {
        let tile = self.area.ground_size().tile_at(at);
        let local = self.tile(tile).walks.island(at)?;
        self.islands().island_of(tile, local.index())
    }

    /// A point on each piece of the ground everyone walks within `bounds`.
    pub fn ground_within(&self, bounds: Rect<Meters>) -> Vec<Pos<Meters>> {
        self.area
            .ground_size()
            .tiles_within(bounds)
            .flat_map(|at| self.tile(at).walks.ground_within(bounds))
            .collect()
    }

    /// The answers to `questions`, in their order: each tile they ask about is laid once, on a
    /// core of its own, however many ask about it, so a check of a whole wide area asks all it
    /// has to at once. Asked about islands before they are worked out, the survey lays every tile
    /// in the same pass, working them out from the tiles it answers on.
    pub fn answer(&self, questions: &[SurveyQuestion]) -> Vec<SurveyAnswer> {
        let size = self.area.ground_size();
        let mut asked: BTreeMap<TileAt, Vec<usize>> = BTreeMap::new();
        for (index, question) in questions.iter().enumerate() {
            for at in size.tiles_within(question.bounds()) {
                asked.entry(at).or_default().push(index);
            }
        }
        let lays_islands =
            questions.iter().any(SurveyQuestion::islands) && self.islands.get().is_none();
        let tiles: Vec<TileAt> = if lays_islands {
            size.each_tile().collect()
        } else {
            asked.keys().copied().collect()
        };
        let surveyed: Vec<AnsweredTile> = tiles
            .into_par_iter()
            .map(|at| {
                let asking = asked.get(&at).map(Vec::as_slice).unwrap_or_default();
                let wild = asking.iter().any(|index| {
                    matches!(questions[*index], SurveyQuestion::Covers { wild: true, .. })
                });
                let tile = SurveyedTile::of(self.area, at, wild);
                AnsweredTile {
                    at,
                    rims: lays_islands.then(|| tile.rims()),
                    answers: asking
                        .iter()
                        .map(|index| (*index, tile.answer(&questions[*index])))
                        .collect(),
                }
            })
            .collect();
        let mut rims = Vec::new();
        let mut partial = Vec::with_capacity(surveyed.len());
        for tile in surveyed {
            rims.extend(tile.rims.map(|tile_rims| (tile.at, tile_rims)));
            partial.push((tile.at, tile.answers));
        }
        if lays_islands {
            let _ = self.islands.set(SurveyIslands::joining(rims));
        }
        let islands = self.islands.get();
        let mut answers: Vec<SurveyAnswer> = questions
            .iter()
            .map(|question| {
                if question.islands() {
                    SurveyAnswer::Islands(BTreeSet::new())
                } else {
                    SurveyAnswer::Yes(false)
                }
            })
            .collect();
        for (at, tile_answers) in partial {
            for (index, answer) in tile_answers {
                match (&mut answers[index], answer) {
                    (SurveyAnswer::Yes(yes), TileAnswer::Yes(more)) => *yes |= more,
                    (SurveyAnswer::Islands(found), TileAnswer::Islands(local)) => {
                        found.extend(local.into_iter().filter_map(|local| {
                            islands.and_then(|islands| islands.island_of(at, local))
                        }));
                    }
                    _ => {}
                }
            }
        }
        answers
    }

    /// The nearest point everyone walks to `at` that `fits`, searched along rings `step` apart
    /// over the ground a tile around `at`.
    pub fn nearest_where(
        &self,
        at: Pos<Meters>,
        step: Meters,
        fits: impl Fn(Pos<Meters>) -> bool,
    ) -> Option<Pos<Meters>> {
        self.around(at).nearest_where(at, step, fits)
    }

    /// The nearest point everyone walks to `at`, over the ground a tile around it.
    pub fn nearest(&self, at: Pos<Meters>) -> Option<Pos<Meters>> {
        self.around(at).nearest(at)
    }

    /// The ground everyone walks a tile around `at`, joined as one mesh.
    pub(super) fn around(&self, at: Pos<Meters>) -> NavMesh {
        let around = Rect::new(at, Size::zero()).inflate(TILE_SIDE.0, TILE_SIDE.0);
        let tiles: Vec<Arc<SurveyedTile>> = self
            .area
            .ground_size()
            .tiles_within(around)
            .map(|at| self.tile(at))
            .collect();
        NavMesh::join(
            self.area.size.bounds(),
            tiles.iter().map(|tile| &tile.layer),
        )
    }

    /// The tile at `at` as the survey looks at it: from those at hand, else triangulated anew.
    fn tile(&self, at: TileAt) -> Arc<SurveyedTile> {
        if let Some((_, tile)) = self
            .at_hand
            .lock()
            .expect("the survey's tiles")
            .iter()
            .find(|(held, _)| *held == at)
        {
            return tile.clone();
        }
        let tile = Arc::new(SurveyedTile::of(self.area, at, false));
        let mut at_hand = self.at_hand.lock().expect("the survey's tiles");
        if at_hand.len() == AT_HAND {
            at_hand.pop_front();
        }
        at_hand.push_back((at, tile.clone()));
        tile
    }

    fn islands(&self) -> &SurveyIslands {
        self.islands.get_or_init(|| SurveyIslands::of(self.area))
    }
}

/// How many tiles a survey keeps at hand: enough for the questions about one place and its
/// surroundings to find them there.
const AT_HAND: usize = 32;

/// A tile as a survey looks at it: the layer and the ground everyone walks on it, and the wild's
/// where a question asks of it.
struct SurveyedTile {
    layer: TileLayer,
    walks: NavMesh,
    wild: Option<NavMesh>,
}

impl SurveyedTile {
    /// The tile at `at`, with the wild's ground where `wild`.
    fn of(area: &Area, at: TileAt, wild: bool) -> SurveyedTile {
        let TileGround {
            tile,
            ground,
            zones,
        } = TileGround::of(area, at);
        let bounds = area.size.bounds();
        let radius = area.bodies.radius;
        let (layer, wild) = if wild {
            let [walks, wild] =
                NavMesh::triangulate_sharing(tile, bounds, radius, &ground, [&[], &zones[..]]);
            (walks, Some(NavMesh::join(bounds, [&wild])))
        } else {
            (NavMesh::triangulate(tile, bounds, &ground, radius), None)
        };
        SurveyedTile {
            walks: NavMesh::join(bounds, [&layer]),
            wild,
            layer,
        }
    }

    fn rims(&self) -> Rims {
        Rims::of(&self.walks, self.layer.rect())
    }

    /// What the tile answers to `question`, its islands its own.
    fn answer(&self, question: &SurveyQuestion) -> TileAnswer {
        match *question {
            SurveyQuestion::Walkable(point) => TileAnswer::Yes(self.walks.walkable(point)),
            SurveyQuestion::Covers { bounds, wild } => TileAnswer::Yes(if wild {
                self.wild
                    .as_ref()
                    .expect("the wild's ground, laid for the questions that ask of it")
                    .covers(bounds)
            } else {
                self.walks.covers(bounds)
            }),
            SurveyQuestion::Islands(bounds) => TileAnswer::Islands(
                self.walks
                    .ground_within(bounds)
                    .into_iter()
                    .filter_map(|point| Some(self.walks.island(point)?.index()))
                    .collect(),
            ),
            SurveyQuestion::IslandAt(point) => TileAnswer::Islands(
                self.walks
                    .island(point)
                    .map(|island| island.index())
                    .into_iter()
                    .collect(),
            ),
        }
    }
}

/// A tile a survey answered on: its answers to the questions asking of it, by their index, and its
/// rims where the islands are worked out from it.
struct AnsweredTile {
    at: TileAt,
    rims: Option<Rims>,
    answers: Vec<(usize, TileAnswer)>,
}

/// A tile's answer to a [`SurveyQuestion`]: yes or no, or the tile's own islands asked for.
enum TileAnswer {
    Yes(bool),
    Islands(Vec<usize>),
}

/// The islands of each tile of an area, as islands of the whole area: those meeting across a seam
/// are one, as they are where the tiles' meshes are joined.
struct SurveyIslands {
    /// Each tile's first island among all the tiles', and how many it has.
    first: BTreeMap<TileAt, (u32, u32)>,
    island: Vec<u32>,
}

impl SurveyIslands {
    fn of(area: &Area) -> SurveyIslands {
        SurveyIslands::joining(
            area.ground_size()
                .each_tile()
                .collect::<Vec<_>>()
                .into_par_iter()
                .map(|at| (at, SurveyedTile::of(area, at, false).rims()))
                .collect(),
        )
    }

    /// The islands of the tiles whose `tiles` these are, joined across their seams.
    fn joining(tiles: Vec<(TileAt, Rims)>) -> SurveyIslands {
        let mut first = BTreeMap::new();
        let mut count = 0;
        for (at, rims) in &tiles {
            first.insert(*at, (count, rims.islands));
            count += rims.islands;
        }
        let mut parent: Vec<u32> = (0..count).collect();
        let by_tile: BTreeMap<TileAt, &Rims> = tiles.iter().map(|(at, rims)| (*at, rims)).collect();
        for (at, rims) in &by_tile {
            let east = TileAt {
                column: at.column + 1,
                row: at.row,
            };
            let south = TileAt {
                column: at.column,
                row: at.row + 1,
            };
            if let Some(next) = by_tile.get(&east) {
                for (a, b) in meeting(&rims.east, &next.west) {
                    unite(&mut parent, first[at].0 + a, first[&east].0 + b);
                }
            }
            if let Some(next) = by_tile.get(&south) {
                for (a, b) in meeting(&rims.south, &next.north) {
                    unite(&mut parent, first[at].0 + a, first[&south].0 + b);
                }
            }
        }
        let island = (0..count).map(|index| root(&mut parent, index)).collect();
        SurveyIslands { first, island }
    }

    /// The island of the whole area the `local`th island of the tile at `at` belongs to.
    fn island_of(&self, at: TileAt, local: usize) -> Option<SurveyIsland> {
        let (first, count) = *self.first.get(&at)?;
        (local < count as usize).then(|| SurveyIsland(self.island[first as usize + local]))
    }
}

/// The corners of a tile's mesh lying on each edge of its ground, each with the island it is a
/// corner of, by how far along the edge it lies.
struct Rims {
    islands: u32,
    north: Vec<(f32, u32)>,
    south: Vec<(f32, u32)>,
    west: Vec<(f32, u32)>,
    east: Vec<(f32, u32)>,
}

impl Rims {
    fn of(mesh: &NavMesh, rect: Rect<Meters>) -> Rims {
        let mut rims = Rims {
            islands: mesh.island_count() as u32,
            north: Vec::new(),
            south: Vec::new(),
            west: Vec::new(),
            east: Vec::new(),
        };
        if rect.is_empty() {
            return rims;
        }
        for (corner, island) in mesh.corners_by_island() {
            let island = island.index() as u32;
            if corner.y == rect.min_y() {
                rims.north.push((corner.x, island));
            }
            if corner.y == rect.max_y() {
                rims.south.push((corner.x, island));
            }
            if corner.x == rect.min_x() {
                rims.west.push((corner.y, island));
            }
            if corner.x == rect.max_x() {
                rims.east.push((corner.y, island));
            }
        }
        for rim in [
            &mut rims.north,
            &mut rims.south,
            &mut rims.west,
            &mut rims.east,
        ] {
            rim.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        rims
    }
}

/// The islands on either side of a seam that meet there: those with a corner where the other
/// side has one.
fn meeting<'a>(
    one: &'a [(f32, u32)],
    other: &'a [(f32, u32)],
) -> impl Iterator<Item = (u32, u32)> + 'a {
    one.iter().flat_map(move |(along, island)| {
        let from = other.partition_point(|(at, _)| *at < along - SEAM_SNAP);
        other[from..]
            .iter()
            .take_while(move |(at, _)| *at <= along + SEAM_SNAP)
            .map(move |(_, other)| (*island, *other))
    })
}

fn root(parent: &mut [u32], mut index: u32) -> u32 {
    while parent[index as usize] != index {
        parent[index as usize] = parent[parent[index as usize] as usize];
        index = parent[index as usize];
    }
    index
}

fn unite(parent: &mut [u32], a: u32, b: u32) {
    let (a, b) = (root(parent, a), root(parent, b));
    if a != b {
        parent[a as usize] = b;
    }
}

/// Lets go of a hold on each of `tiles`, and of the tiles no hold wants any longer: whether any
/// was let go.
fn release(meshes: &Meshes, tiles: &BTreeSet<TileAt>) -> bool {
    if tiles.is_empty() {
        return false;
    }
    let mut held = meshes.held.write().expect("the held lock");
    let mut released = false;
    for at in tiles {
        if let Some(tile) = held.tiles.get_mut(at) {
            tile.holds -= 1;
            if tile.holds == 0 {
                held.tiles.remove(at);
                released = true;
            }
        }
    }
    held.changes += u64::from(released);
    released
}

/// Joins the meshes anew over the tiles held unless they cover them already, and again if the
/// tiles change meanwhile. Whoever holds a tile another hold is still laying the meshes over
/// waits here until they cover it.
fn join(meshes: &Meshes) {
    let mut joined = meshes.joined.lock().expect("the join lock");
    loop {
        let started = std::time::Instant::now();
        let (changes, tiles): (u64, Vec<Arc<[TileLayer; KINDS]>>) = {
            let held = meshes.held.read().expect("the held lock");
            (
                held.changes,
                held.tiles
                    .values()
                    .map(|tile| tile.layers.clone())
                    .collect(),
            )
        };
        if *joined == changes {
            return;
        }
        let laid: Vec<Arc<NavMesh>> = (0..KINDS)
            .into_par_iter()
            .map(|kind| {
                Arc::new(NavMesh::join(
                    meshes.bounds,
                    tiles.iter().map(|tile| &tile[kind]),
                ))
            })
            .collect();
        for (kind, mesh) in laid.into_iter().enumerate() {
            meshes.meshes[kind].store(mesh);
        }
        *joined = changes;
        bevy::log::debug!(
            "meshes over {} ground tiles in {:?}",
            tiles.len(),
            started.elapsed()
        );
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

/// Which of the four meshes: the ground's for everyone, the wild's, the air's, the wild air's.
fn kind(wild: bool, flies: bool) -> usize {
    usize::from(wild) | usize::from(flies) << 1
}

/// The four layers of the tile at `at`.
fn triangulate(area: &Area, at: TileAt) -> [TileLayer; KINDS] {
    let TileGround {
        tile,
        ground,
        zones,
    } = TileGround::of(area, at);
    let bounds = area.size.bounds();
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

/// The ground of a tile as it is triangulated: its rect, the outlines no one walks within, and the
/// safe zones the wild keeps out of besides.
struct TileGround {
    tile: Rect<Meters>,
    ground: Vec<Vec<Pos<Meters>>>,
    zones: Vec<Vec<Pos<Meters>>>,
}

impl TileGround {
    fn of(area: &Area, at: TileAt) -> TileGround {
        let size = area.heights.size();
        let tile = at.rect(size);
        let reach = tile.inflate(area.bodies.radius.0, area.bodies.radius.0);
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
        let zones = area
            .safe_zones
            .iter()
            .map(|zone| corners(*zone).to_vec())
            .collect();
        TileGround {
            tile,
            ground,
            zones,
        }
    }
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
    if let Err(problem) = area.water.load(at) {
        bevy::log::warn!("water tile {} doesn't load: {problem}", at.key());
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
