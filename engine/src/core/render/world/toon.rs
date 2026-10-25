//! The toon material every mesh of the world is drawn with: a flat face per triangle, lit in three
//! steps, outlined one pixel wide along silhouettes and creases, and dithered away as it fades.

use bevy::asset::{AssetPath, embedded_path};
use bevy::ecs::system::SystemParam;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::world_serialization::WorldInstanceReady;

use super::fade::Fades;

pub type ToonMaterial = ExtendedMaterial<StandardMaterial, Toon>;

#[derive(Asset, AsBindGroup, Clone, Debug, Default, TypePath)]
pub struct Toon {
    #[uniform(100)]
    pub style: ToonStyle,
}

/// How a mesh is drawn beyond its colours: how far it has faded for standing in front of the
/// camera's subject (`shown` 1 untouched, 0 fully: a circle of `focus_radius` picture pixels around
/// `focus`, where the subject shows, is dithered away, softening out to its rim; the shadow it
/// casts stays whole, as the thing itself does), whether its edges are inked, and whether each
/// face is lit flat from its own plane (1) or from the normals its vertices carry (0), as grass is
/// from the ground under it.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ToonStyle {
    pub focus: Vec2,
    pub shown: f32,
    pub outlined: f32,
    pub flat: f32,
    pub focus_radius: f32,
    pub padding: Vec2,
}

impl Default for ToonStyle {
    fn default() -> ToonStyle {
        ToonStyle {
            focus: Vec2::ZERO,
            shown: 1.0,
            outlined: 1.0,
            flat: 1.0,
            focus_radius: 0.0,
            padding: Vec2::ZERO,
        }
    }
}

impl MaterialExtension for Toon {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("toon.wesl")).with_source("embedded"),
        )
    }

    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("toon_prepass.wesl")).with_source("embedded"),
        )
    }
}

/// The prepass of a mesh of the world drawn without the toon's style, as one under a surface
/// effect is: it writes what the style reads of its neighbours all the same.
pub fn plain_prepass_shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("plain_prepass.wesl")).with_source("embedded"),
    )
}

/// The toon material a mesh gets for the standard material it was loaded with.
pub fn toon_of(base: StandardMaterial) -> ToonMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            ..base
        },
        extension: Toon::default(),
    }
}

/// A mesh drawn with its own material rather than one shared with everything that looks alike, so
/// its style can change on its own, as a fading prop's does.
#[derive(Component)]
pub struct OwnMaterial;

/// The toon materials standing in for each standard material, shared by every mesh loaded with it.
#[derive(Resource, Default)]
pub(super) struct ToonCache(HashMap<AssetId<StandardMaterial>, Handle<ToonMaterial>>);

/// The materials a scene's meshes are dressed from.
#[derive(SystemParam)]
pub(super) struct ToonPalette<'w> {
    standard: Res<'w, Assets<StandardMaterial>>,
    toons: ResMut<'w, Assets<ToonMaterial>>,
    cache: ResMut<'w, ToonCache>,
}

impl ToonPalette<'_> {
    /// The toon material standing in for `standard`: one of the mesh's own when `own`, else the
    /// one every mesh with that standard material shares.
    fn toon_for(
        &mut self,
        standard: &Handle<StandardMaterial>,
        own: bool,
    ) -> Option<Handle<ToonMaterial>> {
        let base = self.standard.get(standard)?;
        Some(if own {
            self.toons.add(toon_of(base.clone()))
        } else {
            self.cache
                .0
                .entry(standard.id())
                .or_insert_with(|| self.toons.add(toon_of(base.clone())))
                .clone()
        })
    }
}

/// Every scene that finishes spawning trades its standard materials for toon ones: the meshes of a
/// prop that fades get materials of their own.
pub(super) fn toon_scenes(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(Entity, &MeshMaterial3d<StandardMaterial>)>,
    fading: Query<Has<Fades>>,
    mut palette: ToonPalette,
    mut commands: Commands,
) {
    let root = ready.entity;
    let own = fading.get(root).unwrap_or(false);
    for entity in children.iter_descendants(root) {
        let Ok((entity, material)) = meshes.get(entity) else {
            continue;
        };
        let Some(toon) = palette.toon_for(&material.0, own) else {
            continue;
        };
        let mut mesh = commands.entity(entity);
        mesh.remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(toon));
        if own {
            mesh.insert(OwnMaterial);
        }
    }
}
