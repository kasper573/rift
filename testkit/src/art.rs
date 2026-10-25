use std::io::Cursor;

use bevy_math::{Quat, Vec3};
use engine::core::content::gltf::{
    NodeTree, Rest, TreeNode, placeholder_body, placeholder_mesh, placeholder_rig,
};
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

/// A glTF binary of `triangles` triangles animated by `clips`: a model as heavy as a test wants.
pub fn glb_of(triangles: usize, clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_mesh(clips, triangles)
}

/// The joints of every rig a test declares: a root, a body beneath it, and a right and a left hand
/// on the body.
pub fn rig() -> NodeTree {
    let joint = |name: &str, parent: Option<usize>, at: [f32; 3]| TreeNode {
        name: name.to_owned(),
        parent,
        rest: Rest {
            translation: Vec3::from(at),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
        mesh: false,
    };
    NodeTree {
        nodes: vec![
            joint("model", None, [0.0, 0.0, 0.0]),
            joint("body", Some(0), [0.0, 1.0, 0.0]),
            joint("hand.r", Some(1), [-0.3, 0.2, 0.0]),
            joint("hand.l", Some(1), [0.3, 0.2, 0.0]),
        ],
    }
}

/// A rig's glTF: `tree`'s joints at rest and nothing to draw.
pub fn rig_glb(tree: &NodeTree) -> Vec<u8> {
    placeholder_body(tree, &[], 0)
}

/// A library of `clips` made on `tree`: its joints at rest, the clips moving them, nothing drawn.
pub fn library_glb(tree: &NodeTree, clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_body(tree, clips, 0)
}

/// A skin on `tree`: its joints at rest, a triangle nobody would notice, and `clips` of its own.
pub fn skin_glb(tree: &NodeTree, clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_body(tree, clips, 1)
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
