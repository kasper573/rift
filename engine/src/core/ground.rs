//! The grids an area's ground is made of: its size in whole metres, the height of the ground at
//! every half metre, and which terrain paints it there. The renderer samples them, the loader
//! checks them, and the server reads slopes and blocking paint from them.

use std::io::Cursor;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{ContentProblem, RowCheck};
use crate::core::geometry::Degrees;
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{CellPos, GROUND_ORIGIN, GridSize, MeterSize, Meters};

/// The ground is held whole, its height and paint maps with it, so an area spans at most this.
pub const MAX_SIDE: u32 = 4096;
pub const MIN_SIDE: u32 = 8;
pub const MAX_CELLS: u32 = MAX_SIDE * MAX_SIDE;
/// The terrains one area's paint can name: a byte per half metre.
pub const MAX_TERRAINS: usize = 256;
pub const SAMPLES_PER_METER: u32 = 2;
/// The heights a sample can hold, the two ends of its 16 bits.
pub const LOWEST: Meters = Meters(-8.0);
pub const HIGHEST: Meters = Meters(16.0);

/// A stretch of ground in whole metres, as an area's and a blueprint's canvas are measured.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundSize {
    pub width: u32,
    pub height: u32,
}

impl GroundSize {
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

/// The height of the ground at every half metre of an area, as its `height.png` holds it:
/// 16-bit grey from [`LOWEST`] to [`HIGHEST`]. Sample `(2x, 2y)` is the height at the centre of the
/// metre cell `(x, y)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heightfield {
    columns: u32,
    rows: u32,
    heights: Vec<Meters>,
}

impl Heightfield {
    pub fn flat(size: GroundSize) -> Heightfield {
        let (columns, rows) = size.samples();
        Heightfield {
            columns,
            rows,
            heights: vec![Meters(0.0); (columns * rows) as usize],
        }
    }

