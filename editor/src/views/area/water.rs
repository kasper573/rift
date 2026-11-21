//! Laying water over an area's ground. A flood fills the basin under the pointer up to a level,
//! stopping at its rim the way poured water would, a tile of ground at a time so a wide basin
//! floods over the frames it takes; the other modes are strokes of the brush over water that
//! already lies.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use engine::core::ground::{
    GroundSize, HeightRange, Heightfield, SAMPLES_PER_METER, TILE_PIXELS, TileAt, WaterSurface,
};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::water::WaterId;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};

use super::brush::{Stroke, StrokeKind, next_revision, water_index};

/// How far over the ground under the pointer a flood's level stands until one is set.
pub const OVER_THE_GROUND: Meters = Meters(1.0);

/// What the Water tool does where it is used.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, VariantArray, AsRefStr,
)]
#[strum(serialize_all = "lowercase")]
pub enum WaterMode {
    /// Fills the basin under the pointer up to the level.
    #[default]
    Flood,
    /// Lifts the level of the water under the brush.
    Raise,
    /// Sinks it.
    Lower,
    /// Leaves the ground under the brush dry.
    Drain,
    /// Lays another water where water lies under the brush, at the level it has.
    Repaint,
}

impl WaterMode {
    /// What the mode does, for whoever picks it.
    pub fn hint(self) -> &'static str {
        match self {
            WaterMode::Flood => "Fills the basin under the pointer up to the level.",
            WaterMode::Raise => "Lifts the level of the water under the brush.",
            WaterMode::Lower => "Sinks the level of the water under the brush.",
            WaterMode::Drain => "Leaves the ground under the brush dry.",
            WaterMode::Repaint => "Lays this water where water lies under the brush, at its level.",
        }
    }
}

/// What the Water tool is set to in a view, and the flood it shows or lays.
#[derive(Default)]
pub struct WaterTool {
    /// The water file the tool lays, once one is picked.
    pub file: Option<String>,
    pub mode: WaterMode,
    /// The level a flood fills to, once one is set: until then a flood stands
    /// [`OVER_THE_GROUND`] over what lies under the pointer.
    pub level: Option<Meters>,
    /// Whether the next click takes the level of the water it lands on.
    pub picking_level: bool,
    /// The flood shown from under the pointer before it is laid, and spreading once it is.
    pub flood: Option<Flood>,
    /// Why nothing floods where the pointer is.
    pub refused: Option<FloodRefused>,
    /// Whether the flood shown is being laid: it lands in the area's files once it stands whole.
    pub laying: bool,
}

impl WaterTool {
    /// Lets go of the flood and of a level being picked: whether there was either.
    pub fn cancel(&mut self) -> bool {
        self.refused = None;
        self.flood.take().is_some()
            | std::mem::take(&mut self.laying)
            | std::mem::take(&mut self.picking_level)
    }
}

/// Why a flood has nothing to fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloodRefused {
    /// The ground where it would start stands at `ground`, at or above the level.
    AboveTheLevel { ground: Meters },
    /// Water already stands where it would start, its surface at `surface`, at or above the
    /// level.
    AlreadyUnder { surface: Meters },
    /// The area already names every water it can.
    NoRoomForAWater,
}

/// The sides of an area a flood ran out over, where its basin has no rim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenSides {
    pub north: bool,
    pub east: bool,
    pub south: bool,
    pub west: bool,
}

impl OpenSides {
    /// The sides the flood left the area by, as a designer names them.
    pub fn names(self) -> Vec<&'static str> {
        [
            (self.north, "north"),
            (self.east, "east"),
            (self.south, "south"),
            (self.west, "west"),
        ]
        .into_iter()
        .filter_map(|(open, name)| open.then_some(name))
        .collect()
    }
}

/// A flood spreading from where it began: the ground as it has flooded it so far, what is left
/// to flood, and how much water it has laid.
pub struct Flood {
    pub stroke: Stroke,
    pub water: WaterId,
    pub from: Pos<Meters>,
    /// The level asked for.
    pub level: Meters,
    /// The level as the area's files hold it: the nearest step of its range of heights.
    held: Meters,
    /// The ground flooded so far, in square metres.
    pub flooded: f32,
    pub deepest: Meters,
    pub open: OpenSides,
    index: u8,
    seen: Vec<u64>,
    seeds: HashMap<TileAt, Vec<(u32, u32)>>,
    waiting: VecDeque<TileAt>,
    reached: HashSet<TileAt>,
}

