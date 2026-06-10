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
