//! The material every mesh of the world is drawn with: bevy's standard lighting over the model's
//! own vertex colours, matte, a face marked as glowing shining unlit, and dithered away where the
//! mesh stands between the camera and its subject, in every pass alike, so what fades keeps its
//! depth, its shadow and its lighting and never shows its inside.

use bevy::asset::{AssetPath, embedded_path};
use bevy::ecs::system::SystemParam;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::world_serialization::WorldInstanceReady;

use super::camera::FullPicture;
use super::fade::Fades;

pub type WorldMaterial = ExtendedMaterial<StandardMaterial, Fade>;

/// The fade of a mesh, and whether the picture it is drawn in settles over the frames, which
/// the clearing's dither steps on for; it stands still otherwise.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(FadeKey)]
pub struct Fade {
    #[uniform(100)]
    pub style: FadeStyle,
    pub settling: bool,
}

/// How far a mesh has faded for standing in front of the camera's subject: `shown` 1 untouched,
/// 0 fully, a circle of `focus_radius` picture pixels around `focus`, where the subject shows,
/// seen through and softening out to its rim. The shadow it casts stays whole, as the thing
/// itself does.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct FadeStyle {
    pub focus: Vec2,
    pub shown: f32,
    pub focus_radius: f32,
}

impl Default for FadeStyle {
    fn default() -> FadeStyle {
        FadeStyle {
            focus: Vec2::ZERO,
            shown: 1.0,
            focus_radius: 0.0,
        }
    }
}

/// What the fade's shaders are built for: a settling picture or a still one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FadeKey {
    settling: bool,
}

impl From<&Fade> for FadeKey {
    fn from(fade: &Fade) -> FadeKey {
        FadeKey {
            settling: fade.settling,
        }
    }
}

impl MaterialExtension for Fade {
    fn fragment_shader() -> ShaderRef {
        embedded("fade.wesl")
    }

    fn prepass_fragment_shader() -> ShaderRef {
        embedded("fade_prepass.wesl")
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Fade>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.settling
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            fragment.shader_defs.push("SETTLING".into());
        }
        Ok(())
    }
}

fn embedded(shader: &str) -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(embedded_path!(shader)).with_source("embedded"))
}

/// The world material a mesh gets for the standard material it was loaded with: matte, lit by
/// its vertex colours alone. It is masked at a cutoff of zero, which drops nothing by itself:
/// bevy lets a material's shaders drop pixels only when its alpha mode says they may, and the
/// fade drops them in every pass.
pub fn world_material(base: StandardMaterial, full: FullPicture) -> WorldMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Mask(0.0),
            ..base
        },
        extension: Fade {
            style: FadeStyle::default(),
            settling: full.0,
        },
    }
}

/// A mesh drawn with its own material rather than one shared with everything that looks alike, so
/// its fade can move on its own.
#[derive(Component)]
pub struct OwnMaterial;

/// The world materials standing in for each standard material, shared by every mesh loaded with
/// it.
#[derive(Resource, Default)]
pub(super) struct MaterialCache(HashMap<AssetId<StandardMaterial>, Handle<WorldMaterial>>);

/// The materials a scene's meshes are dressed from.
#[derive(SystemParam)]
pub(super) struct Wardrobe<'w> {
    standard: Res<'w, Assets<StandardMaterial>>,
    materials: ResMut<'w, Assets<WorldMaterial>>,
    cache: ResMut<'w, MaterialCache>,
    full: Res<'w, FullPicture>,
}

impl Wardrobe<'_> {
    /// The world material standing in for `standard`: one of the mesh's own when `own`, else the
    /// one every mesh with that standard material shares.
    fn material_for(
        &mut self,
        standard: &Handle<StandardMaterial>,
        own: bool,
    ) -> Option<Handle<WorldMaterial>> {
        let base = self.standard.get(standard)?;
        let full = *self.full;
        Some(if own {
            self.materials.add(world_material(base.clone(), full))
        } else {
            self.cache
                .0
                .entry(standard.id())
                .or_insert_with(|| self.materials.add(world_material(base.clone(), full)))
                .clone()
        })
    }
}

/// Every scene that finishes spawning trades its standard materials for world ones: the meshes
/// of a prop that fades get materials of their own.
pub(super) fn dress_scenes(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(Entity, &MeshMaterial3d<StandardMaterial>)>,
    fading: Query<Has<Fades>>,
    mut wardrobe: Wardrobe,
    mut commands: Commands,
) {
    let root = ready.entity;
    let own = fading.get(root).unwrap_or(false);
    for entity in children.iter_descendants(root) {
        let Ok((entity, material)) = meshes.get(entity) else {
            continue;
        };
        let Some(dressed) = wardrobe.material_for(&material.0, own) else {
            continue;
        };
        let mut mesh = commands.entity(entity);
        mesh.remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(dressed));
        if own {
            mesh.insert(OwnMaterial);
        }
    }
}
