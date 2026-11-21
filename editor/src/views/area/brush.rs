//! The brushes that shape an area's terrain: sculpting its heights, painting its materials and
//! moving the water that lies on it, a stroke at a time. A stroke works on a copy of the terrain
//! the viewport shows as it goes, and lands in the area's files as one step when the button comes
//! up.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use engine::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER, WaterMap, WaterSurface};
use engine::core::math::{Offset, Pos, Rect, Size};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::AreaDef;
use engine::systems::area::TerrainId;
use engine::systems::water::{MAX_WATERS, WaterId};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};

use super::nodes;
use super::water::WaterMode;
use crate::app::Editor;
use crate::project::{ContentEdit, FileBytes};

pub const SMALLEST_RADIUS: Meters = Meters(0.5);
pub const LARGEST_RADIUS: Meters = Meters(32.0);
/// How many times bigger or smaller a step of the radius keys makes the brush.
const RADIUS_STEP: f32 = 1.25;
/// Metres a second a full-strength sculpt moves the ground by.
const SCULPT_RATE: f32 = 6.0;
/// How much of the way toward the neighbours' mean a second of full-strength smoothing goes.
const SMOOTH_RATE: f32 = 4.0;
/// How much of the way toward the anchor a second of full-strength flattening goes.
const FLATTEN_RATE: f32 = 4.0;
/// Metres a second a full-strength stroke moves the level of water by.
const LEVEL_RATE: f32 = 2.0;

/// How wide and hard a brush is, and how its strength falls toward its edge: 0 keeps full
/// strength to the rim, 1 falls off from the centre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub radius: Meters,
    pub strength: f32,
    pub falloff: f32,
}

impl Default for Brush {
    fn default() -> Brush {
        Brush {
            radius: Meters(4.0),
            strength: 0.5,
            falloff: 0.5,
        }
    }
}

impl Brush {
    pub fn grow(&mut self) {
        self.radius = Meters((self.radius.0 * RADIUS_STEP).min(LARGEST_RADIUS.0));
    }

    pub fn shrink(&mut self) {
        self.radius = Meters((self.radius.0 / RADIUS_STEP).max(SMALLEST_RADIUS.0));
    }

    /// How much of the brush's strength lands `distance` from its centre.
    pub fn weight(&self, distance: f32) -> f32 {
        let t = (distance / self.radius.0).clamp(0.0, 1.0);
        if t >= 1.0 {
            return 0.0;
        }
        let soft = (1.0 - t * t).powi(2);
        (1.0 - self.falloff) + self.falloff * soft
    }
}

/// What a sculpt stroke does to the ground under the brush.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, VariantArray, AsRefStr,
)]
#[strum(serialize_all = "lowercase")]
pub enum SculptMode {
    #[default]
    Raise,
    Lower,
    Smooth,
    /// Toward the height where the stroke began.
    Flatten,
    /// Toward a height set on the brush.
    Level,
}

/// Which ground a paint stroke may lay its terrain on, told by the water over it.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, VariantArray, AsRefStr,
)]
pub enum PaintMask {
    #[default]
    #[strum(serialize = "anywhere")]
    Anywhere,
    #[strum(serialize = "under water")]
    UnderWater,
    #[strum(serialize = "dry ground")]
    DryGround,
}

/// A stroke of a terrain brush in progress: the terrain as the stroke has shaped it so far,
/// numbered anew at every change, and the height a flatten stroke pulls toward.
pub struct Stroke {
    pub heights: Arc<Heightfield>,
    pub paint: Arc<PaintMap>,
    pub water: Arc<WaterSurface>,
    pub water_map: Arc<WaterMap>,
    /// The area's terrains, with the one a paint stroke lays added when the area lacked it.
    pub terrains: Vec<TerrainId>,
    /// The area's waters, with the one a water stroke lays added when the area lacked it.
    pub waters: Vec<WaterId>,
    /// Unlike that of any other stroke, and of this one before its last change.
    pub revision: u64,
    pub anchor: Meters,
    pub what: StrokeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Sculpt(SculptMode),
    Paint { terrain: TerrainId, mask: PaintMask },
    Water { water: WaterId, mode: WaterMode },
}

