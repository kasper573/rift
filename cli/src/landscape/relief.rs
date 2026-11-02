//! The ground's shape, worked out over a lattice coarser than the ground's own samples: warped
//! fractal noise for the hills, ridged noise for the mountain ranges, then rain wearing both down.
//! The ground's samples read it between the lattice's nodes, with the bumps underfoot on top.

use engine::core::ground::{GroundSize, HeightRange, Heightfield, SAMPLES_PER_METER};
use engine::core::math::{Pos, Rng};
use engine::core::units::{Degrees, Meters};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin, RidgedMulti};
use rayon::prelude::*;

use super::Land;

pub struct Relief {
    origin: Pos<Meters>,
    /// The lattice's spacing, in metres.
    spacing: f32,
    columns: usize,
    rows: usize,
    /// Each node's height, in metres.
    heights: Vec<f32>,
    /// How much of the rain ran over each node, from 0 where none did to 1 where most did.
    runoff: Vec<f32>,
    relief: f32,
}

impl Relief {
    /// The land's ground before the rain, its lowest node at 0 and its highest at the relief.
    pub fn shaped(size: GroundSize, land: &Land) -> Relief {
        let wavelength = land.shape.wavelength.0;
        let spacing = (wavelength / NODES_PER_WAVELENGTH).max(1.0 / SAMPLES_PER_METER as f32);
        let bounds = size.bounds();
        let columns = (bounds.size.width / spacing).ceil() as usize + 1;
        let rows = (bounds.size.height / spacing).ceil() as usize + 1;
        let seed = land.seed as u32;
        let hills = Fbm::<Perlin>::new(seed).set_octaves(6);
        let ranges = RidgedMulti::<Perlin>::new(seed.wrapping_add(1)).set_octaves(5);
        let where_ranges = Fbm::<Perlin>::new(seed.wrapping_add(2)).set_octaves(3);
        let bend_x = Fbm::<Perlin>::new(seed.wrapping_add(3)).set_octaves(4);
        let bend_y = Fbm::<Perlin>::new(seed.wrapping_add(4)).set_octaves(4);
        let raw: Vec<f32> = (0..rows * columns)
            .into_par_iter()
            .map(|index| {
                let (x, y) = (
                    (index % columns) as f64 * f64::from(spacing),
                    (index / columns) as f64 * f64::from(spacing),
                );
                let at = [x / f64::from(wavelength), y / f64::from(wavelength)];
                let warp = f64::from(land.shape.warp);
                let broad = [at[0] * 0.5, at[1] * 0.5];
                let bent = [
                    at[0] + warp * bend_x.get(broad),
                    at[1] + warp * bend_y.get(broad),
                ];
                let rolling = unit(hills.get(bent));
                let ridged = unit(ranges.get([bent[0] * 0.5, bent[1] * 0.5]));
                let range = smoothstep(
                    0.35,
                    0.65,
                    unit(where_ranges.get([at[0] * 0.25, at[1] * 0.25])),
                );
                let mountains = f64::from(land.shape.mountains) * range;
                (rolling * (1.0 - mountains) + ridged * mountains) as f32
            })
            .collect();
        let (low, high) = span(&raw);
        let relief = land.relief.0;
        let heights = raw
            .into_iter()
            .map(|height| ((height - low) / (high - low)).powf(LOWLAND_FLATTENING) * relief)
            .collect();
        Relief {
            origin: bounds.origin,
            spacing,
            columns,
            rows,
            heights,
            runoff: vec![0.0; rows * columns],
            relief,
        }
    }

