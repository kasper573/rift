//! The ground as the GPU holds it to draw. The overview of the whole area is always there. Each
//! tile the views want in detail holds a slot of a texture array: its heights, paint and water,
//! and how far its water lies from the shore, with one value of its neighbours' all round, so what
//! reads a tile reads across its seams from its slot alone. A table of the area's tiles says which slot holds each, so a draw reads the
//! overview wherever a tile holds none. A tile keeps its slot while it is wanted and is filled
//! again as its values change; a tile wanted anew takes a free slot or the one wanted longest ago.
//! Over the overview lies the canopy: what the area's props look like from above wherever they
//! stand, for the ground to show where the view thins them away.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::{RenderEntity, SyncToRenderWorld};
use bevy::render::{ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};

use super::residency::Residency;
use super::upload::UploadBudget;
use crate::core::ground::{
    GroundGrid, GroundSize, GroundValue, HeightRange, Heightfield, Level, OVERVIEW_STRIDE,
    PaintMap, SAMPLES_PER_METER, TILE_PIXELS, TileAt, TileVersion, WaterLevel, WaterSurface,
    shore_distances,
};
use crate::core::jobs::Handoff;
use crate::core::math::{Rect, Size};
use crate::core::render::sample::UploadedBytes;
use crate::core::render::world::cover::SHORE_REACH;
use crate::core::units::{GROUND_ORIGIN, Meters};

/// How many tiles the GPU holds in detail at once.
pub const DETAIL_SLOTS: u32 = 49;
/// The values along a slot's side: a tile's samples, and one more all round.
pub const SLOT_SIDE: u32 = TILE_PIXELS + 3;
/// The widest a canopy texel's props are told to be: an extent past it reads as this wide.
pub const CANOPY_EXTENT_MAX: Meters = Meters(256.0);
/// The canopy keeps the props over a texel in bands of their extent, a layer each, so the view
/// thins each band on its own: a band holds the props up to its extent that the band below
/// doesn't, the last every wider one.
pub const CANOPY_BANDS: [Meters; 4] = [Meters(1.0), Meters(4.0), Meters(16.0), Meters(64.0)];

pub struct TerrainAtlasPlugin;

impl Plugin for TerrainAtlasPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, keep_atlases);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(ExtractSchedule, extract_writes)
            .add_systems(Render, write_atlas.in_set(RenderSystems::PrepareResources));
        app.insert_resource(AtlasesDrawn);
    }
}

/// The ground a terrain draws, as the main world keeps the GPU's in step with it.
#[derive(Component, Default)]
#[require(SyncToRenderWorld)]
pub struct TerrainAtlas {
    laid: Option<Laid>,
    wanted: Vec<TileAt>,
    writes: Vec<AtlasWrite>,
}

/// How the atlas's textures lie over an area's ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasLayout {
    pub size: GroundSize,
    pub range: HeightRange,
    /// The tiles across and down, a texel of the table each.
    pub tiles: (u32, u32),
    /// The overview's values each way to a texel of its textures: a power of two, more than one
    /// only where the overview is wider than a texture may be.
    pub overview_step: u32,
    /// The texels across and down of the overview of the heights and the water.
    pub heights_overview: (u32, u32),
    /// The texels across and down of the overview of the paint.
    pub paint_overview: (u32, u32),
    /// The texels across and down of the canopy.
    pub canopy: (u32, u32),
}

/// The textures of the atlas, as they are bound to draw: the detail arrays a layer a slot, the
/// overviews, and the table naming the slot of each tile, one past it, or none. A shore texel says
/// how far its point lies from the nearest dry ground, up to [`SHORE_REACH`] at 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasTexture {
    Heights,
    Paint,
    Water,
    Shore,
    OverviewHeights,
    OverviewPaint,
    OverviewWater,
    OverviewShore,
    /// What the props standing over each stretch of ground look like from above, a layer a
    /// band of [`CANOPY_BANDS`]: their colour, and how much of the ground they cover.
    OverviewCanopy,
    /// How wide those props are, a layer a band, as a share of [`CANOPY_EXTENT_MAX`].
    OverviewCanopyExtent,
    Table,
}

/// What the area's props look like from above over each texel of the canopy, laid out for a
/// layout of the atlas, in the bands of [`CANOPY_BANDS`]: a colour with how much of the texel's
/// ground the band's props cover, and how wide they typically are.
#[derive(Clone, Debug, PartialEq)]
pub struct CanopyMap {
    pub layout: AtlasLayout,
    /// Red, green and blue in sRGB, then the cover: a texel each, row by row, a band after the
    /// other.
    pub colors: Vec<[u8; 4]>,
    /// The props' extent as a share of [`CANOPY_EXTENT_MAX`] in 16 bits, laid out as the colours.
    pub extents: Vec<u16>,
}

