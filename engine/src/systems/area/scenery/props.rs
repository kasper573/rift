//! How an area's props stand and look: where a placement stands its prop, and the materials of a
//! prop's model the game's shaders draw.

use bevy::prelude::*;

use crate::core::content::Content;
use crate::core::content::gltf::GltfInfo;
use crate::core::content::media::GltfInfos;
use crate::core::ground::Heightfield;
use crate::core::render::world::material::MaterialLook;
use crate::core::render::world_point;
use crate::core::units::Meters;
use crate::core::vfx::render::{self as vfx_render, SiteExtent};
use crate::core::vfx::standing::StandingVfx;
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

/// How big a thing drawn as a model of `info` at `scale` times its size is, for what plays on it.
pub fn extent_of(info: &GltfInfo, scale: f32) -> SiteExtent {
    let size = info.bounds.max - info.bounds.min;
    SiteExtent {
        footprint: Vec2::new(size.x, size.z) * scale,
        height: info.bounds.max.y * scale,
    }
}

/// Stands, under `root`, what the vfx a placed prop plays of its own accord play on: the prop's
/// place and size, and each point of its model its vfx name, where the model has it.
pub(super) fn stand_playing(
    world: &mut World,
    content: &Content,
    heights: &Heightfield,
    placed: &PropPlacement,
    root: Entity,
) -> Entity {
    let def = placed.prop.get(content);
    let info = content.built::<GltfInfos>().0.get(def.model.as_str());
    let stands = prop_transform(heights, placed);
    let extent = info.map_or(
        SiteExtent {
            footprint: Vec2::ONE,
            height: 1.0,
        },
        |info| extent_of(info, placed.scale),
    );
    let stood = world
        .spawn((
            stands,
            GlobalTransform::from(stands),
            Visibility::default(),
            extent,
            StandingVfx(def.effects.clone()),
            ChildOf(root),
        ))
        .id();
    let sockets = def
        .effects
        .iter()
        .filter_map(|vfx| vfx.get(content).socket.as_deref());
    for socket in sockets {
        let rests = info.and_then(|info| {
            let node = info.tree.named(socket)?;
            info.tree.worlds().get(node).copied()
        });
        if let Some(rests) = rests {
            let within = Transform::from_matrix(rests);
            world.spawn((
                Name::new(socket.to_owned()),
                within,
                GlobalTransform::from(stands * within),
                ChildOf(stood),
            ));
        }
    }
    stood
}

/// How far the shaders of `def`'s materials draw past its model at most, in the model's metres.
pub(super) fn overhang(content: &Content, def: &PropDef) -> Meters {
    def.materials
        .values()
        .map(|material| material.get(content).overhang)
        .fold(
            Meters(0.0),
            |most, overhang| {
                if overhang > most { overhang } else { most }
            },
        )
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
