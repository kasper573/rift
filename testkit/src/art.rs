use std::io::Cursor;

use engine::core::content::gltf::placeholder_rig;
use engine::core::time::Seconds;

/// A PNG of `width` × `height` transparent pixels: an image a row can name, the right size and no
/// more.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("a png header");
    writer
        .write_image_data(&vec![0; (width * height * 4) as usize])
        .expect("png pixels");
    writer.finish().expect("a png");
    bytes
}

/// A glTF binary of one triangle animated by `clips`, each named and as long as given.
pub fn glb(clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_rig(clips)
}

/// A WAV of a moment of silence.
pub fn wav() -> Vec<u8> {
    wav_of(&[0; 8])
}

/// A WAV of `samples`, a moment long.
pub fn wav_of(samples: &[i16]) -> Vec<u8> {
    const RATE: u32 = 8000;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut bytes, spec).expect("a wav header");
    for sample in samples {
        writer.write_sample(*sample).expect("a wav sample");
    }
    writer.finalize().expect("a wav");
    bytes.into_inner()
}