impl Flood {
    /// A flood of `water` up to `level`, from the ground of `area` at `at`.
    pub fn begin(
        area: &AreaDef,
        at: Pos<Meters>,
        water: WaterId,
        level: Meters,
    ) -> Result<Flood, FloodRefused> {
        let what = StrokeKind::Water {
            water,
            mode: WaterMode::Flood,
        };
        let mut stroke = Stroke::begin(area, at, what).ok_or(FloodRefused::NoRoomForAWater)?;
        let index = water_index(&mut stroke.waters, water).ok_or(FloodRefused::NoRoomForAWater)?;
        let (columns, rows) = stroke.heights.samples();
        let first = Heightfield::sample_pos(0, 0);
        let nearest = |along: f32, samples: u32| {
            ((along * SAMPLES_PER_METER as f32).round().max(0.0) as u32).min(samples - 1)
        };
        let from = (
            nearest(at.x - first.x, columns),
            nearest(at.y - first.y, rows),
        );
        let stroke_range = stroke.water.range();
        let mut flood = Flood {
            stroke,
            water,
            from: at,
            level,
            held: held_level(stroke_range, level),
            flooded: 0.0,
            deepest: Meters(0.0),
            open: OpenSides::default(),
            index,
            seen: vec![0; (columns as usize * rows as usize).div_ceil(64)],
            seeds: HashMap::new(),
            waiting: VecDeque::new(),
            reached: HashSet::new(),
        };
        let tile = flood.tile_of(from);
        flood.load(tile);
        let ground = flood.stroke.heights.sample(from.0, from.1);
        if ground >= flood.held {
            return Err(FloodRefused::AboveTheLevel { ground });
        }
        let surface = flood.stroke.water.sample(from.0, from.1);
        if surface >= flood.held {
            return Err(FloodRefused::AlreadyUnder { surface });
        }
        flood.seed(from);
        Ok(flood)
    }