impl Stroke {
    /// A stroke starting on `area`'s ground at `at`; none when the area already names every
    /// terrain or water it can and `what` lays yet another.
    pub fn begin(area: &AreaDef, at: Pos<Meters>, what: StrokeKind) -> Option<Stroke> {
        let mut terrains = area.terrains.clone();
        let mut waters = area.waters.clone();
        match what {
            StrokeKind::Paint { terrain, .. } => {
                terrain_index(&mut terrains, terrain)?;
            }
            StrokeKind::Water { water, .. } => {
                water_index(&mut waters, water)?;
            }
            StrokeKind::Sculpt(_) => {}
        }
        Some(Stroke {
            anchor: area.heights.height_at(at),
            heights: Arc::new(area.heights.clone()),
            paint: Arc::new(area.paint.clone()),
            water: Arc::new(area.water.clone()),
            water_map: Arc::new(area.water_map.clone()),
            terrains,
            waters,
            revision: next_revision(),
            what,
        })
    }

    /// Applies the brush at `at` for `seconds` of stroke.
    pub fn apply(&mut self, brush: &Brush, at: Pos<Meters>, seconds: f32, level: Meters) {
        self.read_under(brush, at);
        match self.what {
            StrokeKind::Sculpt(mode) => {
                let anchor = match mode {
                    SculptMode::Level => level,
                    _ => self.anchor,
                };
                sculpt(
                    Arc::make_mut(&mut self.heights),
                    brush,
                    mode,
                    at,
                    seconds,
                    anchor,
                );
            }
            StrokeKind::Paint { terrain, mask } => {
                let index = terrain_index(&mut self.terrains, terrain).unwrap_or(0);
                let (heights, water) = (&self.heights, &self.water);
                paint(
                    Arc::make_mut(&mut self.paint),
                    brush,
                    index,
                    at,
                    |column, row| {
                        let wet = water.depth_at(heights, PaintMap::pixel_pos(column, row)).0 > 0.0;
                        match mask {
                            PaintMask::Anywhere => true,
                            PaintMask::UnderWater => wet,
                            PaintMask::DryGround => !wet,
                        }
                    },
                );
            }
            StrokeKind::Water { water, mode } => {
                let index = water_index(&mut self.waters, water).unwrap_or(0);
                match mode {
                    WaterMode::Flood => {}
                    WaterMode::Raise | WaterMode::Lower => {
                        let rise = match mode {
                            WaterMode::Lower => -LEVEL_RATE,
                            _ => LEVEL_RATE,
                        };
                        lift(Arc::make_mut(&mut self.water), brush, at, rise * seconds);
                    }
                    WaterMode::Drain => drain(Arc::make_mut(&mut self.water), brush, at),
                    WaterMode::Repaint => {
                        name_water(Arc::make_mut(&mut self.water_map), brush, index, at);
                    }
                }
            }
        }
        self.revision = next_revision();
    }
}

impl Stroke {
    /// Reads the ground under the brush at `at` from its files, wherever it hasn't been: until
    /// then a tile is known by its overview alone, and a stroke would shape what isn't there.
    fn read_under(&self, brush: &Brush, at: Pos<Meters>) {
        let reach = brush.radius.0;
        let under = Rect::new(
            Pos::new(at.x - reach, at.y - reach),
            Size::new(reach * 2.0, reach * 2.0),
        );
        for tile in self.heights.size().tiles_within(under) {
            let read = [
                self.heights.load(tile),
                self.paint.load(tile),
                self.water.load(tile),
                self.water_map.load(tile),
            ];
            for problem in read.into_iter().filter_map(Result::err) {
                bevy::log::warn!("ground tile {} doesn't load: {problem}", tile.key());
            }
        }
    }
}

