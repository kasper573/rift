//! RGBA image primitives behind a single PNG-codec boundary: the `png` crate is an
//! implementation detail no other crate sees.

#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> &[u8] {
        let offset = (y * self.width as usize + x) * 4;
        &self.rgba[offset..offset + 4]
    }
}

pub fn decode_png(bytes: &[u8]) -> Image {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut raw = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).expect("PNG frame");
    let pixels = (info.width as usize) * (info.height as usize);
    let mut rgba = vec![0u8; pixels * 4];
    match info.color_type {
        png::ColorType::Rgba => rgba.copy_from_slice(&raw[..pixels * 4]),
        png::ColorType::Rgb => {
            for (out, src) in rgba.chunks_mut(4).zip(raw[..pixels * 3].chunks(3)) {
                out.copy_from_slice(&[src[0], src[1], src[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (out, src) in rgba.chunks_mut(4).zip(raw[..pixels * 2].chunks(2)) {
                out.copy_from_slice(&[src[0], src[0], src[0], src[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for (out, &g) in rgba.chunks_mut(4).zip(&raw[..pixels]) {
                out.copy_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::Indexed => panic!("indexed PNG should have been expanded"),
    }
    Image {
        width: info.width,
        height: info.height,
        rgba,
    }
}

pub fn encode_png(image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("png header")
        .write_image_data(&image.rgba)
        .expect("png data");
    out
}

pub use math::{Pixels, Pos, Size};

/// An atlas sub-rectangle in pixels.
pub type Region = math::Rect<Pixels>;

/// The dimensions from a PNG's IHDR header, without decoding any pixels.
pub fn png_size(bytes: &[u8]) -> Option<Size<Pixels>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some(Size::new(Pixels(width as f32), Pixels(height as f32)))
}

pub fn fill(dst: &mut Image, dst_pos: Pos<Pixels>, dst_size: Size<Pixels>, color: u32) {
    let rgba = color.to_be_bytes();
    let (dst_x, dst_y) = (dst_pos.x.0 as i32, dst_pos.y.0 as i32);
    let (dw, dh) = (dst_size.x.0 as i32, dst_size.y.0 as i32);
    for oy in 0..dh {
        let py = dst_y + oy;
        if py < 0 || py >= dst.height as i32 {
            continue;
        }
        for ox in 0..dw {
            let px = dst_x + ox;
            if px < 0 || px >= dst.width as i32 {
                continue;
            }
            let target = ((py as u32 * dst.width + px as u32) * 4) as usize;
            dst.rgba[target..target + 4].copy_from_slice(&rgba);
        }
    }
}

pub fn blit(
    dst: &mut Image,
    src: &Image,
    region: Region,
    dst_pos: Pos<Pixels>,
    dst_size: Size<Pixels>,
    tint: u32,
    flip: (bool, bool),
) {
    let (rx, ry) = (region.pos.x.0 as u32, region.pos.y.0 as u32);
    let (rw, rh) = (region.size.x.0 as u32, region.size.y.0 as u32);
    let (dst_x, dst_y) = (dst_pos.x.0 as i32, dst_pos.y.0 as i32);
    let (dw, dh) = (dst_size.x.0 as u32, dst_size.y.0 as u32);
    if rw == 0 || rh == 0 || dw == 0 || dh == 0 {
        return;
    }
    let oy0 = (-dst_y).max(0) as u32;
    let oy1 = (dst.height as i32 - dst_y).clamp(0, dh as i32) as u32;
    let ox0 = (-dst_x).max(0) as u32;
    let ox1 = (dst.width as i32 - dst_x).clamp(0, dw as i32) as u32;
    if ox0 >= ox1 || oy0 >= oy1 {
        return;
    }

    let unscaled = (rw, rh) == (dw, dh);
    let plain = unscaled && tint == 0xFFFF_FFFF && !flip.0;
    let [tr, tg, tb, ta] = tint.to_be_bytes().map(u32::from);

    for oy in oy0..oy1 {
        let mut sample_y = if unscaled { oy } else { oy * rh / dh };
        if flip.1 {
            sample_y = rh - 1 - sample_y;
        }
        let sy = ry + sample_y;
        if sy >= src.height {
            continue;
        }
        let py = (dst_y + oy as i32) as u32;

        if plain {
            if rx + ox0 >= src.width {
                continue;
            }
            let end = ox1.min(src.width - rx);
            let s = ((sy * src.width + rx + ox0) * 4) as usize;
            let d = ((py * dst.width) as i32 + dst_x + ox0 as i32) as usize * 4;
            let n = ((end - ox0) * 4) as usize;
            blit_row(&mut dst.rgba[d..d + n], &src.rgba[s..s + n]);
            continue;
        }

        for ox in ox0..ox1 {
            let mut sample_x = if unscaled { ox } else { ox * rw / dw };
            if flip.0 {
                sample_x = rw - 1 - sample_x;
            }
            let sx = rx + sample_x;
            if sx >= src.width {
                continue;
            }
            let source = ((sy * src.width + sx) * 4) as usize;
            let alpha = src.rgba[source + 3] as u32 * ta / 255;
            if alpha == 0 {
                continue;
            }
            let r = src.rgba[source] as u32 * tr / 255;
            let g = src.rgba[source + 1] as u32 * tg / 255;
            let b = src.rgba[source + 2] as u32 * tb / 255;
            let target = (((py * dst.width) as i32 + dst_x + ox as i32) * 4) as usize;
            let inverse = 255 - alpha;
            dst.rgba[target] = ((r * alpha + dst.rgba[target] as u32 * inverse) / 255) as u8;
            dst.rgba[target + 1] =
                ((g * alpha + dst.rgba[target + 1] as u32 * inverse) / 255) as u8;
            dst.rgba[target + 2] =
                ((b * alpha + dst.rgba[target + 2] as u32 * inverse) / 255) as u8;
            dst.rgba[target + 3] = 255;
        }
    }
}

/// One unscaled, untinted row: opaque spans copy whole, translucent pixels blend.
fn blit_row(dst: &mut [u8], src: &[u8]) {
    let n = src.len() / 4;
    let mut x = 0;
    while x < n {
        match src[4 * x + 3] {
            255 => {
                let start = x;
                while x < n && src[4 * x + 3] == 255 {
                    x += 1;
                }
                dst[4 * start..4 * x].copy_from_slice(&src[4 * start..4 * x]);
            }
            0 => x += 1,
            alpha => {
                let (a, inverse) = (alpha as u32, 255 - alpha as u32);
                for c in 0..3 {
                    let i = 4 * x + c;
                    dst[i] = ((src[i] as u32 * a + dst[i] as u32 * inverse) / 255) as u8;
                }
                dst[4 * x + 3] = 255;
                x += 1;
            }
        }
    }
}
