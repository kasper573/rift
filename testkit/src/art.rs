use std::collections::BTreeMap;
use std::io::Cursor;

use bevy_math::{Quat, Vec3};
use engine::core::content::gltf::{
    NodeTree, Rest, TreeNode, glb as write_glb, placeholder_body, placeholder_mesh,
    placeholder_rig, posed_body, push_accessor,
};
use engine::core::units::Seconds;
use gltf::json;
use gltf::json::validation::{Checked, USize64};

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

/// A library of one clip, `name`, made on `tree`: its joints at rest, and the clip holding each
/// joint `turns` names at its turn.
pub fn pose_glb(tree: &NodeTree, name: &str, turns: &[(&str, Quat)]) -> Vec<u8> {
    posed_body(tree, name, Seconds(0.5), turns)
}

/// A glTF binary of one triangle animated by `clips`, each named and as long as given.
pub fn glb(clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_rig(clips)
}

/// A glTF binary of a box `size` metres across, tall and deep, standing on its origin, in one
/// material named `material` of the plain colour `color`: a model a picture shows.
pub fn box_glb(size: Vec3, material: &str, color: [f32; 4]) -> Vec<u8> {
    let half = Vec3::new(size.x / 2.0, 0.0, size.z / 2.0);
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y, Vec3::Z),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z, Vec3::X),
        (Vec3::Z, Vec3::Y, Vec3::X),
        (Vec3::NEG_Z, Vec3::X, Vec3::Y),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    for (normal, along, up) in faces {
        let center = Vec3::Y * (size.y / 2.0) + normal * (size / 2.0);
        let corner = |a: f32, b: f32| {
            center
                + along * (a * (size * along).length() / 2.0)
                + up * (b * (size * up).length() / 2.0)
        };
        let quad = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
        ];
        for index in [0, 1, 2, 0, 2, 3] {
            positions.extend_from_slice(&quad[index].to_array());
            normals.extend_from_slice(&normal.to_array());
            uvs.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][index]);
            colors.extend_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        }
    }
    let floats = |values: &[f32]| -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    };
    let mut root = json::Root::default();
    let mut bin = Vec::new();
    let corners = positions.len() / 3;
    let position = push_accessor(
        &mut root,
        &mut bin,
        &floats(&positions),
        corners,
        json::accessor::Type::Vec3,
        Some((&[-half.x, 0.0, -half.z], &[half.x, size.y, half.z])),
    );
    let normal = push_accessor(
        &mut root,
        &mut bin,
        &floats(&normals),
        corners,
        json::accessor::Type::Vec3,
        None,
    );
    let uv = push_accessor(
        &mut root,
        &mut bin,
        &floats(&uvs),
        corners,
        json::accessor::Type::Vec2,
        None,
    );
    let tint = push_accessor(
        &mut root,
        &mut bin,
        &floats(&colors),
        corners,
        json::accessor::Type::Vec4,
        None,
    );
    let material = root.push(json::Material {
        name: Some(material.to_owned()),
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor(color),
            metallic_factor: json::material::StrengthFactor(0.0),
            roughness_factor: json::material::StrengthFactor(1.0),
            ..Default::default()
        },
        ..Default::default()
    });
    let mesh = root.push(json::Mesh {
        extensions: None,
        extras: Default::default(),
        name: None,
        primitives: vec![json::mesh::Primitive {
            attributes: BTreeMap::from([
                (Checked::Valid(json::mesh::Semantic::Positions), position),
                (Checked::Valid(json::mesh::Semantic::Normals), normal),
                (Checked::Valid(json::mesh::Semantic::TexCoords(0)), uv),
                (Checked::Valid(json::mesh::Semantic::Colors(0)), tint),
            ]),
            extensions: None,
            extras: Default::default(),
            indices: None,
            material: Some(material),
            mode: Checked::Valid(json::mesh::Mode::Triangles),
            targets: None,
        }],
        weights: None,
    });
    let node = root.push(json::Node {
        mesh: Some(mesh),
        ..Default::default()
    });
    let scene = root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: vec![node],
    });
    root.scene = Some(scene);
    root.push(json::Buffer {
        byte_length: USize64::from(bin.len()),
        extensions: None,
        extras: Default::default(),
        name: None,
        uri: None,
    });
    write_glb(&root, bin)
}

/// A glTF binary of nothing to look at, keeping the PNG `png` inside it as its image `name`.
pub fn glb_with_image_inside(name: &str, png: &[u8]) -> Vec<u8> {
    let (mut root, mut bin) = opened(placeholder_rig(&[]));
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let view = root.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: USize64::from(png.len()),
        byte_offset: Some(USize64::from(bin.len())),
        byte_stride: None,
        extensions: None,
        extras: Default::default(),
        name: None,
        target: None,
    });
    bin.extend_from_slice(png);
    root.buffers[0].byte_length = USize64::from(bin.len());
    root.push(json::Image {
        buffer_view: Some(view),
        mime_type: Some(json::image::MimeType("image/png".to_owned())),
        name: Some(name.to_owned()),
        uri: None,
        extensions: None,
        extras: Default::default(),
    });
    write_glb(&root, bin)
}

/// A glTF binary of nothing to look at whose one image is the file at `uri` beside it.
pub fn glb_naming_image(uri: &str) -> Vec<u8> {
    let (mut root, bin) = opened(placeholder_rig(&[]));
    root.push(json::Image {
        buffer_view: None,
        mime_type: None,
        name: None,
        uri: Some(uri.to_owned()),
        extensions: None,
        extras: Default::default(),
    });
    write_glb(&root, bin)
}

fn opened(glb: Vec<u8>) -> (json::Root, Vec<u8>) {
    let read = gltf::Gltf::from_slice(&glb).expect("a placeholder reads");
    (read.document.into_json(), read.blob.unwrap_or_default())
}

/// A glTF binary of `triangles` triangles animated by `clips`: a model as heavy as a test wants.
pub fn glb_of(triangles: usize, clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_mesh(clips, triangles)
}

/// The joints of every rig a test declares: a root, a body beneath it, a right and a left hand on
/// the body, and a finger on the right hand.
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
            joint("finger.r", Some(2), [-0.05, 0.0, 0.0]),
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
pub(crate) fn wav_of(samples: &[i16]) -> Vec<u8> {
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
