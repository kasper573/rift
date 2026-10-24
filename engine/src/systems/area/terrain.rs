use std::io::Cursor;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ground::GroundId;
use crate::core::content::{ContentProblem, PartSpec, RowCheck};
use crate::core::geometry::Degrees;
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{CellPos, GROUND_ORIGIN, GridSize, Meters, TileSize};

pub const MAX_SIDE: u32 = 256;
pub const MIN_SIDE: u32 = 8;
pub const MAX_CELLS: u32 = 65_536;
pub const MIN_SLOPE: Degrees = Degrees(10.0);
pub const MAX_SLOPE: Degrees = Degrees(60.0);
/// The ground materials an area blends: one per channel of its paint.
pub const MATERIALS: usize = 4;
pub const SAMPLES_PER_METER: u32 = 2;
/// The heights a sample can hold, the two ends of its 16 bits.
pub const LOWEST: Meters = Meters(-8.0);
pub const HIGHEST: Meters = Meters(16.0);

pub const HEIGHT: PartSpec = PartSpec::File {
    name: "terrain/height",
    exts: &["png"],
};
pub const PAINT: PartSpec = PartSpec::File {
    name: "terrain/paint",
    exts: &["png"],
};

/// The ground of an area: how far it reaches, the four materials its paint blends, and how steep a
/// slope still walks. Its heights and paint are parts of the area's bundle.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    pub size: TerrainSize,
    pub materials: [GroundId; MATERIALS],
    pub max_slope: Degrees,
}

impl Terrain {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        self.size.check(&format!("{pointer}/size"), MIN_SIDE, cx);
        cx.range(
            format!("{pointer}/max_slope"),
            self.max_slope.0,
            MIN_SLOPE.0,
            MAX_SLOPE.0,
            "degrees of slope",
        );
    }
}

/// A stretch of ground in whole metres, as a terrain's and a blueprint's canvas are measured.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainSize {
    pub width: u32,
    pub height: u32,
}

impl TerrainSize {
    pub fn grid(self) -> GridSize {
        GridSize::new(self.width, self.height)
    }

    pub fn meters(self) -> Size<Meters> {
        Size::new(self.width as f32, self.height as f32)
    }

    pub fn bounds(self) -> Rect<Meters> {
        self.meters().bounds()
    }

    /// Height samples across and down: one every half metre, both edges included.
    pub fn samples(self) -> (u32, u32) {
        (
            self.width * SAMPLES_PER_METER + 1,
            self.height * SAMPLES_PER_METER + 1,
        )
    }

    /// Paint pixels across and down: one every half metre.
    pub fn paint_pixels(self) -> (u32, u32) {
        (
            self.width * SAMPLES_PER_METER,
            self.height * SAMPLES_PER_METER,
        )
    }

    pub fn check(self, pointer: &str, min_side: u32, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/width"),
            self.width,
            min_side,
            MAX_SIDE,
            "m",
        );
        cx.range(
            format!("{pointer}/height"),
            self.height,
            min_side,
            MAX_SIDE,
            "m",
        );
        if self.width.saturating_mul(self.height) > MAX_CELLS {
            cx.error(
                pointer,
                "out of range",
                format!("{} m²", self.width * self.height),
                format!("at most {MAX_CELLS} m²"),
            );
        }
    }
}

/// The height of the ground at every half metre of an area, as its `terrain/height.png` holds it:
/// 16-bit grey from [`LOWEST`] to [`HIGHEST`]. Sample `(2x, 2y)` is the height at the centre of the
/// metre cell `(x, y)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heightfield {
    columns: u32,
    rows: u32,
    heights: Vec<Meters>,
}

impl Heightfield {
    pub fn flat(size: TerrainSize) -> Heightfield {
        let (columns, rows) = size.samples();
        Heightfield {
            columns,
            rows,
            heights: vec![Meters(0.0); (columns * rows) as usize],
        }
    }

