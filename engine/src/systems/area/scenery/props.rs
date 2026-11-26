//! How an area's props stand and look: where a placement stands its prop, and the materials of a
//! prop's model the game's shaders draw.

use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;

use crate::core::assets;
use crate::core::content::gltf::GltfInfo;
use crate::core::content::media::GltfInfos;
use crate::core::content::{Content, FileHashes, ImageRef};
use crate::core::ground::Heightfield;
use crate::core::render::world::material::{
    LookWear, MaterialLook, PictureKind, PictureRead, SceneLooks, material_picture,
};
use crate::core::units::Meters;
use crate::core::vfx::render::{self as vfx_render, SiteExtent};
use crate::core::vfx::standing::StandingVfx;
use crate::core::vfx::uniform::{ParamsUniform, ShaderValue};
use crate::systems::area::PropPlacement;
use crate::systems::area::prop::{PropDef, PropId};
use crate::systems::area::snap::SnapSurfaces;
use crate::systems::water::ripples::RippleSpot;

/// The transform a prop placed at `placed` stands with on `heights`, resting on `surfaces` where it
/// snaps to the ground.
pub fn prop_transform(
    heights: &Heightfield,
    surfaces: &SnapSurfaces,
    placed: &PropPlacement,
) -> Transform {
    surfaces.stand(heights, placed)
}

/// How big a thing drawn as a model of `info` at `scale` times its size is, for what plays on it.
pub fn extent_of(info: &GltfInfo, scale: f32) -> SiteExtent {
    let size = info.bounds.max - info.bounds.min;
    SiteExtent {
        footprint: Vec2::new(size.x, size.z) * scale,
        height: info.bounds.max.y * scale,
    }
}

/// The prop whose model an entity shows: an object's look, a prop held in a hand, a resource or a
/// dropped item. Its model stands once the pictures of the prop's own materials are ready,
/// dressed in them.
#[derive(Component, Clone, Copy, Debug)]
pub struct ShowsProp(pub PropId);

/// Stands the model of every prop shown whose materials have become ready since.
pub(super) fn show_props(world: &mut World) {
    let waiting: Vec<(Entity, PropId)> = world
        .query_filtered::<(Entity, &ShowsProp), Without<WorldAssetRoot>>()
        .iter(world)
        .map(|(entity, shown)| (entity, shown.0))
        .collect();
    if waiting.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    for (entity, prop) in waiting {
        let def = prop.get(&content);
        let Some(looks) = compiled_looks(world, &content, def) else {
            continue;
        };
        // A look's shader draws in the stage the arenas stand props in, reading where each
        // instance stands from them; a model shown alone stands in no arena, so it wears the
        // look's pictures without it.
        let looks = looks
            .into_iter()
            .map(|look| MaterialLook {
                shader: None,
                ..look
            })
            .collect();
        let model = assets::model_scene(world.resource::<AssetServer>(), def.model.path());
        if let Ok(mut shown) = world.get_entity_mut(entity) {
            shown.insert((WorldAssetRoot(model), SceneLooks(looks)));
        }
    }
}

/// Stands, under `root`, what a placed prop acts through of its own accord: the prop's place
/// and size and each point of its model its vfx name, where the model has it, for its vfx to
/// play on, and where it sends ripples over the water, the spot they spread from.
pub(super) fn stand_acting(
    world: &mut World,
    content: &Content,
    heights: &Heightfield,
    surfaces: &SnapSurfaces,
    placed: &PropPlacement,
    root: Entity,
) -> Entity {
    let def = placed.prop.get(content);
    let info = content.built::<GltfInfos>().0.get(def.model.as_str());
    let stands = prop_transform(heights, surfaces, placed);
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
            StandingVfx::playing(def.effects.clone()),
            ChildOf(root),
        ))
        .id();
    if let Some(ripples) = def.ripples {
        world.entity_mut(stood).insert(RippleSpot(ripples));
    }
    let sockets = def
        .effects
        .iter()
        .filter_map(|play| play.vfx.get(content).socket.as_deref());
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

/// The materials of `def`'s model the game draws its own way, each with its pictures and its
/// shader's parameters, once every one of their pictures is read and their shaders compiled.
pub fn compiled_looks(
    world: &mut World,
    content: &Content,
    def: &PropDef,
) -> Option<Vec<MaterialLook>> {
    let hashes = content.built::<FileHashes>();
    let looks: Vec<Option<MaterialLook>> = def
        .materials
        .iter()
        .map(|(name, material)| {
            let material = material.get(content);
            let mut picture = |image: &Option<ImageRef>, kind| {
                let Some(image) = image else {
                    return Some(None);
                };
                let held = hashes.0.get(image.as_str()).map(|hash| hash.hash.as_str());
                match material_picture(world, image.as_str(), held.unwrap_or_default(), kind) {
                    PictureRead::Waiting => None,
                    PictureRead::Ready(picture) => Some(Some(picture)),
                    PictureRead::Unreadable => Some(None),
                }
            };
            let color = picture(&material.color, PictureKind::Color);
            let normal = picture(&material.normal, PictureKind::Relief);
            let wear = match &material.wear {
                Some(wear) => {
                    let mask = picture(&Some(wear.mask.clone()), PictureKind::Mask)?;
                    let mottle = picture(&Some(wear.mottle.clone()), PictureKind::Mask)?;
                    mask.zip(mottle).map(|(mask, mottle)| LookWear {
                        mask,
                        mottle,
                        colors: [wear.colors.from, wear.colors.to]
                            .map(|color| color.color().to_linear()),
                    })
                }
                None => None,
            };
            let shader = match material.shader {
                Some(shader) => Some((
                    vfx_render::compiled_shader(world, shader)?,
                    ParamsUniform::pack(shader.get(content), |param| {
                        material.params.get(param).copied().map(ShaderValue::from)
                    }),
                )),
                None => None,
            };
            Some(MaterialLook {
                material: name.clone(),
                color: color?,
                normal: normal?,
                wear,
                params: shader
                    .as_ref()
                    .map(|(_, params)| *params)
                    .unwrap_or_default(),
                shader: shader.map(|(shader, _)| shader),
                translucency: material.translucency,
                shadow: material.shadow,
            })
        })
        .collect();
    looks.into_iter().collect()
}