    /// Lets the land's rain fall and run off, carving and silting the ground as it goes.
    ///
    /// Each drop is a particle that runs down the slope under it with some inertia, picks up
    /// silt where it runs faster than its load allows and leaves it where it slows or climbs,
    /// and dries up as it goes (Beyer's particle erosion, as games commonly use it). The drops
    /// fall one after another from the land's seed, so a seed always wears the land alike.
    pub fn erode(&mut self, land: &Land) {
        let width = (self.columns - 1) as f32 * self.spacing;
        let height = (self.rows - 1) as f32 * self.spacing;
        let drops = (land.erosion.drops_per_square_meter * width * height) as u64;
        let mut ground: Vec<f32> = self.heights.iter().map(|h| h / self.spacing).collect();
        let mut runoff = vec![0.0f32; ground.len()];
        let brush = Brush::new(BRUSH_RADIUS, self.columns, self.rows);
        let mut rng = Rng::new(land.seed);
        let (last_x, last_y) = ((self.columns - 1) as f32, (self.rows - 1) as f32);
        for _ in 0..drops {
            let mut drop = Drop::falling(rng.rand_float() * last_x, rng.rand_float() * last_y);
            for _ in 0..DROP_LIFETIME {
                let (column, row) = (drop.x as usize, drop.y as usize);
                let (fx, fy) = (drop.x - column as f32, drop.y - row as f32);
                let before = self.surface(&ground, drop.x, drop.y);
                drop.dx = drop.dx * INERTIA - before.dx * (1.0 - INERTIA);
                drop.dy = drop.dy * INERTIA - before.dy * (1.0 - INERTIA);
                let length = drop.dx.hypot(drop.dy);
                if length < f32::EPSILON {
                    break;
                }
                drop.x += drop.dx / length;
                drop.y += drop.dy / length;
                if !(0.0..last_x).contains(&drop.x) || !(0.0..last_y).contains(&drop.y) {
                    break;
                }
                runoff[row * self.columns + column] += drop.water;
                let rise = self.surface(&ground, drop.x, drop.y).height - before.height;
                let capacity = (-rise * drop.speed * drop.water * SILT_CAPACITY).max(SILT_FLOOR);
                if drop.silt > capacity || rise > 0.0 {
                    let left = if rise > 0.0 {
                        rise.min(drop.silt)
                    } else {
                        (drop.silt - capacity) * SILT_DEPOSIT
                    };
                    drop.silt -= left;
                    let at = row * self.columns + column;
                    ground[at] += left * (1.0 - fx) * (1.0 - fy);
                    ground[at + 1] += left * fx * (1.0 - fy);
                    ground[at + self.columns] += left * (1.0 - fx) * fy;
                    ground[at + self.columns + 1] += left * fx * fy;
                } else {
                    let carved = ((capacity - drop.silt) * SILT_PICKUP).min(-rise);
                    drop.silt += brush.carve(&mut ground, column, row, carved);
                }
                drop.speed = (drop.speed * drop.speed - rise * GRAVITY).max(0.0).sqrt();
                drop.water *= 1.0 - EVAPORATION;
            }
        }
        let (low, high) = span(&ground);
        self.heights = ground
            .into_iter()
            .map(|h| (h - low) / (high - low) * self.relief)
            .collect();
        let most = runoff
            .iter()
            .copied()
            .fold(0.0f32, f32::max)
            .max(f32::EPSILON);
        self.runoff = runoff
            .into_iter()
            .map(|ran| ran.ln_1p() / most.ln_1p())
            .collect();
    }

    /// The ground's samples: the lattice between its nodes and the bumps underfoot, lifted and
    /// stretched to rise over the relief exactly.
    pub fn heightfield(&self, size: GroundSize, range: HeightRange, land: &Land) -> Heightfield {
        let bumps = Fbm::<Perlin>::new((land.seed as u32).wrapping_add(5)).set_octaves(3);
        let bump_wavelength = f64::from(self.spacing * NODES_PER_BUMP);
        let roughness = land.shape.roughness.0;
        let (columns, rows) = size.samples();
        let raw: Vec<f32> = (0..rows * columns)
            .into_par_iter()
            .map(|index| {
                let at = Heightfield::sample_pos(index % columns, index / columns);
                let bump = bumps.get([
                    f64::from(at.x) / bump_wavelength,
                    f64::from(at.y) / bump_wavelength,
                ]) as f32;
                self.height(at) + bump * roughness
            })
            .collect();
        let (low, high) = span(&raw);
        Heightfield::from_fn(size, range, |column, row| {
            Meters((raw[(row * columns + column) as usize] - low) / (high - low) * self.relief)
        })
    }

    /// The ground's height at `at`, from the lowest node, in metres.
    pub fn height(&self, at: Pos<Meters>) -> f32 {
        let (x, y) = self.lattice(at);
        self.surface(&self.heights, x, y).height
    }

    /// The share of the relief the ground at `at` rises to.
    pub fn elevation(&self, at: Pos<Meters>) -> f32 {
        self.height(at) / self.relief
    }

    /// How steep the ground at `at` is, the bumps underfoot aside.
    pub fn slope(&self, at: Pos<Meters>) -> Degrees {
        let (x, y) = self.lattice(at);
        let surface = self.surface(&self.heights, x, y);
        Degrees(
            (surface.dx.hypot(surface.dy) / self.spacing)
                .atan()
                .to_degrees(),
        )
    }

    /// How much of the rain ran over `at`, from 0 to 1.
    pub fn runoff(&self, at: Pos<Meters>) -> f32 {
        self.between_nodes(&self.runoff, at)
    }

    /// `values`, one for each node row by row, between the nodes around `at`.
    pub fn between_nodes(&self, values: &[f32], at: Pos<Meters>) -> f32 {
        let (x, y) = self.lattice(at);
        self.surface(values, x, y).height
    }