impl CanopyMap {
    /// The band of [`CANOPY_BANDS`] a prop `extent` wide belongs to.
    pub fn band(extent: Meters) -> usize {
        CANOPY_BANDS
            .iter()
            .position(|band| extent < *band)
            .unwrap_or(CANOPY_BANDS.len() - 1)
    }

    /// How many texels each band holds.
    pub fn texels(layout: &AtlasLayout) -> usize {
        (layout.canopy.0 * layout.canopy.1) as usize
    }
}

impl TerrainAtlas {
    /// Holds the ground of `heights`, `paint` and `water`: laid anew for a ground of another size
    /// or range, else each tile filled again as its values changed.
    pub fn lay(&mut self, heights: &Heightfield, paint: &PaintMap, water: &WaterSurface) {
        let ground = AtlasGround {
            heights: heights.clone(),
            paint: paint.clone(),
            water: water.clone(),
        };
        let layout = AtlasLayout::of(heights, paint);
        match &mut self.laid {
            Some(laid) if laid.layout == layout => laid.relay(ground),
            _ => {
                self.writes.push(AtlasWrite::Lay(layout));
                self.laid = Some(Laid::new(ground, layout));
            }
        }
    }

    /// Lets go of the ground: nothing is drawn from the atlas until one is laid again.
    pub fn clear(&mut self) {
        self.laid = None;
        self.wanted.clear();
    }

    /// Asks for `tiles` in detail from now on, the most wanted first, until other tiles are asked
    /// for: as many as there are slots keep or take one, and the rest draw from the overview.
    pub fn want(&mut self, tiles: impl IntoIterator<Item = TileAt>) {
        self.wanted.clear();
        self.wanted.extend(tiles);
    }

    /// How much of what is asked for the GPU has nothing of yet: the overview until it is first
    /// sent, and each tile wanted there is a slot for that holds no slot.
    pub fn missing(&self) -> u32 {
        let Some(laid) = &self.laid else {
            return 0;
        };
        let overview = u32::from(!laid.overview_sent);
        let tiles = self
            .wanted
            .iter()
            .take(DETAIL_SLOTS as usize)
            .filter(|&&tile| laid.residency.slot(tile).is_none())
            .count() as u32;
        overview + tiles
    }

    /// How much of what is asked for the GPU draws as it stood before a change: the overview
    /// while it is drawn anew, and each tile whose slot holds earlier values than its own.
    pub fn stale(&self) -> u32 {
        let Some(laid) = &self.laid else {
            return 0;
        };
        let overview = u32::from(
            laid.overview_sent && (!laid.overview.is_empty() || !laid.overview_writes.is_empty()),
        );
        let tiles = self
            .wanted
            .iter()
            .take(DETAIL_SLOTS as usize)
            .filter(|&&tile| {
                let round = AtlasGround::round(tile, laid.layout.tiles);
                laid.residency.slot(tile).is_some()
                    && !laid.residency.is_current(tile, &laid.ground.stamp(&round))
            })
            .count() as u32;
        overview + tiles
    }

    pub fn layout(&self) -> Option<AtlasLayout> {
        self.laid.as_ref().map(|laid| laid.layout)
    }

    /// Whether the overview is on its way to the GPU, so the whole ground draws.
    pub fn is_laid(&self) -> bool {
        self.laid.as_ref().is_some_and(|laid| laid.overview_sent)
    }

    /// The slot holding `tile` in detail.
    pub fn slot(&self, tile: TileAt) -> Option<u32> {
        self.laid.as_ref()?.residency.slot(tile)
    }

    /// The paint of the ground laid.
    pub fn paint(&self) -> Option<&PaintMap> {
        self.laid.as_ref().map(|laid| &laid.ground.paint)
    }