    /// A heightfield of `size` whose sample `(column, row)` is `height(column, row)`.
    pub fn from_fn(size: GroundSize, height: impl Fn(u32, u32) -> Meters) -> Heightfield {
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

    pub fn decode(size: GroundSize, bytes: &[u8]) -> Result<Heightfield, ContentProblem> {
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
    pub fn resized(&self, size: GroundSize) -> Heightfield {
        let (columns, rows) = (self.columns, self.rows);
        Heightfield::from_fn(size, |column, row| {
            self.sample(column.min(columns - 1), row.min(rows - 1))
        })
    }

    /// Samples across and down.
    pub fn samples(&self) -> (u32, u32) {
        (self.columns, self.rows)
    }

    /// Sets sample `(column, row)` to `height`, kept within the range the file holds.
    pub fn set_sample(&mut self, column: u32, row: u32, height: Meters) {
        if column < self.columns
            && row < self.rows
            && let Some(sample) = self.heights.get_mut((row * self.columns + column) as usize)
        {
            *sample = Meters(height.0.clamp(LOWEST.0, HIGHEST.0));
        }
    }

    /// Where sample `(column, row)` lies on the ground.
    pub fn sample_pos(column: u32, row: u32) -> Pos<Meters> {
        let step = 1.0 / SAMPLES_PER_METER as f32;
        Pos::new(
            GROUND_ORIGIN.x + column as f32 * step,
            GROUND_ORIGIN.y + row as f32 * step,
        )
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

/// Which terrain paints every half metre of an area, as its `paint.png` holds it: a byte per
/// pixel, the index of the terrain in the area's list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paint {
    columns: u32,
    rows: u32,
    indices: Vec<u8>,
}

/// A terrain and how much of a point it covers, as the paint around the point blends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Share {
    pub terrain: u8,
    pub weight: f32,
}

impl Paint {
    /// All the first terrain.
    pub fn plain(size: GroundSize) -> Paint {
        Paint::from_fn(size, |_, _| 0)
    }

    /// A paint of `size` whose pixel `(column, row)` is the terrain `index(column, row)`.
    pub fn from_fn(size: GroundSize, index: impl Fn(u32, u32) -> u8) -> Paint {
        let (columns, rows) = size.paint_pixels();
        let indices = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| index(column, row))
            .collect();
        Paint {
            columns,
            rows,
            indices,
        }
    }

    pub fn decode(size: GroundSize, bytes: &[u8]) -> Result<Paint, ContentProblem> {
        let (columns, rows) = size.paint_pixels();
        let picture = Picture::decode(bytes, columns, rows, Channels::Grey8)?;
        Ok(Paint {
            columns,
            rows,
            indices: picture.bytes,
        })
    }

    /// Whether every pixel is the first terrain, as a ground without a paint part is.
    pub fn is_plain(&self) -> bool {
        self.indices.iter().all(|index| *index == 0)
    }

    pub fn encode(&self) -> Vec<u8> {
        Picture::encode(&self.indices, self.columns, self.rows, Channels::Grey8)
    }

    /// This paint over a ground of `size`, kept from the top-left corner: what reaches past the
    /// new edge is cut, and new ground is the first terrain.
    pub fn resized(&self, size: GroundSize) -> Paint {
        let (columns, rows) = (self.columns, self.rows);
        Paint::from_fn(size, |column, row| {
            if column < columns && row < rows {
                self.index(column, row)
            } else {
                0
            }
        })
    }

    /// Paints pixel `(column, row)` with the terrain `index`.
    pub fn set(&mut self, column: u32, row: u32, index: u8) {
        if column < self.columns
            && row < self.rows
            && let Some(pixel) = self.indices.get_mut((row * self.columns + column) as usize)
        {
            *pixel = index;
        }
    }

    /// Pixels across and down.
    pub fn pixels(&self) -> (u32, u32) {
        (self.columns, self.rows)
    }

    /// The middle of pixel `(column, row)` on the ground.
    pub fn pixel_pos(column: u32, row: u32) -> Pos<Meters> {
        let step = 1.0 / SAMPLES_PER_METER as f32;
        Pos::new(
            GROUND_ORIGIN.x + (column as f32 + 0.5) * step,
            GROUND_ORIGIN.y + (row as f32 + 0.5) * step,
        )
    }

    /// The terrain painting pixel `(column, row)`; the nearest pixel past the edge.
    pub fn index(&self, column: u32, row: u32) -> u8 {
        let column = column.min(self.columns.saturating_sub(1));
        let row = row.min(self.rows.saturating_sub(1));
        self.indices
            .get((row * self.columns + column) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// The terrain painting the half metre under `at`.
    pub fn at(&self, at: Pos<Meters>) -> u8 {
        let (column, row) = self.pixel_at(at);
        self.index(column, row)
    }

    /// The highest terrain index any pixel names.
    pub fn highest(&self) -> u8 {
        self.indices.iter().copied().max().unwrap_or(0)
    }

    /// Every terrain index some pixel names, lowest first.
    pub fn used(&self) -> Vec<u8> {
        let mut seen = [false; MAX_TERRAINS];
        for index in &self.indices {
            seen[usize::from(*index)] = true;
        }
        (0..MAX_TERRAINS)
            .filter(|index| seen[*index])
            .map(|index| index as u8)
            .collect()
    }

    /// The terrains around `at` and how much of the point each covers, blended between the four
    /// pixel middles around it: one share of weight 1 inside a field of one terrain, up to four
    /// along a boundary, so the ground's colour and its cover thin out over a half metre.
    pub fn blend_at(&self, at: Pos<Meters>) -> Vec<Share> {
        let step = 1.0 / SAMPLES_PER_METER as f32;
        let x = (at.x - GROUND_ORIGIN.x) / step - 0.5;
        let y = (at.y - GROUND_ORIGIN.y) / step - 0.5;
        let (column, row) = (x.floor(), y.floor());
        let (fx, fy) = (x - column, y - row);
        let pixel = |dx: f32, dy: f32| {
            let column = (column + dx).max(0.0) as u32;
            let row = (row + dy).max(0.0) as u32;
            self.index(column, row)
        };
        let corners = [
            (pixel(0.0, 0.0), (1.0 - fx) * (1.0 - fy)),
            (pixel(1.0, 0.0), fx * (1.0 - fy)),
            (pixel(0.0, 1.0), (1.0 - fx) * fy),
            (pixel(1.0, 1.0), fx * fy),
        ];
        let mut shares: Vec<Share> = Vec::with_capacity(4);
        for (terrain, weight) in corners {
            if weight <= 0.0 {
                continue;
            }
            match shares.iter_mut().find(|share| share.terrain == terrain) {
                Some(share) => share.weight += weight,
                None => shares.push(Share { terrain, weight }),
            }
        }
        shares
    }

    /// How much of the point `at` the terrain `index` covers, 0 to 1.
    pub fn share_at(&self, at: Pos<Meters>, index: u8) -> f32 {
        self.blend_at(at)
            .into_iter()
            .filter(|share| share.terrain == index)
            .map(|share| share.weight)
            .sum()
    }

    /// The terrain painting most of the metre cell `cell`; of equals, the first.
    pub fn dominant(&self, cell: CellPos) -> u8 {
        let (column, row) = (
            (cell.x.max(0) as u32) * SAMPLES_PER_METER,
            (cell.y.max(0) as u32) * SAMPLES_PER_METER,
        );
        let mut counts = [0u32; MAX_TERRAINS];
        for dy in 0..SAMPLES_PER_METER {
            for dx in 0..SAMPLES_PER_METER {
                counts[usize::from(self.index(column + dx, row + dy))] += 1;
            }
        }
        counts
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|(_, count)| **count)
            .map_or(0, |(index, _)| index as u8)
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
    Grey8,
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
                format!("{columns} × {rows} px, two samples per metre of the ground"),
            ));
        }
        let (want_color, want_depth, expected) = match channels {
            Channels::Grey16 => (
                png::ColorType::Grayscale,
                png::BitDepth::Sixteen,
                "16-bit greyscale",
            ),
            Channels::Grey8 => (
                png::ColorType::Grayscale,
                png::BitDepth::Eight,
                "8-bit greyscale",
            ),
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
            Channels::Grey8 => {
                encoder.set_color(png::ColorType::Grayscale);
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
