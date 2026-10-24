use std::io::BufWriter;
use std::path::{Path, PathBuf};

use image::RgbaImage;

pub fn gif(dir: &Path, name: &str, frames: &[RgbaImage], delay: u16) -> PathBuf {
    let path = dir.join(format!("{name}.gif"));
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = image::codecs::gif::GifEncoder::new(BufWriter::new(file));
    encoder
        .set_repeat(image::codecs::gif::Repeat::Infinite)
        .unwrap();
    for frame in frames {
        let frame = image::Frame::from_parts(
            frame.clone(),
            0,
            0,
            image::Delay::from_numer_denom_ms(u32::from(delay) * 10, 1),
        );
        encoder.encode_frame(frame).unwrap();
    }
    path
}

pub fn spaced_grid(
    dir: &Path,
    name: &str,
    packed: &RgbaImage,
    tile: u32,
    margin: u32,
    spacing: u32,
    extruded: u32,
) -> PathBuf {
    let columns = packed.width() / tile;
    let rows = packed.height() / tile;
    let cell = tile + 2 * extruded;
    let mut out = RgbaImage::new(
        2 * margin + columns * cell + (columns - 1) * spacing,
        2 * margin + rows * cell + (rows - 1) * spacing,
    );
    for row in 0..rows {
        for column in 0..columns {
            let cut =
                image::imageops::crop_imm(packed, column * tile, row * tile, tile, tile).to_image();
            let x = margin + column * (cell + spacing);
            let y = margin + row * (cell + spacing);
            for dy in 0..cell {
                for dx in 0..cell {
                    let sx = dx.saturating_sub(extruded).min(tile - 1);
                    let sy = dy.saturating_sub(extruded).min(tile - 1);
                    out.put_pixel(x + dx, y + dy, *cut.get_pixel(sx, sy));
                }
            }
        }
    }
    let path = dir.join(format!("{name}.png"));
    save(&path, &out);
    path
}

fn save(path: &Path, image: &RgbaImage) {
    image.save(path).expect("saves");
}