    /// Lays `canopy` over the ground laid, if it was worked out for its layout: written after
    /// whatever of the overview is on its way, within the frames' budgets.
    pub fn lay_canopy(&mut self, canopy: &CanopyMap) {
        let Some(laid) = &mut self.laid else {
            return;
        };
        if laid.layout != canopy.layout {
            return;
        }
        let (columns, rows) = canopy.layout.canopy;
        let band_texels = CanopyMap::texels(&canopy.layout);
        for band in 0..CANOPY_BANDS.len() {
            for first_row in (0..rows).step_by(CANOPY_ROWS_PER_WRITE as usize) {
                let past = (first_row + CANOPY_ROWS_PER_WRITE).min(rows);
                let first = band * band_texels + (first_row * columns) as usize;
                let texels = first..first + ((past - first_row) * columns) as usize;
                let size = UVec2::new(columns, past - first_row);
                let origin = UVec3::new(0, first_row, band as u32);
                laid.overview_writes.push_back(AtlasWrite::Texels {
                    texture: AtlasTexture::OverviewCanopy,
                    origin,
                    size,
                    bytes: canopy.colors[texels.clone()].concat(),
                });
                laid.overview_writes.push_back(AtlasWrite::Texels {
                    texture: AtlasTexture::OverviewCanopyExtent,
                    origin,
                    size,
                    bytes: canopy.extents[texels]
                        .iter()
                        .flat_map(|extent| extent.to_le_bytes())
                        .collect(),
                });
            }
        }
    }
}

impl AtlasLayout {
    /// The ground a canopy texel spans each way: the first texel from the ground's corner.
    pub fn canopy_texel(&self) -> Meters {
        Meters(
            (OVERVIEW_STRIDE * self.overview_step * CANOPY_STRIDE) as f32
                / SAMPLES_PER_METER as f32,
        )
    }

    /// The ground the canopy's texels span.
    pub fn canopy_bounds(&self) -> Rect<Meters> {
        let texel = self.canopy_texel().0;
        Rect::new(
            GROUND_ORIGIN,
            Size::new(self.canopy.0 as f32 * texel, self.canopy.1 as f32 * texel),
        )
    }

    fn of(heights: &Heightfield, paint: &PaintMap) -> AtlasLayout {
        let size = heights.size();
        let overview = heights.overview_dims();
        let paint = paint.overview_dims();
        let mut step = 1;
        while texels(overview.0.max(overview.1), step) > MAX_TEXTURE_SIDE {
            step *= 2;
        }
        let mut layout = AtlasLayout {
            size,
            range: heights.range(),
            tiles: size.tiles(),
            overview_step: step,
            heights_overview: (texels(overview.0, step), texels(overview.1, step)),
            paint_overview: (texels(paint.0, step), texels(paint.1, step)),
            canopy: (0, 0),
        };
        let texel = layout.canopy_texel().0;
        let ground = size.meters();
        layout.canopy = (
            (ground.width / texel).ceil().max(1.0) as u32,
            (ground.height / texel).ceil().max(1.0) as u32,
        );
        layout
    }
}

/// The GPU's textures of a terrain's atlas, laid out for the ground last laid.
#[derive(Component)]
pub struct GpuTerrainAtlas {
    pub layout: AtlasLayout,
    textures: Vec<(AtlasTexture, Texture, TextureView)>,
}

impl GpuTerrainAtlas {
    pub fn view(&self, texture: AtlasTexture) -> &TextureView {
        self.textures
            .iter()
            .find(|(which, ..)| *which == texture)
            .map(|(_, _, view)| view)
            .expect("a view of every texture of the atlas")
    }

    fn texture(&self, texture: AtlasTexture) -> &Texture {
        self.textures
            .iter()
            .find(|(which, ..)| *which == texture)
            .map(|(_, texture, _)| texture)
            .expect("every texture of the atlas")
    }
}

/// Marks a world whose atlases go to a GPU. Without one (a world stepped in a test) nothing is
/// read or written, and every tile wanted is loaded and counts as held at once.
#[derive(Resource)]
struct AtlasesDrawn;

/// The largest side of a texture every GPU holds, as wgpu's default limits guarantee.
const MAX_TEXTURE_SIDE: u32 = 8192;

/// How many slots are read at once, so what waits to be written stays a few frames' worth.
const FILLS_AT_ONCE: usize = 4;

/// The overview's texels a canopy texel spans each way: the far field thins by the grain at
/// hundreds of metres, which a texel this coarse still follows.
const CANOPY_STRIDE: u32 = 4;

/// The rows of the canopy written at once, so each write fits a frame's budget.
const CANOPY_ROWS_PER_WRITE: u32 = 64;

/// The bytes a slot's heights, paint, water and shore take.
const SLOT_BYTES: usize = (SLOT_SIDE * SLOT_SIDE) as usize * 6;

/// The samples past a slot that its shore is read across, so a slot's shore meets its
/// neighbours'.
const SHORE_MARGIN: u32 = (SHORE_REACH.0 * SAMPLES_PER_METER as f32) as u32;