    /// A heightfield of `size` whose sample `(column, row)` is `height(column, row)`.
    pub fn from_fn(size: TerrainSize, height: impl Fn(u32, u32) -> Meters) -> Heightfield {
        let (columns, rows) = size.samples();
        let heights = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| height(column, row))
            .collect();
        Heightfield {
            columns,
            rows,
            heights,
        }
    }

    pub fn decode(size: TerrainSize, bytes: &[u8]) -> Result<Heightfield, ContentProblem> {
        let (columns, rows) = size.samples();
        let picture = Picture::decode(bytes, columns, rows, Channels::Grey16)?;
        let heights = picture
            .bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let level = u16::from_be_bytes(*pair) as f32 / u16::MAX as f32;
                Meters(LOWEST.0 + level * (HIGHEST.0 - LOWEST.0))
            })
            .collect();
        Ok(Heightfield {
            columns,
            rows,
            heights,
        })
    }

    /// Whether every sample sits at ground level, as a terrain without a height part does.
    pub fn is_flat(&self) -> bool {
        self.heights.iter().all(|height| height.0 == 0.0)
    }

    pub fn encode(&self) -> Vec<u8> {
        let levels: Vec<u8> = self
            .heights
            .iter()
            .flat_map(|height| {
                let level = (height.0 - LOWEST.0) / (HIGHEST.0 - LOWEST.0);
                ((level.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16).to_be_bytes()
            })
            .collect();
        Picture::encode(&levels, self.columns, self.rows, Channels::Grey16)
    }

    /// This heightfield over a terrain of `size`, kept from the top-left corner: what reaches past
    /// the new edge is cut, and new ground is flat at the old edge's height.
    pub fn resized(&self, size: TerrainSize) -> Heightfield {
        let (columns, rows) = (self.columns, self.rows);
        Heightfield::from_fn(size, |column, row| {
            self.sample(column.min(columns - 1), row.min(rows - 1))
        })
    }

    pub fn sample(&self, column: u32, row: u32) -> Meters {
        let column = column.min(self.columns.saturating_sub(1));
        let row = row.min(self.rows.saturating_sub(1));
        self.heights
            .get((row * self.columns + column) as usize)
            .copied()
            .unwrap_or(Meters(0.0))
    }

    /// The ground's height at `at`, between the samples around it.
    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        let (x, y) = self.sample_space(at);
        let (column, row) = (x.floor(), y.floor());
        let (u, v) = (x - column, y - row);
        let (column, row) = (column.max(0.0) as u32, row.max(0.0) as u32);
        let lerp = |a: Meters, b: Meters, t: f32| Meters(a.0 + (b.0 - a.0) * t);
        let top = lerp(self.sample(column, row), self.sample(column + 1, row), u);
        let bottom = lerp(
            self.sample(column, row + 1),
            self.sample(column + 1, row + 1),
            u,
        );
        lerp(top, bottom, v)
    }

    /// The steepest slope between neighbouring samples under the metre cell `cell`.
    pub fn steepest(&self, cell: CellPos) -> Degrees {
        let (column, row) = (
            (cell.x.max(0) as u32) * SAMPLES_PER_METER,
            (cell.y.max(0) as u32) * SAMPLES_PER_METER,
        );
        let step = 1.0 / SAMPLES_PER_METER as f32;
        let mut steepest = 0.0f32;
        for dy in 0..=SAMPLES_PER_METER {
            for dx in 0..=SAMPLES_PER_METER {
                let here = self.sample(column + dx, row + dy);
                if dx < SAMPLES_PER_METER {
                    let rise = (self.sample(column + dx + 1, row + dy).0 - here.0).abs();
                    steepest = steepest.max(rise / step);
                }
                if dy < SAMPLES_PER_METER {
                    let rise = (self.sample(column + dx, row + dy + 1).0 - here.0).abs();
                    steepest = steepest.max(rise / step);
                }
            }
        }
        Degrees(steepest.atan().to_degrees())
    }

    fn sample_space(&self, at: Pos<Meters>) -> (f32, f32) {
        let x = ((at.x - GROUND_ORIGIN.x) * SAMPLES_PER_METER as f32)
            .clamp(0.0, self.columns.saturating_sub(1) as f32);
        let y = ((at.y - GROUND_ORIGIN.y) * SAMPLES_PER_METER as f32)
            .clamp(0.0, self.rows.saturating_sub(1) as f32);
        (x, y)
    }
}

/// How much of each of the four materials covers every half metre of an area, as its
/// `terrain/paint.png` holds it: one channel per material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paint {
    columns: u32,
    rows: u32,
    weights: Vec<[u8; MATERIALS]>,
}

impl Paint {
    /// All the first material.
    pub fn plain(size: TerrainSize) -> Paint {
        Paint::from_fn(size, |_, _| [255, 0, 0, 0])
    }