    /// Every node's place on the ground, row by row.
    pub fn nodes(&self) -> impl IndexedParallelIterator<Item = Pos<Meters>> + '_ {
        (0..self.rows * self.columns).into_par_iter().map(|index| {
            Pos::new(
                self.origin.x + (index % self.columns) as f32 * self.spacing,
                self.origin.y + (index / self.columns) as f32 * self.spacing,
            )
        })
    }

    /// Where `at` lies on the lattice, in nodes, clamped to its last cell.
    fn lattice(&self, at: Pos<Meters>) -> (f32, f32) {
        let largest = |nodes: usize| (nodes - 1) as f32 - f32::EPSILON * nodes as f32;
        (
            ((at.x - self.origin.x) / self.spacing).clamp(0.0, largest(self.columns)),
            ((at.y - self.origin.y) / self.spacing).clamp(0.0, largest(self.rows)),
        )
    }

    /// `values` between the nodes around `(x, y)`, and how they change along each axis per node.
    fn surface(&self, values: &[f32], x: f32, y: f32) -> Surface {
        let (column, row) = (
            (x as usize).min(self.columns - 2),
            (y as usize).min(self.rows - 2),
        );
        let (fx, fy) = (x - column as f32, y - row as f32);
        let at = row * self.columns + column;
        let (nw, ne) = (values[at], values[at + 1]);
        let (sw, se) = (values[at + self.columns], values[at + self.columns + 1]);
        Surface {
            height: nw * (1.0 - fx) * (1.0 - fy)
                + ne * fx * (1.0 - fy)
                + sw * (1.0 - fx) * fy
                + se * fx * fy,
            dx: (ne - nw) * (1.0 - fy) + (se - sw) * fy,
            dy: (sw - nw) * (1.0 - fx) + (se - ne) * fx,
        }
    }
}

/// How many lattice nodes span a wavelength of the land's shape: enough for the rain to carve
/// gullies into every hill.
const NODES_PER_WAVELENGTH: f32 = 128.0;
/// How many lattice nodes the bumps underfoot span: they rise between the nodes, where the
/// lattice alone is flat.
const NODES_PER_BUMP: f32 = 4.0;
/// How much flatter low ground is than high: plains and valley floors widen, peaks sharpen.
const LOWLAND_FLATTENING: f32 = 1.5;

// The particle erosion's own constants, at the values its authors tuned it to; how much rain
// falls is the land's.
const INERTIA: f32 = 0.05;
const SILT_CAPACITY: f32 = 4.0;
const SILT_FLOOR: f32 = 0.01;
const SILT_DEPOSIT: f32 = 0.3;
const SILT_PICKUP: f32 = 0.3;
const EVAPORATION: f32 = 0.01;
const GRAVITY: f32 = 4.0;
const DROP_LIFETIME: u32 = 30;
const BRUSH_RADIUS: usize = 3;

struct Drop {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    speed: f32,
    water: f32,
    silt: f32,
}

impl Drop {
    /// A drop just fallen at `(x, y)`, in nodes.
    fn falling(x: f32, y: f32) -> Drop {
        Drop {
            x,
            y,
            dx: 0.0,
            dy: 0.0,
            speed: 1.0,
            water: 1.0,
            silt: 0.0,
        }
    }
}

struct Surface {
    height: f32,
    dx: f32,
    dy: f32,
}

/// The nodes a drop carves around the one it stands on, nearer ones more; cut at the lattice's
/// edge.
struct Brush {
    columns: usize,
    rows: usize,
    reach: Vec<(isize, isize, f32)>,
}

impl Brush {
    fn new(radius: usize, columns: usize, rows: usize) -> Brush {
        let radius = radius as isize;
        let reach = (-radius..=radius)
            .flat_map(|dy| (-radius..=radius).map(move |dx| (dx, dy)))
            .filter_map(|(dx, dy)| {
                let weight = 1.0 - ((dx * dx + dy * dy) as f32).sqrt() / radius as f32;
                (weight > 0.0).then_some((dx, dy, weight))
            })
            .collect();
        Brush {
            columns,
            rows,
            reach,
        }
    }

    /// Carves `amount` from the ground around `(column, row)`, as much as there is of it, and
    /// answers how much was carved.
    fn carve(&self, ground: &mut [f32], column: usize, row: usize, amount: f32) -> f32 {
        let nodes = || {
            self.reach.iter().filter_map(move |&(dx, dy, weight)| {
                let x = column
                    .checked_add_signed(dx)
                    .filter(|&x| x < self.columns)?;
                let y = row.checked_add_signed(dy).filter(|&y| y < self.rows)?;
                Some((y * self.columns + x, weight))
            })
        };
        let total: f32 = nodes().map(|(_, weight)| weight).sum();
        let mut carved = 0.0;
        for (at, weight) in nodes() {
            let taken = ground[at].min(amount * weight / total);
            ground[at] -= taken;
            carved += taken;
        }
        carved
    }
}

/// Noise from about -1 to 1 on 0 to 1.
fn unit(value: f64) -> f64 {
    (value * 0.5 + 0.5).clamp(0.0, 1.0)
}

fn smoothstep(from: f64, to: f64, value: f64) -> f64 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The lowest and highest of `values`, apart by at least a little.
fn span(values: &[f32]) -> (f32, f32) {
    let low = values
        .par_iter()
        .copied()
        .reduce(|| f32::INFINITY, f32::min);
    let high = values
        .par_iter()
        .copied()
        .reduce(|| f32::NEG_INFINITY, f32::max);
    (low, high.max(low + f32::EPSILON))
}