/// A number no stroke has had yet, so a viewport that drew one stroke's terrain never takes
/// another's for it.
pub fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Moves the heights under the brush at `at` for `seconds` of stroke.
pub fn sculpt(
    heights: &mut Heightfield,
    brush: &Brush,
    mode: SculptMode,
    at: Pos<Meters>,
    seconds: f32,
    anchor: Meters,
) {
    let before = heights.clone();
    for (column, row, distance) in under(heights.samples(), brush, at) {
        let weight = brush.weight(distance) * brush.strength;
        if weight <= 0.0 {
            continue;
        }
        let here = before.sample(column, row).0;
        let moved = match mode {
            SculptMode::Raise => here + SCULPT_RATE * weight * seconds,
            SculptMode::Lower => here - SCULPT_RATE * weight * seconds,
            SculptMode::Smooth => {
                let mean = [
                    before.sample(column.saturating_sub(1), row),
                    before.sample(column + 1, row),
                    before.sample(column, row.saturating_sub(1)),
                    before.sample(column, row + 1),
                ]
                .iter()
                .map(|height| height.0)
                .sum::<f32>()
                    / 4.0;
                here + (mean - here) * (SMOOTH_RATE * weight * seconds).min(1.0)
            }
            SculptMode::Flatten | SculptMode::Level => {
                here + (anchor.0 - here) * (FLATTEN_RATE * weight * seconds).min(1.0)
            }
        };
        heights.set_sample(column, row, Meters(moved));
    }
}

/// Lays the terrain `index` under the brush at `at`, on the pixels `takes` says take it: wholly
/// where the brush is strong, and dithered toward its rim as its falloff weakens, so a painted
/// edge frays instead of cutting.
pub fn paint(
    paint: &mut PaintMap,
    brush: &Brush,
    index: u8,
    at: Pos<Meters>,
    takes: impl Fn(u32, u32) -> bool,
) {
    let (columns, rows) = paint.pixels();
    let half = 0.5 / SAMPLES_PER_METER as f32;
    for (column, row, distance) in under((columns, rows), brush, at + Offset::new(-half, -half)) {
        let weight = brush.weight(distance);
        if weight > 0.0 && weight >= dither(column, row) && takes(column, row) {
            paint.set(column, row, index);
        }
    }
}

/// Moves the level of the water under the brush at `at` by up to `rise` metres, wherever the
/// surface holds a level at all: the dry ground round a water holds the water's level too, so a
/// rising water climbs its shore.
pub fn lift(water: &mut WaterSurface, brush: &Brush, at: Pos<Meters>, rise: f32) {
    let dry = water.range().lowest;
    let before = water.clone();
    for (column, row, distance) in under(water.samples(), brush, at) {
        let weight = brush.weight(distance) * brush.strength;
        let here = before.sample(column, row);
        if weight > 0.0 && here > dry {
            water.set_sample(column, row, Meters(here.0 + rise * weight));
        }
    }
}

/// Leaves the ground under the brush at `at` dry.
pub fn drain(water: &mut WaterSurface, brush: &Brush, at: Pos<Meters>) {
    let dry = water.range().lowest;
    for (column, row, _) in under(water.samples(), brush, at) {
        water.set_sample(column, row, dry);
    }
}

/// Names the water `index` as the one lying under the brush at `at`, whatever its level.
pub fn name_water(waters: &mut WaterMap, brush: &Brush, index: u8, at: Pos<Meters>) {
    let half = 0.5 / SAMPLES_PER_METER as f32;
    for (column, row, _) in under(waters.pixels(), brush, at + Offset::new(-half, -half)) {
        waters.set(column, row, index);
    }
}

/// The index the paint names `terrain` by in `terrains`, listing it when it is new; none when the
/// list is full.
pub fn terrain_index(terrains: &mut Vec<TerrainId>, terrain: TerrainId) -> Option<u8> {
    if let Some(index) = terrains.iter().position(|known| *known == terrain) {
        return u8::try_from(index).ok();
    }
    if terrains.len() >= engine::core::ground::MAX_TERRAINS {
        return None;
    }
    terrains.push(terrain);
    u8::try_from(terrains.len() - 1).ok()
}

/// The index the water map names `water` by in `waters`, listing it when it is new; none when the
/// list is full.
pub fn water_index(waters: &mut Vec<WaterId>, water: WaterId) -> Option<u8> {
    if let Some(index) = waters.iter().position(|known| *known == water) {
        return u8::try_from(index).ok();
    }
    if waters.len() >= MAX_WATERS {
        return None;
    }
    waters.push(water);
    u8::try_from(waters.len() - 1).ok()
}