    /// A paint of `size` whose pixel `(column, row)` weighs `weights(column, row)`.
    pub fn from_fn(size: TerrainSize, weights: impl Fn(u32, u32) -> [u8; MATERIALS]) -> Paint {
        let (columns, rows) = size.paint_pixels();
        let weights = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| weights(column, row))
            .collect();
        Paint {
            columns,
            rows,
            weights,
        }
    }

    pub fn decode(size: TerrainSize, bytes: &[u8]) -> Result<Paint, ContentProblem> {
        let (columns, rows) = size.paint_pixels();
        let picture = Picture::decode(bytes, columns, rows, Channels::Rgba8)?;
        let weights = picture.bytes.as_chunks::<MATERIALS>().0.to_vec();
        Ok(Paint {
            columns,
            rows,
            weights,
        })
    }

    /// Whether every pixel is wholly the first material, as a terrain without a paint part is.
    pub fn is_plain(&self) -> bool {
        self.weights
            .iter()
            .all(|weights| *weights == [255, 0, 0, 0])
    }

    pub fn encode(&self) -> Vec<u8> {
        let bytes: Vec<u8> = self.weights.iter().flatten().copied().collect();
        Picture::encode(&bytes, self.columns, self.rows, Channels::Rgba8)
    }

    /// This paint over a terrain of `size`, kept from the top-left corner: what reaches past the
    /// new edge is cut, and new ground is the first material.
    pub fn resized(&self, size: TerrainSize) -> Paint {
        let (columns, rows) = (self.columns, self.rows);
        Paint::from_fn(size, |column, row| {
            if column < columns && row < rows {
                self.weights(column, row)
            } else {
                [255, 0, 0, 0]
            }
        })
    }

    pub fn weights(&self, column: u32, row: u32) -> [u8; MATERIALS] {
        let column = column.min(self.columns.saturating_sub(1));
        let row = row.min(self.rows.saturating_sub(1));
        self.weights
            .get((row * self.columns + column) as usize)
            .copied()
            .unwrap_or([255, 0, 0, 0])
    }

    /// The material weighing most over the metre cell `cell`; of equals, the first.
    pub fn dominant(&self, cell: CellPos) -> usize {
        let (column, row) = (
            (cell.x.max(0) as u32) * SAMPLES_PER_METER,
            (cell.y.max(0) as u32) * SAMPLES_PER_METER,
        );
        let mut total = [0u32; MATERIALS];
        for dy in 0..SAMPLES_PER_METER {
            for dx in 0..SAMPLES_PER_METER {
                for (sum, weight) in total.iter_mut().zip(self.weights(column + dx, row + dy)) {
                    *sum += u32::from(weight);
                }
            }
        }
        total
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|(_, weight)| **weight)
            .map_or(0, |(index, _)| index)
    }

    /// The pixel under `at`.
    pub fn pixel_at(&self, at: Pos<Meters>) -> (u32, u32) {
        let column = ((at.x - GROUND_ORIGIN.x) * SAMPLES_PER_METER as f32).floor();
        let row = ((at.y - GROUND_ORIGIN.y) * SAMPLES_PER_METER as f32).floor();
        (
            (column.max(0.0) as u32).min(self.columns.saturating_sub(1)),
            (row.max(0.0) as u32).min(self.rows.saturating_sub(1)),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channels {
    Grey16,
    Rgba8,
}

struct Picture {
    bytes: Vec<u8>,
}

impl Picture {
    fn decode(
        bytes: &[u8],
        columns: u32,
        rows: u32,
        channels: Channels,
    ) -> Result<Picture, ContentProblem> {
        let unreadable = |found: String| ContentProblem::new("unreadable image", found, "a PNG");
        let mut reader = png::Decoder::new(Cursor::new(bytes))
            .read_info()
            .map_err(|error| unreadable(error.to_string()))?;
        let info = reader.info();
        if (info.width, info.height) != (columns, rows) {
            return Err(ContentProblem::new(
                "wrong size",
                format!("{} × {} px", info.width, info.height),
                format!("{columns} × {rows} px, two samples per metre of the terrain"),
            ));
        }
        let (want_color, want_depth, expected) = match channels {
            Channels::Grey16 => (
                png::ColorType::Grayscale,
                png::BitDepth::Sixteen,
                "16-bit greyscale",
            ),
            Channels::Rgba8 => (png::ColorType::Rgba, png::BitDepth::Eight, "8-bit RGBA"),
        };
        if (info.color_type, info.bit_depth) != (want_color, want_depth) {
            return Err(ContentProblem::new(
                "wrong format",
                format!("{:?} {:?}", info.color_type, info.bit_depth),
                expected,
            ));
        }
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| unreadable("an image too large to decode".to_owned()))?;
        let mut pixels = vec![0; size];
        let frame = reader
            .next_frame(&mut pixels)
            .map_err(|error| unreadable(error.to_string()))?;
        pixels.truncate(frame.buffer_size());
        Ok(Picture { bytes: pixels })
    }

    fn encode(bytes: &[u8], columns: u32, rows: u32, channels: Channels) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, columns, rows);
        match channels {
            Channels::Grey16 => {
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(png::BitDepth::Sixteen);
            }
            Channels::Rgba8 => {
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
            }
        }
        let mut writer = encoder
            .write_header()
            .expect("a png header writes to memory");
        writer
            .write_image_data(bytes)
            .expect("png pixels write to memory");
        writer.finish().expect("a png finishes in memory");
        out
    }
}
