//! A model as the world draws it: the pieces its glTF holds, each mesh placed by its node's
//! transform, and their vertices joined by the material they are drawn in.

use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::mesh::VertexAttributeValues;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

/// A piece of a model: its mesh, placed by its node's transform, and the material the model
/// draws it with.
pub struct Piece {
    pub mesh: Mesh,
    pub transform: Transform,
    pub material: Option<Handle<GltfMaterial>>,
}

/// The vertices of `pieces` joined into one piece for each material they are drawn in.
pub fn by_material(pieces: &[Piece]) -> Vec<DrawnIn> {
    let placed: Vec<(&Piece, PieceVertices)> = pieces
        .iter()
        .filter_map(|piece| Some((piece, piece_vertices(piece)?)))
        .collect();
    let mut materials: Vec<Option<Handle<GltfMaterial>>> = Vec::new();
    for (piece, _) in &placed {
        if !materials.contains(&piece.material) {
            materials.push(piece.material.clone());
        }
    }
    materials
        .into_iter()
        .map(|material| {
            let drawn: Vec<&PieceVertices> = placed
                .iter()
                .filter(|(piece, _)| piece.material == material)
                .map(|(_, vertices)| vertices)
                .collect();
            DrawnIn {
                material,
                vertices: joined(&drawn),
            }
        })
        .collect()
}

/// The vertices of a model's pieces drawn in one of its materials.
pub struct DrawnIn {
    pub material: Option<Handle<GltfMaterial>>,
    pub vertices: PieceVertices,
}

/// The pieces' vertices as one piece's.
fn joined(pieces: &[&PieceVertices]) -> PieceVertices {
    let mut joined = PieceVertices {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        colors: Some(Vec::new()),
        indices: Vec::new(),
    };
    for piece in pieces {
        let base = joined.positions.len() as u32;
        joined.positions.extend(&piece.positions);
        joined.normals.extend(&piece.normals);
        joined.uvs.extend(&piece.uvs);
        if let Some(colors) = joined.colors.as_mut() {
            match &piece.colors {
                Some(piece_colors) => colors.extend(piece_colors),
                None => colors.extend(std::iter::repeat_n([1.0; 4], piece.positions.len())),
            }
        }
        joined
            .indices
            .extend(piece.indices.iter().map(|index| base + index));
    }
    joined
}

/// A piece's vertices in the model's own space, with the colours it carries if any.
pub struct PieceVertices {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
}

/// The model's meshes as pieces, each placed by its node's transform down from the roots, once
/// every mesh has loaded.
pub fn pieces_of(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Option<Vec<Piece>> {
    let children: HashSet<AssetId<GltfNode>> = gltf
        .nodes
        .iter()
        .filter_map(|node| nodes.get(node))
        .flat_map(|node| node.children.iter().map(Handle::id))
        .collect();
    let mut pieces = Vec::new();
    let mut pending: Vec<(Handle<GltfNode>, Transform)> = gltf
        .nodes
        .iter()
        .filter(|node| !children.contains(&node.id()))
        .map(|node| (node.clone(), Transform::IDENTITY))
        .collect();
    while let Some((handle, parent)) = pending.pop() {
        let node = nodes.get(&handle)?;
        let transform = parent * node.transform;
        if let Some(mesh) = &node.mesh {
            for primitive in &gltf_meshes.get(mesh)?.primitives {
                pieces.push(Piece {
                    mesh: meshes.get(&primitive.mesh)?.clone(),
                    transform,
                    material: primitive.material.clone(),
                });
            }
        }
        pending.extend(node.children.iter().map(|child| (child.clone(), transform)));
    }
    Some(pieces)
}

fn piece_vertices(piece: &Piece) -> Option<PieceVertices> {
    let positions: Vec<Vec3> = piece
        .mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)?
        .as_float3()?
        .iter()
        .map(|position| piece.transform.transform_point(Vec3::from_array(*position)))
        .collect();
    let normals: Vec<Vec3> = match piece.mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(normals) => normals
            .as_float3()?
            .iter()
            .map(|normal| piece.transform.rotation * Vec3::from_array(*normal))
            .collect(),
        None => vec![Vec3::Y; positions.len()],
    };
    let uvs: Vec<[f32; 2]> = match piece.mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uvs)) => uvs.clone(),
        _ => vec![[0.0, 0.0]; positions.len()],
    };
    let colors = match piece.mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(colors)) => Some(colors.clone()),
        _ => None,
    };
    let indices: Vec<u32> = match piece.mesh.indices() {
        Some(indices) => indices.iter().map(|index| index as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    Some(PieceVertices {
        positions,
        normals,
        uvs,
        colors,
        indices,
    })
}
