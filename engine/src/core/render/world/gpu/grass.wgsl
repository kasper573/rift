// Lays out a cell's blades of a piece from the cell's ground alone, a workgroup a cell and a row
// of its paint a thread. Each pixel its piece's terrain paints grows as many blades as its draws
// say, each at its own place in the pixel, turned, sized and tinted by its draws, standing on
// the ground's height there and leaning as the ground slopes; a row's blades go where its start
// says, in the order of its pixels.

struct Piece {
    origin: vec2<f32>,
    per_pixel: f32,
    size_low: f32,
    size_high: f32,
    salt: u32,
    terrain: u32,
    samples_per_metre: u32,
}

struct Cell {
    first: vec2<u32>,
    blade: u32,
    unused: u32,
}

struct Blade {
    root: vec3<f32>,
    size: f32,
    normal: vec3<f32>,
    yaw: f32,
    tint: f32,
    weight: f32,
    seed: f32,
    unused: f32,
}

@group(0) @binding(0) var<uniform> piece: Piece;
@group(0) @binding(1) var<storage, read> cells: array<Cell>;
// Where each row of each cell starts among the cell's blades.
@group(0) @binding(2) var<storage, read> rows: array<u32>;
// The terrain painting each pixel of each cell, four to a word, the first in the lowest byte.
@group(0) @binding(3) var<storage, read> paint: array<u32>;
// The ground's height at each sample round each cell, in metres.
@group(0) @binding(4) var<storage, read> heights: array<f32>;
@group(0) @binding(5) var<storage, read_write> blades: array<Blade>;

const CELL_PIXELS: u32 = 64u;
const HEIGHT_SIDE: u32 = CELL_PIXELS + 3u;
const BLADE_DRAWS: u32 = 6u;
const TAU: f32 = 6.28318530718;

@compute @workgroup_size(64)
fn grow(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let at_cell = group.x;
    let cell = cells[at_cell];
    let row = local.x;
    let pixel = 1.0 / f32(piece.samples_per_metre);
    var blade = cell.blade + rows[at_cell * CELL_PIXELS + row];
    for (var column = 0u; column < CELL_PIXELS; column++) {
        if painter(at_cell, column, row) != piece.terrain {
            continue;
        }
        let global = cell.first + vec2<u32>(column, row);
        let stream = pcg(pcg(global.x ^ piece.salt) ^ global.y);
        let count = u32(floor(piece.per_pixel)) + select(0u, 1u, random(stream, 0u) < fract(piece.per_pixel));
        for (var k = 0u; k < count; k++) {
            let draw = 1u + k * BLADE_DRAWS;
            let x = piece.origin.x + (f32(global.x) + random(stream, draw)) * pixel;
            let z = piece.origin.y + (f32(global.y) + random(stream, draw + 1u)) * pixel;
            let yaw = random(stream, draw + 2u) * TAU;
            let size = piece.size_low + (piece.size_high - piece.size_low) * random(stream, draw + 3u);
            let tint = random(stream, draw + 4u);
            let seed = random(stream, draw + 5u);
            let ground = height_at(at_cell, cell.first, vec2<f32>(x, z));
            let east = height_at(at_cell, cell.first, vec2<f32>(x + pixel, z))
                - height_at(at_cell, cell.first, vec2<f32>(x - pixel, z));
            let south = height_at(at_cell, cell.first, vec2<f32>(x, z + pixel))
                - height_at(at_cell, cell.first, vec2<f32>(x, z - pixel));
            var laid: Blade;
            laid.root = vec3<f32>(x, ground, z);
            laid.size = size;
            laid.normal = normalize(vec3<f32>(-east, 2.0 * pixel, -south));
            laid.yaw = yaw;
            laid.tint = tint;
            laid.weight = 1.0;
            laid.seed = seed;
            laid.unused = 0.0;
            blades[blade] = laid;
            blade += 1u;
        }
    }
}

// The terrain painting the cell's pixel at `column` and `row`.
fn painter(at_cell: u32, column: u32, row: u32) -> u32 {
    let at = at_cell * CELL_PIXELS * CELL_PIXELS + row * CELL_PIXELS + column;
    return (paint[at / 4u] >> ((at % 4u) * 8u)) & 255u;
}

// The ground's height at `at`, read between the samples round it as the ground's own is.
fn height_at(at_cell: u32, first: vec2<u32>, at: vec2<f32>) -> f32 {
    let sample = (at - piece.origin) * f32(piece.samples_per_metre);
    let whole = floor(sample);
    let t = sample - whole;
    let local = vec2<i32>(whole) - vec2<i32>(first) + vec2<i32>(1);
    let a = height_of(at_cell, local);
    let b = height_of(at_cell, local + vec2<i32>(1, 0));
    let c = height_of(at_cell, local + vec2<i32>(0, 1));
    let d = height_of(at_cell, local + vec2<i32>(1, 1));
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

fn height_of(at_cell: u32, local: vec2<i32>) -> f32 {
    let clamped = vec2<u32>(clamp(local, vec2<i32>(0), vec2<i32>(i32(HEIGHT_SIDE) - 1)));
    return heights[at_cell * HEIGHT_SIDE * HEIGHT_SIDE + clamped.y * HEIGHT_SIDE + clamped.x];
}

fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// The `draw`th draw of `stream`, from 0 up to but not 1.
fn random(stream: u32, draw: u32) -> f32 {
    return f32(pcg(stream + draw) >> 8u) / 16777216.0;
}
