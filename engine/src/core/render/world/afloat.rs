//! What floats keeps the water out of itself. A sheet of water lies level and is drawn over all
//! that is solid, so it would stand inside a hull that rides lower than its surface. Every mesh
//! under something afloat is therefore drawn once more before the water, into the depth alone,
//! with whatever of it lies under the water's level raised to that level: a lid over what the
//! body displaces, which the water finds nearer than itself. Within the lid the eye meets the
//! boat's own boards and whoever sits in it; outside it, the hull through the water.

use bevy::light::NotShadowCaster;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{MeshTag, MeshVertexBufferLayoutRef};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, ColorWrites, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::material::{WorldMaterial, embedded};
use super::opacity::Opacity;
use crate::core::units::Meters;

/// Marks a body that floats on water whose surface lies at `level`: it and all that hangs under
/// it keeps the water out.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Afloat {
    pub level: Meters,
}

/// Marks a body carried in something afloat: the hull round it is what breaks the water, so it
/// raises no rings and trails no wake of its own, however low it sits.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aboard;

/// A mesh drawn again as its part of the lid, held by the mesh it repeats.
#[derive(Component)]
pub(super) struct WaterLid;

/// Draws a mesh into the depth alone, all of it that lies under the level its tag tells raised
/// to that level, once everything solid is drawn and before the water is.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct WaterLidMaterial {}

impl Material for WaterLidMaterial {
    fn vertex_shader() -> ShaderRef {
        embedded("afloat.wesl")
    }

    fn fragment_shader() -> ShaderRef {
        embedded("afloat.wesl")
    }

    fn reads_view_transmission_texture(&self) -> bool {
        true
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        let targets = descriptor
            .fragment
            .iter_mut()
            .flat_map(|fragment| fragment.targets.iter_mut().flatten());
        for target in targets {
            target.write_mask = ColorWrites::empty();
        }
        Ok(())
    }
}

/// The one material every lid is drawn with.
#[derive(Resource)]
pub(super) struct LidMaterial(Handle<WaterLidMaterial>);

impl FromWorld for LidMaterial {
    fn from_world(world: &mut World) -> LidMaterial {
        LidMaterial(
            world
                .resource_mut::<Assets<WaterLidMaterial>>()
                .add(WaterLidMaterial {}),
        )
    }
}

type Floating<'a> = (&'a Mesh3d, Option<Ref<'a, SkinnedMesh>>);
type Lid<'a> = (
    Entity,
    &'a ChildOf,
    &'a Mesh3d,
    &'a MeshTag,
    Has<SkinnedMesh>,
);

/// Keeps a part of the lid for every mesh under something afloat that shows whole, in step with
/// the mesh it repeats and the level its body floats at, and takes away those whose mesh floats
/// no longer.
pub(super) fn lid_the_floating(
    afloat: Query<(Entity, &Afloat)>,
    children: Query<&Children>,
    (holders, opacities): (Query<&ChildOf>, Query<&Opacity>),
    meshes: Query<Floating, (With<MeshMaterial3d<WorldMaterial>>, Without<WaterLid>)>,
    lids: Query<Lid, With<WaterLid>>,
    lid: Res<LidMaterial>,
    mut commands: Commands,
) {
    let mut floating: HashMap<Entity, MeshTag> = afloat
        .iter()
        .filter(|(body, _)| Opacity::inherited(*body, &opacities, &holders) >= 1.0)
        .flat_map(|(body, afloat)| {
            let level = MeshTag::new(afloat.level.0.to_bits());
            std::iter::once(body)
                .chain(children.iter_descendants(body))
                .map(move |part| (part, level.clone()))
        })
        .filter(|(part, _)| meshes.contains(*part))
        .collect();
    for (part, held, drawn, level, skinned) in &lids {
        let mesh = held.parent();
        let Some((wanted, (repeated, skin))) = floating.remove(&mesh).zip(meshes.get(mesh).ok())
        else {
            commands.entity(part).try_despawn();
            continue;
        };
        let mut part = commands.entity(part);
        if *level != wanted {
            part.try_insert(wanted);
        }
        if drawn != repeated {
            part.try_insert(repeated.clone());
        }
        match skin {
            Some(skin) if skin.is_changed() || !skinned => {
                part.try_insert(SkinnedMesh::clone(&skin));
            }
            None if skinned => {
                part.try_remove::<SkinnedMesh>();
            }
            _ => {}
        }
    }
    for (mesh, level) in floating {
        let Ok((drawn, skin)) = meshes.get(mesh) else {
            continue;
        };
        let mut part = commands.spawn((
            ChildOf(mesh),
            WaterLid,
            drawn.clone(),
            MeshMaterial3d(lid.0.clone()),
            level,
            Transform::IDENTITY,
            NotShadowCaster,
        ));
        if let Some(skin) = skin {
            part.insert(SkinnedMesh::clone(&skin));
        }
    }
}