/// The grids of the ground the atlas holds, shared with the area's.
#[derive(Clone)]
struct AtlasGround {
    heights: Heightfield,
    paint: PaintMap,
    water: WaterSurface,
}

/// A ground laid in the atlas, and what is under way to keep the GPU's in step with it.
struct Laid {
    ground: AtlasGround,
    layout: AtlasLayout,
    residency: Residency<TileAt, TileStamp>,
    /// The versions of each tile's heights, paint and water the overview was read from.
    versions: Vec<[TileVersion; 3]>,
    /// The overview's texels being read, the whole first, in the order they are to be written.
    overview: VecDeque<Job<Vec<AtlasWrite>>>,
    /// The overview's writes read and waiting for the frame's budget, tile by tile.
    overview_writes: VecDeque<AtlasWrite>,
    /// Whether the whole overview has been sent, so the whole ground draws.
    overview_sent: bool,
    /// The slots being read for the tiles wanted, each at the stamp it is read at.
    filling: HashMap<TileAt, (TileStamp, Job<SlotBytes>)>,
    loading: HashMap<TileAt, Task<()>>,
}

impl Laid {
    fn new(ground: AtlasGround, layout: AtlasLayout) -> Laid {
        let tiles: Vec<TileAt> = layout.size.each_tile().collect();
        let versions = tiles.iter().map(|&tile| ground.versions(tile)).collect();
        let reading = ground.clone();
        let whole = Job::spawn(move || reading.overview(layout, &tiles));
        Laid {
            ground,
            layout,
            residency: Residency::new(DETAIL_SLOTS),
            versions,
            overview: VecDeque::from([whole]),
            overview_writes: VecDeque::new(),
            overview_sent: false,
            filling: HashMap::new(),
            loading: HashMap::new(),
        }
    }

    /// Takes `ground` in place of the one held, of the same layout: the overview of every tile
    /// whose values changed is read again.
    fn relay(&mut self, ground: AtlasGround) {
        let changed: Vec<TileAt> = self
            .layout
            .size
            .each_tile()
            .zip(&mut self.versions)
            .filter_map(|(tile, held)| {
                let now = ground.versions(tile);
                (now != *held).then(|| {
                    *held = now;
                    tile
                })
            })
            .collect();
        self.ground = ground;
        if changed.is_empty() {
            return;
        }
        let (reading, layout) = (self.ground.clone(), self.layout);
        self.overview
            .push_back(Job::spawn(move || reading.overview(layout, &changed)));
    }
}

/// A slot's heights, paint, water and shore, as the GPU takes them.
type SlotBytes = [Vec<u8>; 4];

impl Laid {
    /// Holds what is wanted as a GPU would, loaded at once, with nothing read or written.
    fn hold_unseen(&mut self, wanted: &[TileAt]) {
        self.overview.clear();
        self.overview_writes.clear();
        self.overview_sent = true;
        self.residency.next_frame();
        for &tile in wanted.iter().take(DETAIL_SLOTS as usize) {
            let round = AtlasGround::round(tile, self.layout.tiles);
            for near in round.into_iter().flatten() {
                self.ground.load(near);
            }
            let stamp = self.ground.stamp(&round);
            self.residency.want(tile);
            if !self.residency.is_current(tile, &stamp) {
                self.residency.admit(tile, stamp);
            }
        }
    }
}

/// The versions of the values of `tile` and of every tile round it, which its slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TileStamp([[TileVersion; 3]; 9]);

impl AtlasGround {
    fn versions(&self, tile: TileAt) -> [TileVersion; 3] {
        [
            self.heights.version(tile),
            self.paint.version(tile),
            self.water.version(tile),
        ]
    }

    fn is_loaded(&self, tile: TileAt) -> bool {
        self.heights.is_loaded(tile) && self.paint.is_loaded(tile) && self.water.is_loaded(tile)
    }

    /// The tile at `at` and every tile round it on a ground of `tiles` across and down, where
    /// they lie.
    fn round(at: TileAt, tiles: (u32, u32)) -> [Option<TileAt>; 9] {
        let mut round = [None; 9];
        for (index, (dc, dr)) in (-1..=1i64)
            .flat_map(|dr| (-1..=1i64).map(move |dc| (dc, dr)))
            .enumerate()
        {
            let (column, row) = (i64::from(at.column) + dc, i64::from(at.row) + dr);
            let inside =
                (0..i64::from(tiles.0)).contains(&column) && (0..i64::from(tiles.1)).contains(&row);
            round[index] = inside.then_some(TileAt {
                column: column as u32,
                row: row as u32,
            });
        }
        round
    }