/// A threshold in 0..1 fixed to the pixel, so a stroke's rim frays the same way however often the
/// brush passes over it.
fn dither(column: u32, row: u32) -> f32 {
    let mut hash = column.wrapping_mul(0x9E37_79B1) ^ row.wrapping_mul(0x85EB_CA77);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2C1B_3C6D);
    hash ^= hash >> 12;
    (hash & 0xFFFF) as f32 / 65536.0
}

/// The samples of a grid two to the metre within the brush at `at`, each with its distance from
/// the brush's centre.
fn under(
    (columns, rows): (u32, u32),
    brush: &Brush,
    at: Pos<Meters>,
) -> impl Iterator<Item = (u32, u32, f32)> {
    let per_meter = SAMPLES_PER_METER as f32;
    let reach = brush.radius.0;
    let first = Heightfield::sample_pos(0, 0);
    let column_range = (((at.x - reach - first.x) * per_meter).floor().max(0.0) as u32)
        ..=(((at.x + reach - first.x) * per_meter).ceil().max(0.0) as u32)
            .min(columns.saturating_sub(1));
    let row_range = (((at.y - reach - first.y) * per_meter).floor().max(0.0) as u32)
        ..=(((at.y + reach - first.y) * per_meter).ceil().max(0.0) as u32)
            .min(rows.saturating_sub(1));
    row_range.flat_map(move |row| {
        column_range.clone().filter_map(move |column| {
            let distance = Heightfield::sample_pos(column, row).distance(at).0;
            (distance <= reach).then_some((column, row, distance))
        })
    })
}

/// Writes the stroke's terrain into the area at `bundle` as one step.
pub fn commit(editor: &mut Editor, bundle: &str, stroke: &Stroke) -> bool {
    let Some(nodes::SceneBundle::Area(before)) = nodes::bundle_of(editor, bundle) else {
        return false;
    };
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut area = (*before).clone();
    let label = match stroke.what {
        StrokeKind::Sculpt(mode) => format!("{} the ground of {bundle}", title(mode.as_ref())),
        StrokeKind::Paint { terrain, .. } => format!(
            "Paint {} on {bundle}",
            project
                .reader()
                .path_of(terrain)
                .map(crate::tabs::file_title)
                .unwrap_or("terrain")
        ),
        StrokeKind::Water { water, mode } => {
            let reader = project.reader();
            let water = reader
                .path_of(water)
                .map(crate::tabs::file_title)
                .unwrap_or("water");
            match mode {
                WaterMode::Flood => format!("Flood {bundle} with {water}"),
                WaterMode::Raise => format!("Raise the water of {bundle}"),
                WaterMode::Lower => format!("Lower the water of {bundle}"),
                WaterMode::Drain => format!("Drain {bundle}"),
                WaterMode::Repaint => format!("Lay {water} on the water of {bundle}"),
            }
        }
    };
    area.heights = (*stroke.heights).clone();
    area.paint = (*stroke.paint).clone();
    area.water = (*stroke.water).clone();
    area.water_map = (*stroke.water_map).clone();
    area.terrains = stroke.terrains.clone();
    area.waters = stroke.waters.clone();
    let changed = area.ground_files_changed(&before, bundle);
    let reader = project.reader();
    let mut edit = ContentEdit::new(label);
    let written = reader
        .write_only(bundle, &area, move |file| changed.contains(file))
        .and_then(|written| {
            let fields = (area.terrains != before.terrains || area.waters != before.waters)
                .then(|| reader.write_fields(bundle, &area))
                .transpose()?;
            Ok((written, fields))
        });
    match written {
        Ok((written, fields)) => {
            for (file, bytes) in written.files.into_iter().chain(fields) {
                edit.write(file, FileBytes::from(bytes));
            }
            for file in written.removed {
                edit.remove(file);
            }
        }
        Err(error) => {
            editor.toast(format!(
                "{} does not write: {error}",
                crate::tabs::shown_path(bundle)
            ));
            return false;
        }
    }
    editor.apply(edit)
}

fn title(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
