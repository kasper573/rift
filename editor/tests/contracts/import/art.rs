use image::{Rgba, RgbaImage};
use serde_json::{Value, json};

pub const TILE: u32 = 16;
pub const COLUMNS: u32 = 36;
pub const ROWS: u32 = 26;
/// The water's four frames lie side by side on the tileset, each a block this many tiles wide and
/// tall, the first at column 1, row 1, and each next nine columns on.
pub const WATER: u32 = 7;
/// Of the water's cells, the last this many in reading order look alike in every frame.
pub const STILL_WATER: u32 = 16;
/// A tile with a walk flag and a surface.
pub const PAVED: u32 = 450;
/// A transparent tile that carries data all the same.
pub const BLANK_WITH_DATA: u32 = COLUMNS * ROWS - 1;

/// A tileset row with a paved tile, a transparent tile carrying data, and the water's first block
/// animated through its four frames.
pub fn tileset_row() -> Value {
    let mut tiles = serde_json::Map::new();
    tiles.insert(
        PAVED.to_string(),
        json!({ "walk": "blocks", "surface": testkit::path("surface", "sand") }),
    );
    tiles.insert(BLANK_WITH_DATA.to_string(), json!({ "walk": "blocks" }));
    let first = COLUMNS + 1;
    tiles.insert(
        first.to_string(),
        json!({
            "animation": (0..4).map(|k| json!({ "tile": first + 9 * k, "ms": 250 })).collect::<Vec<_>>(),
        }),
    );
    json!({ "tile": TILE, "tiles": tiles })
}

/// A tileset of solid tiles each of its own colour, its last row transparent, and the water's four
/// frames: blocks whose cells change colour from frame to frame, but for the still ones.
pub fn tileset() -> RgbaImage {
    let mut image = RgbaImage::new(COLUMNS * TILE, ROWS * TILE);
    for index in 0..COLUMNS * (ROWS - 1) {
        let color = [(index % 256) as u8, (index / 256) as u8, 100, 255];
        paint_tile(&mut image, index, color);
    }
    for frame in 0..4 {
        for cell in 0..WATER * WATER {
            if cell >= WATER * WATER - STILL_WATER {
                let (column, row) = (1 + 9 * frame + cell % WATER, 1 + cell / WATER);
                paint_tile(
                    &mut image,
                    row * COLUMNS + column,
                    [200, cell as u8, 50, 255],
                );
            }
        }
    }
    image
}

/// The water's four frames, cut from the tileset.
pub fn water(tileset: &RgbaImage) -> Vec<RgbaImage> {
    (0..4)
        .map(|k| {
            image::imageops::crop_imm(
                tileset,
                (1 + 9 * k) * TILE,
                TILE,
                WATER * TILE,
                WATER * TILE,
            )
            .to_image()
        })
        .collect()
}

/// A palette painting the paved tile, the water's first frame and another plain tile.
pub fn island_palette() -> String {
    let beach = testkit::path("tileset", "beach");
    format!(
        "tileset,tile,flip\n{beach},{PAVED},\n{beach},{},horizontal\n{beach},3,\n",
        COLUMNS + 1
    )
}

pub fn fill(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    for dy in 0..h {
        for dx in 0..w {
            image.put_pixel(x + dx, y + dy, Rgba(color));
        }
    }
}

fn paint_tile(image: &mut RgbaImage, index: u32, color: [u8; 4]) {
    let (x, y) = ((index % COLUMNS) * TILE, (index / COLUMNS) * TILE);
    fill(image, x, y, TILE, TILE, color);
}