    fn stamp(&self, round: &[Option<TileAt>; 9]) -> TileStamp {
        TileStamp(round.map(|tile| tile.map(|tile| self.versions(tile)).unwrap_or_default()))
    }

    /// The heights, paint, water and shore of the slot of `tile`, as the GPU takes them.
    fn slot(&self, tile: TileAt) -> SlotBytes {
        let first = (
            i64::from(tile.column * TILE_PIXELS) - 1,
            i64::from(tile.row * TILE_PIXELS) - 1,
        );
        let margin = i64::from(SHORE_MARGIN);
        let wide = SLOT_SIDE + SHORE_MARGIN * 2;
        let around = (first.0 - margin, first.1 - margin);
        let heights = self.heights.block(around, wide, wide);
        let water = self.water.block(around, wide, wide);
        let spacing = Meters(1.0 / SAMPLES_PER_METER as f32);
        let shore = shore_bytes(&heights, &water, wide as usize, spacing);
        let inner = |values: &[u8], bytes: usize| -> Vec<u8> {
            let (side, wide) = (SLOT_SIDE as usize, wide as usize);
            let skip = SHORE_MARGIN as usize;
            (skip..skip + side)
                .flat_map(|row| {
                    let start = (row * wide + skip) * bytes;
                    values[start..start + side * bytes].iter().copied()
                })
                .collect()
        };
        [
            inner(&levels(&heights), 2),
            self.paint.block(first, SLOT_SIDE, SLOT_SIDE),
            inner(&levels(&water), 2),
            inner(&shore, 1),
        ]
    }

    /// The writes of the overview's texels of `tiles`, or of all of it.
    /// The writes of the overview under each of `tiles`, a tile at a time so each fits a frame's
    /// budget.
    fn overview(&self, layout: AtlasLayout, tiles: &[TileAt]) -> Vec<AtlasWrite> {
        let step = layout.overview_step;
        let mut writes = Vec::new();
        for &tile in tiles {
            writes.extend(overview_write(
                AtlasTexture::OverviewHeights,
                &self.heights,
                tile,
                step,
                levels,
            ));
            writes.extend(overview_write(
                AtlasTexture::OverviewPaint,
                &self.paint,
                tile,
                step,
                <[u8]>::to_vec,
            ));
            writes.extend(overview_write(
                AtlasTexture::OverviewWater,
                &self.water,
                tile,
                step,
                levels,
            ));
            writes.extend(self.overview_shore(tile, step));
        }
        writes
    }

    /// The write of the overview's shore under `tile` at one texel every `step` overview values
    /// each way: read across as far round as the shore reaches, so it meets the shore round it.
    fn overview_shore(&self, tile: TileAt, step: u32) -> Option<AtlasWrite> {
        let values = self.heights.overview_dims();
        let (low, high) = overview_bounds(&self.heights, tile);
        let spacing = Meters((OVERVIEW_STRIDE * step) as f32 / SAMPLES_PER_METER as f32);
        let margin = (SHORE_REACH.0 / spacing.0).ceil() as u32;
        let (first_column, columns) = sources(low.0, high.0, values.0, step);
        let (first_row, rows) = sources(low.1, high.1, values.1, step);
        if columns.is_empty() || rows.is_empty() {
            return None;
        }
        let around = |first: u32, count: usize, values: u32| {
            let low = first.saturating_sub(margin);
            let past = (first + count as u32 + margin).min(texels(values, step));
            let source = |texel: u32| (texel * step).min(values.saturating_sub(1));
            (first - low, (low..past).map(source).collect::<Vec<u32>>())
        };
        let (skip_columns, wide_columns) = around(first_column, columns.len(), values.0);
        let (skip_rows, wide_rows) = around(first_row, rows.len(), values.1);
        let heights = self.heights.overview_at(&wide_columns, &wide_rows);
        let water = self.water.overview_at(&wide_columns, &wide_rows);
        let shore = shore_bytes(&heights, &water, wide_columns.len(), spacing);
        let bytes = (0..rows.len())
            .flat_map(|row| {
                let start = (row + skip_rows as usize) * wide_columns.len() + skip_columns as usize;
                shore[start..start + columns.len()].iter().copied()
            })
            .collect();
        Some(AtlasWrite::Texels {
            texture: AtlasTexture::OverviewShore,
            origin: UVec3::new(first_column, first_row, 0),
            size: UVec2::new(columns.len() as u32, rows.len() as u32),
            bytes,
        })
    }
}