    /// Whether the flood stands whole, with nothing left to spread to.
    pub fn is_whole(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Whether the flood has laid its water over the ground at `at`.
    pub fn floods(&self, at: Pos<Meters>) -> bool {
        let (columns, rows) = self.stroke.heights.samples();
        let first = Heightfield::sample_pos(0, 0);
        let column = ((at.x - first.x) * SAMPLES_PER_METER as f32).round();
        let row = ((at.y - first.y) * SAMPLES_PER_METER as f32).round();
        if column < 0.0 || row < 0.0 || column as u32 >= columns || row as u32 >= rows {
            return false;
        }
        let sample = (column as u32, row as u32);
        self.saw(sample)
            && self.stroke.heights.sample(sample.0, sample.1) < self.held
            && self.stroke.water.sample(sample.0, sample.1) == self.held
    }

    /// The tiles of ground the flood is done with, of those it has reached.
    pub fn tiles(&self) -> (usize, usize) {
        let waiting: HashSet<&TileAt> = self.waiting.iter().collect();
        (self.reached.len() - waiting.len(), self.reached.len())
    }

    /// Spreads on, a tile of ground at a time for as long as `go_on` says after each, until
    /// nothing is left to flood: whether the flood stands whole.
    pub fn spread(&mut self, mut go_on: impl FnMut() -> bool) -> bool {
        while let Some(tile) = self.waiting.pop_front() {
            self.flood_tile(tile);
            self.stroke.revision = next_revision();
            if !go_on() {
                break;
            }
        }
        self.is_whole()
    }

    /// Floods all of `tile` the flood has come to, and leaves what it comes to beyond the tile
    /// for that ground's turn.
    fn flood_tile(&mut self, tile: TileAt) {
        self.load(tile);
        let Some(mut reach) = self.seeds.remove(&tile) else {
            return;
        };
        let (columns, rows) = self.stroke.heights.samples();
        let (pixels_across, pixels_down) = self.stroke.water_map.pixels();
        let level = self.held;
        let heights = self.stroke.heights.clone();
        let mut ground = heights.reader();
        let water = Arc::make_mut(&mut self.stroke.water);
        let named = Arc::make_mut(&mut self.stroke.water_map);
        let cell = 1.0 / (SAMPLES_PER_METER * SAMPLES_PER_METER) as f32;
        while let Some((column, row)) = reach.pop() {
            let bit = row as usize * columns as usize + column as usize;
            if self.seen[bit / 64] & (1 << (bit % 64)) != 0 {
                continue;
            }
            self.seen[bit / 64] |= 1 << (bit % 64);
            let stands = ground.sample(column, row);
            let surface = water.sample(column, row);
            if surface >= level && surface > stands {
                continue;
            }
            if surface < level {
                water.set_sample(column, row, level);
            }
            if stands >= level {
                continue;
            }
            self.flooded += cell;
            self.deepest = Meters(self.deepest.0.max(level.0 - stands.0));
            for (across, down) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                if let (Some(across), Some(down)) =
                    (column.checked_sub(across), row.checked_sub(down))
                    && across < pixels_across
                    && down < pixels_down
                {
                    named.set(across, down, self.index);
                }
            }
            self.open.west |= column == 0;
            self.open.north |= row == 0;
            self.open.east |= column + 1 == columns;
            self.open.south |= row + 1 == rows;
            let neighbours = [
                column.checked_sub(1).map(|column| (column, row)),
                (column + 1 < columns).then_some((column + 1, row)),
                row.checked_sub(1).map(|row| (column, row)),
                (row + 1 < rows).then_some((column, row + 1)),
            ];
            for next in neighbours.into_iter().flatten() {
                let bit = next.1 as usize * columns as usize + next.0 as usize;
                if self.seen[bit / 64] & (1 << (bit % 64)) != 0 {
                    continue;
                }
                let holding = tile_holding(&heights, next);
                if holding == tile {
                    reach.push(next);
                } else {
                    self.seeds.entry(holding).or_default().push(next);
                    if !self.waiting.contains(&holding) {
                        self.waiting.push_back(holding);
                    }
                    self.reached.insert(holding);
                }
            }
        }
    }

    fn seed(&mut self, from: (u32, u32)) {
        let tile = self.tile_of(from);
        self.seeds.entry(tile).or_default().push(from);
        self.waiting.push_back(tile);
        self.reached.insert(tile);
    }

    fn saw(&self, (column, row): (u32, u32)) -> bool {
        let (columns, _) = self.stroke.heights.samples();
        let bit = row as usize * columns as usize + column as usize;
        self.seen[bit / 64] & (1 << (bit % 64)) != 0
    }

    fn tile_of(&self, sample: (u32, u32)) -> TileAt {
        tile_holding(&self.stroke.heights, sample)
    }

    /// Reads the ground and the water of `tile` from their files, where they haven't been: a
    /// flood told of ground by its overview would stop at rims that aren't there.
    fn load(&self, tile: TileAt) {
        for problem in [
            self.stroke.heights.load(tile).err(),
            self.stroke.water.load(tile).err(),
        ]
        .into_iter()
        .flatten()
        {
            bevy::log::warn!("ground tile {} doesn't load: {problem}", tile.key());
        }
    }
}

/// `level` as a water's surface holds it over ground of `range`: the nearest step of the range.
fn held_level(range: HeightRange, level: Meters) -> Meters {
    let side = engine::core::ground::MIN_SIDE;
    let mut surface = WaterSurface::dry(
        GroundSize {
            width: side,
            height: side,
        },
        range,
    );
    surface.set_sample(0, 0, level);
    surface.sample(0, 0)
}

/// The tile of `heights` whose turn floods sample `(column, row)`.
fn tile_holding(heights: &Heightfield, (column, row): (u32, u32)) -> TileAt {
    let (across, down) = heights.size().tiles();
    TileAt {
        column: (column / TILE_PIXELS).min(across.saturating_sub(1)),
        row: (row / TILE_PIXELS).min(down.saturating_sub(1)),
    }
}
