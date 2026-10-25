//! The ground as one mesh: a vertex at every height sample, coloured by the materials painted
//! there, shaded smooth by the world material; and its heights as a picture, for what a shader
//! settles on the ground.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};

use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::units::GROUND_ORIGIN;

/// Where the ground's samples lie, as a shader settling a mesh on the ground reads them: the
/// metres the first stands at, how many stand to the metre, and how far above the ground to draw.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct GroundUniform {
    pub origin: Vec2,
    pub samples_per_metre: f32,
    pub lift: f32,
}

impl GroundUniform {
    pub fn lifted(lift: f32) -> GroundUniform {
        GroundUniform {
            origin: Vec2::new(GROUND_ORIGIN.x, GROUND_ORIGIN.y),
            samples_per_metre: SAMPLES_PER_METER as f32,
            lift,
        }
    }
}

/// The ground's heights as a picture a texel per sample, in metres.
pub fn height_image(heights: &Heightfield) -> Image {
    let (columns, rows) = heights.samples();
    let data = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| heights.sample(column, row).0))
        .flat_map(f32::to_le_bytes)
        .collect();
    Image::new(
        Extent3d {
            width: columns.max(1),
            height: rows.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// The terrain's mesh, in the world's metres from the ground's corner, each sample coloured by
/// `color(column, row)` in linear RGBA.
pub fn mesh(heights: &Heightfield, color: impl Fn(u32, u32) -> [f32; 4]) -> Mesh {
    let (columns, rows) = heights.samples();
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let mut positions = Vec::with_capacity((columns * rows) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut colors = Vec::with_capacity(positions.capacity());
    for row in 0..rows {
        for column in 0..columns {
            let x = GROUND_ORIGIN.x + column as f32 * step;
            let z = GROUND_ORIGIN.y + row as f32 * step;
            positions.push([x, heights.sample(column, row).0, z]);
            let east =
                heights.sample(column + 1, row).0 - heights.sample(column.saturating_sub(1), row).0;
            let south =
                heights.sample(column, row + 1).0 - heights.sample(column, row.saturating_sub(1)).0;
            normals.push(Vec3::new(-east, 2.0 * step, -south).normalize().to_array());
            uvs.push([
                column as f32 / (columns - 1).max(1) as f32,
                row as f32 / (rows - 1).max(1) as f32,
            ]);
            colors.push(color(column, row));
        }
    }
    let mut indices = Vec::with_capacity(((columns - 1) * (rows - 1) * 6) as usize);
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let corner = row * columns + column;
            let (east, south, far) = (corner + 1, corner + columns, corner + columns + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

/// A patch of ground `size` metres across, centred on `at` and turned by `yaw`, as a grid of
/// quads half a metre apart laid `lift` above the terrain, for what is drawn on the ground. Its uv
/// runs 0..1 over the patch, y down as a picture's does.
pub fn patch(heights: &Heightfield, at: Vec2, size: Vec2, yaw: f32, lift: f32) -> Mesh {
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let across = ((size.x / step).ceil() as u32).max(1) + 1;
    let down = ((size.y / step).ceil() as u32).max(1) + 1;
    let (sin, cos) = yaw.sin_cos();
    let mut positions = Vec::with_capacity((across * down) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for row in 0..down {
        for column in 0..across {
            let u = column as f32 / (across - 1) as f32;
            let v = row as f32 / (down - 1) as f32;
            let local = Vec2::new((u - 0.5) * size.x, (v - 0.5) * size.y);
            let turned = Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos);
            let ground = at + turned;
            let height = heights
                .height_at(crate::core::math::Pos::new(ground.x, ground.y))
                .0;
            positions.push([ground.x, height + lift, ground.y]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity(((across - 1) * (down - 1) * 6) as usize);
    for row in 0..down - 1 {
        for column in 0..across - 1 {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}