/// How far each of a lattice of points `columns` across lies from the nearest dry ground, as the
/// shore's texels hold it, the ground's and the water's levels at each given, the points lying
/// `spacing` apart.
fn shore_bytes(heights: &[u16], water: &[WaterLevel], columns: usize, spacing: Meters) -> Vec<u8> {
    let wet: Vec<bool> = heights
        .iter()
        .zip(water)
        .map(|(ground, surface)| surface.level() > *ground)
        .collect();
    shore_distances(&wet, columns, spacing, SHORE_REACH)
        .into_iter()
        .map(|distance| (distance.0 / SHORE_REACH.0 * 255.0).round() as u8)
        .collect()
}

/// The overview values under `tile`, or all of them: from the first, and past the last, each way.
fn overview_bounds<T: GroundValue>(grid: &GroundGrid<T>, tile: TileAt) -> ((u32, u32), (u32, u32)) {
    let (first, columns, rows) = grid.overview_span(tile);
    (first, (first.0 + columns, first.1 + rows))
}

/// The write of `texture`'s texels of `grid`'s overview under `tile` at one texel every `step`
/// values each way.
fn overview_write<T: GroundValue>(
    texture: AtlasTexture,
    grid: &GroundGrid<T>,
    tile: TileAt,
    step: u32,
    bytes: fn(&[T]) -> Vec<u8>,
) -> Option<AtlasWrite> {
    let values = grid.overview_dims();
    let (low, high) = overview_bounds(grid, tile);
    let (first_column, columns) = sources(low.0, high.0, values.0, step);
    let (first_row, rows) = sources(low.1, high.1, values.1, step);
    if columns.is_empty() || rows.is_empty() {
        return None;
    }
    Some(AtlasWrite::Texels {
        texture,
        origin: UVec3::new(first_column, first_row, 0),
        size: UVec2::new(columns.len() as u32, rows.len() as u32),
        bytes: bytes(&grid.overview_at(&columns, &rows)),
    })
}

/// What the main world asks the GPU to write into the atlas, in order.
enum AtlasWrite {
    /// The atlas's textures made anew for a ground of `layout`, blank.
    Lay(AtlasLayout),
    /// `bytes` written into `texture` from `origin`, its layer in `z`, `size` texels across and
    /// down.
    Texels {
        texture: AtlasTexture,
        origin: UVec3,
        size: UVec2,
        bytes: Vec<u8>,
    },
}

impl AtlasWrite {
    fn bytes(&self) -> usize {
        match self {
            AtlasWrite::Lay(_) => 0,
            AtlasWrite::Texels { bytes, .. } => bytes.len(),
        }
    }
}

/// Work on a worker, and what it came to once taken back.
enum Job<T> {
    Working(Task<T>),
    Done(T),
    Taken,
}

impl<T: Send + 'static> Job<T> {
    fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Job<T> {
        Job::Working(AsyncComputeTaskPool::get().spawn(async move { work() }))
    }

    /// Whether the work is done, taken back as `handoff` takes work.
    fn is_done(&mut self, handoff: Handoff) -> bool {
        if matches!(self, Job::Working(_))
            && let Job::Working(task) = std::mem::replace(self, Job::Taken)
        {
            *self = match handoff.take(task) {
                Ok(done) => Job::Done(done),
                Err(task) => Job::Working(task),
            };
        }
        matches!(self, Job::Done(_))
    }

    fn take(&mut self) -> Option<T> {
        match std::mem::replace(self, Job::Taken) {
            Job::Done(done) => Some(done),
            other => {
                *self = other;
                None
            }
        }
    }
}

