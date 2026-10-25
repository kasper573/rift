//! The brushes that shape an area's terrain: sculpting its heights and painting its materials, a
//! stroke at a time. A stroke works on a copy of the terrain the viewport shows as it goes, and
//! lands in the area's files as one step when the button comes up.

use std::sync::Arc;

use engine::core::math::{Offset, Pos};
use engine::core::terrain::{Heightfield, MATERIALS, Paint, SAMPLES_PER_METER};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::AreaDef;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};

use super::nodes;
use crate::app::Editor;
use crate::project::ContentEdit;

pub const SMALLEST_RADIUS: Meters = Meters(0.5);
pub const LARGEST_RADIUS: Meters = Meters(32.0);
/// How many times bigger or smaller a step of the radius keys makes the brush.
pub const RADIUS_STEP: f32 = 1.25;
/// Metres a second a full-strength sculpt moves the ground by.
const SCULPT_RATE: f32 = 6.0;
/// How much of the way toward the neighbours' mean a second of full-strength smoothing goes.
const SMOOTH_RATE: f32 = 4.0;
/// How much of the way toward the anchor a second of full-strength flattening goes.
const FLATTEN_RATE: f32 = 4.0;
/// Full weight a second of full-strength painting lays.
const PAINT_RATE: f32 = 3.0;

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

/// A stroke of a terrain brush in progress: the terrain as the stroke has shaped it so far,
/// numbered at every change, and the height a flatten stroke pulls toward.
pub struct Stroke {
    pub heights: Arc<Heightfield>,
    pub paint: Arc<Paint>,
    pub revision: u64,
    pub anchor: Meters,
    pub what: StrokeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Sculpt(SculptMode),
    Paint { slot: usize },
}

impl Stroke {
    /// A stroke starting on `area`'s terrain at `at`.
    pub fn begin(area: &AreaDef, at: Pos<Meters>, what: StrokeKind) -> Stroke {
        Stroke {
            anchor: area.heights.height_at(at),
            heights: Arc::new(area.heights.clone()),
            paint: Arc::new(area.paint.clone()),
            revision: 0,
            what,
        }
    }

    /// Applies the brush at `at` for `seconds` of stroke.
    pub fn apply(&mut self, brush: &Brush, at: Pos<Meters>, seconds: f32, level: Meters) {
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
            StrokeKind::Paint { slot } => {
                paint(Arc::make_mut(&mut self.paint), brush, slot, at, seconds);
            }
        }
        self.revision += 1;
    }
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

/// Lays the material `slot` under the brush at `at` for `seconds` of stroke, taking its weight
/// from the other materials in proportion.
pub fn paint(paint: &mut Paint, brush: &Brush, slot: usize, at: Pos<Meters>, seconds: f32) {
    let (columns, rows) = paint.pixels();
    let half = 0.5 / SAMPLES_PER_METER as f32;
    for (column, row, distance) in under((columns, rows), brush, at + Offset::new(-half, -half)) {
        let weight = brush.weight(distance) * brush.strength;
        if weight <= 0.0 {
            continue;
        }
        let weights = paint.weights(column, row);
        let laid = (PAINT_RATE * weight * seconds).min(1.0);
        let mut shares = weights.map(|w| f32::from(w) / 255.0);
        let total: f32 = shares.iter().sum();
        if total > 0.0 {
            for share in &mut shares {
                *share /= total;
            }
        }
        for (index, share) in shares.iter_mut().enumerate() {
            *share = if index == slot {
                *share + (1.0 - *share) * laid
            } else {
                *share * (1.0 - laid)
            };
        }
        let mut bytes = [0u8; MATERIALS];
        for (byte, share) in bytes.iter_mut().zip(shares) {
            *byte = (share * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        paint.set_weights(column, row, bytes);
    }
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
    let Some(nodes::SceneBundle::Area(area)) = nodes::bundle_of(editor, bundle) else {
        return false;
    };
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut area = (*area).clone();
    let label = match stroke.what {
        StrokeKind::Sculpt(mode) => format!("{} the ground of {bundle}", title(mode.as_ref())),
        StrokeKind::Paint { slot } => format!(
            "Paint {} on {bundle}",
            project
                .reader()
                .path_of(area.terrain.materials[slot.min(MATERIALS - 1)])
                .map(crate::tabs::file_title)
                .unwrap_or("ground")
        ),
    };
    area.heights = (*stroke.heights).clone();
    area.paint = (*stroke.paint).clone();
    let mut edit = ContentEdit::new(label);
    if let Err(error) = edit.write_content(project, bundle, &area) {
        editor.toast(error);
        return false;
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
