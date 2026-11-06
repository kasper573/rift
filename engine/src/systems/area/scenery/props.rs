//! How an area's props stand and look: where a placement stands its prop, and the materials of a
//! prop's model the game's shaders draw.

use bevy::prelude::*;

use crate::core::content::Content;
use crate::core::ground::Heightfield;
use crate::core::render::world::material::MaterialLook;
use crate::core::render::world_point;
use crate::core::vfx::render as vfx_render;
use crate::core::vfx::uniform::ParamsUniform;
use crate::systems::area::PropPlacement;
use crate::systems::area::prop::PropDef;

/// The transform a prop placed at `placed` stands with on `heights`.
pub fn prop_transform(heights: &Heightfield, placed: &PropPlacement) -> Transform {
    Transform {
        translation: world_point(placed.at, heights.height_at(placed.at) - placed.sunk),
        rotation: Quat::from_rotation_y(-placed.yaw.radians().0),
        scale: Vec3::splat(placed.scale),
    }
}

/// The materials of `def`'s model the game's shaders draw, each with its parameters, once every
/// one of their shaders is compiled.
pub(super) fn compiled_looks(
    world: &World,
    content: &Content,
    def: &PropDef,
) -> Option<Vec<MaterialLook>> {
    def.materials
        .iter()
        .map(|(name, material)| {
            let material = material.get(content);
            Some(MaterialLook {
                material: name.clone(),
                shader: vfx_render::compiled_shader(world, material.shader)?,
                params: ParamsUniform::pack(material.shader.get(content), |param| {
                    material.params.get(param).copied()
                }),
                translucency: material.translucency,
            })
        })
        .collect()
}