/// Keeps the GPU's atlases in step with the grounds laid and the tiles wanted this frame, within
/// the frame's budget of uploads.
fn keep_atlases(
    mut atlases: Query<&mut TerrainAtlas>,
    mut budget: UploadBudget,
    handoff: Option<Res<Handoff>>,
    drawn: Option<Res<AtlasesDrawn>>,
) {
    let handoff = handoff.as_deref().copied().unwrap_or_default();
    for mut atlas in &mut atlases {
        let atlas = &mut *atlas;
        let Some(laid) = &mut atlas.laid else {
            continue;
        };
        let wanted = &atlas.wanted;
        if drawn.is_none() {
            laid.hold_unseen(wanted);
            atlas.writes.clear();
            continue;
        }
        while let Some(job) = laid.overview.front_mut() {
            if !job.is_done(handoff) {
                break;
            }
            let writes = job.take().unwrap_or_default();
            laid.overview_sent |= writes.is_empty();
            laid.overview_writes.extend(writes);
            laid.overview.pop_front();
        }
        while let Some(write) = laid.overview_writes.front() {
            if !budget.spend(write.bytes()) {
                break;
            }
            atlas.writes.extend(laid.overview_writes.pop_front());
            laid.overview_sent |= laid.overview_writes.is_empty();
        }
        laid.loading.retain(|_, task| !task.is_finished());
        laid.residency.next_frame();
        for &tile in wanted.iter().take(DETAIL_SLOTS as usize) {
            let round = AtlasGround::round(tile, laid.layout.tiles);
            let mut loaded = true;
            for near in round.into_iter().flatten() {
                if laid.ground.is_loaded(near) {
                    continue;
                }
                loaded = false;
                laid.loading.entry(near).or_insert_with(|| {
                    let ground = laid.ground.clone();
                    IoTaskPool::get().spawn(async move { ground.load(near) })
                });
            }
            let shown = laid.residency.want(tile).is_some();
            if !loaded {
                continue;
            }
            let stamp = laid.ground.stamp(&round);
            if laid.residency.is_current(tile, &stamp) {
                continue;
            }
            if !shown && !laid.residency.has_room(tile)
                || !laid.filling.contains_key(&tile) && laid.filling.len() >= FILLS_AT_ONCE
            {
                continue;
            }
            let (filling, job) = laid.filling.entry(tile).or_insert_with(|| {
                let ground = laid.ground.clone();
                (stamp, Job::spawn(move || ground.slot(tile)))
            });
            if *filling != stamp {
                let ground = laid.ground.clone();
                *filling = stamp;
                *job = Job::spawn(move || ground.slot(tile));
            }
            if !job.is_done(handoff) || !budget.spend(SLOT_BYTES) {
                continue;
            }
            let Some([heights, paint, water, shore]) = job.take() else {
                continue;
            };
            laid.filling.remove(&tile);
            let Some(admitted) = laid.residency.admit(tile, stamp) else {
                continue;
            };
            let slot = admitted.slot;
            for (texture, bytes) in [
                (AtlasTexture::Heights, heights),
                (AtlasTexture::Paint, paint),
                (AtlasTexture::Water, water),
                (AtlasTexture::Shore, shore),
            ] {
                atlas.writes.push(AtlasWrite::Texels {
                    texture,
                    origin: UVec3::new(0, 0, slot),
                    size: UVec2::splat(SLOT_SIDE),
                    bytes,
                });
            }
            for (at, value) in admitted
                .evicted
                .map(|evicted| (evicted, 0))
                .into_iter()
                .chain([(tile, slot + 1)])
            {
                atlas.writes.push(AtlasWrite::Texels {
                    texture: AtlasTexture::Table,
                    origin: UVec3::new(at.column, at.row, 0),
                    size: UVec2::ONE,
                    bytes: value.to_le_bytes().to_vec(),
                });
            }
        }
        laid.filling.retain(|tile, _| wanted.contains(tile));
    }
}

impl AtlasGround {
    fn load(&self, tile: TileAt) {
        for problem in [
            self.heights.load(tile).err(),
            self.paint.load(tile).err(),
            self.water.load(tile).err(),
        ]
        .into_iter()
        .flatten()
        {
            warn!("ground tile {} doesn't load: {problem}", tile.key());
        }
    }
}

/// The writes the render world is yet to make into a terrain's atlas.
#[derive(Component, Default)]
struct AtlasWrites(Vec<AtlasWrite>);

fn extract_writes(mut main: ResMut<MainWorld>, mut commands: Commands) {
    let main: &mut World = &mut main;
    let mut atlases = main.query::<(&RenderEntity, &mut TerrainAtlas)>();
    for (render, mut atlas) in atlases.iter_mut(main) {
        if atlas.writes.is_empty() {
            continue;
        }
        let writes = std::mem::take(&mut atlas.bypass_change_detection().writes);
        commands.entity(render.id()).insert(AtlasWrites(writes));
    }
}

fn write_atlas(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut atlases: Query<(Entity, &mut AtlasWrites, Option<&GpuTerrainAtlas>)>,
    mut uploaded: ResMut<UploadedBytes>,
) {
    for (entity, mut writes, held) in &mut atlases {
        let mut laid: Option<GpuTerrainAtlas> = None;
        for write in writes.0.drain(..) {
            match write {
                AtlasWrite::Lay(layout) => laid = Some(GpuTerrainAtlas::new(&device, layout)),
                AtlasWrite::Texels {
                    texture,
                    origin,
                    size,
                    bytes,
                } => {
                    let Some(atlas) = laid.as_ref().or(held) else {
                        continue;
                    };
                    atlas.write(&queue, texture, origin, size, &bytes);
                    uploaded.0 += bytes.len() as u64;
                }
            }
        }
        if let Some(laid) = laid {
            commands.entity(entity).insert(laid);
        }
    }
}

impl GpuTerrainAtlas {
    /// Writes `bytes` into `texture` from `origin`, its layer in `z`, `size` texels across and
    /// down.
    fn write(
        &self,
        queue: &RenderQueue,
        texture: AtlasTexture,
        origin: UVec3,
        size: UVec2,
        bytes: &[u8],
    ) {
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: self.texture(texture),
                mip_level: 0,
                origin: Origin3d {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z,
                },
                aspect: TextureAspect::All,
            },
            bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.x * texture.texel_bytes()),
                rows_per_image: Some(size.y),
            },
            Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
        );
    }

    fn new(device: &RenderDevice, layout: AtlasLayout) -> GpuTerrainAtlas {
        let textures = [
            AtlasTexture::Heights,
            AtlasTexture::Paint,
            AtlasTexture::Water,
            AtlasTexture::Shore,
            AtlasTexture::OverviewHeights,
            AtlasTexture::OverviewPaint,
            AtlasTexture::OverviewWater,
            AtlasTexture::OverviewShore,
            AtlasTexture::OverviewCanopy,
            AtlasTexture::OverviewCanopyExtent,
            AtlasTexture::Table,
        ]
        .into_iter()
        .map(|which| {
            let (width, height, layers) = which.extent(layout);
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("terrain atlas"),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: which.format(),
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(if layers > 1 {
                    TextureViewDimension::D2Array
                } else {
                    TextureViewDimension::D2
                }),
                ..default()
            });
            (which, texture, view)
        })
        .collect();
        GpuTerrainAtlas { layout, textures }
    }
}

impl AtlasTexture {
    fn format(self) -> TextureFormat {
        match self {
            AtlasTexture::Paint
            | AtlasTexture::OverviewPaint
            | AtlasTexture::Shore
            | AtlasTexture::OverviewShore => TextureFormat::R8Uint,
            AtlasTexture::OverviewCanopy => TextureFormat::Rgba8UnormSrgb,
            AtlasTexture::OverviewCanopyExtent => TextureFormat::R16Unorm,
            AtlasTexture::Table => TextureFormat::R32Uint,
            _ => TextureFormat::R16Uint,
        }
    }

    fn texel_bytes(self) -> u32 {
        match self.format() {
            TextureFormat::R8Uint => 1,
            TextureFormat::R32Uint | TextureFormat::Rgba8UnormSrgb => 4,
            _ => 2,
        }
    }

    /// The texels across and down and the layers of the texture over a ground of `layout`.
    fn extent(self, layout: AtlasLayout) -> (u32, u32, u32) {
        match self {
            AtlasTexture::Heights
            | AtlasTexture::Paint
            | AtlasTexture::Water
            | AtlasTexture::Shore => (SLOT_SIDE, SLOT_SIDE, DETAIL_SLOTS),
            AtlasTexture::OverviewHeights
            | AtlasTexture::OverviewWater
            | AtlasTexture::OverviewShore => {
                (layout.heights_overview.0, layout.heights_overview.1, 1)
            }
            AtlasTexture::OverviewPaint => (layout.paint_overview.0, layout.paint_overview.1, 1),
            AtlasTexture::OverviewCanopy | AtlasTexture::OverviewCanopyExtent => {
                (layout.canopy.0, layout.canopy.1, CANOPY_BANDS.len() as u32)
            }
            AtlasTexture::Table => (layout.tiles.0, layout.tiles.1, 1),
        }
    }
}

/// The texels a texture keeps of `values` overview values at one texel every `step`: the first
/// and the last value always.
fn texels(values: u32, step: u32) -> u32 {
    values.saturating_sub(1).div_ceil(step) + 1
}

/// The first texel of those whose overview values lie from `low` up to `high` along one way of
/// `values` at `step`, and the overview value each is read from.
fn sources(low: u32, high: u32, values: u32, step: u32) -> (u32, Vec<u32>) {
    let count = texels(values, step);
    let first = low.div_ceil(step).min(count);
    let past = if high >= values {
        count
    } else {
        (high.saturating_sub(1) / step + 1).min(count)
    };
    let source = |texel: u32| (texel * step).min(values.saturating_sub(1));
    (first, (first..past.max(first)).map(source).collect())
}

/// The bytes of `values`' levels, as the GPU reads 16-bit texels.
fn levels<T: Level>(values: &[T]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.level().to_le_bytes())
        .collect()
}
