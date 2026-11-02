//! The material every mesh of the world is drawn with: bevy's standard lighting over the model's
//! own colours, a rough dielectric with a faint sheen, a face marked as glowing shining unlit, and dithered away where the
//! mesh stands between the camera and its subject, in every pass alike, so what fades keeps its
//! depth, its shadow and its lighting and never shows its inside. A scene may have the game's own
//! shaders draw some of its materials in place of that look, each named by its material.

use bevy::asset::{AssetPath, embedded_path};
use bevy::ecs::system::SystemParam;
use bevy::gltf::GltfMaterialName;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::world_serialization::WorldInstanceReady;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::render::Look;
use crate::core::vfx::shader::PARAMS_BYTES;
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey, WeatherImage};

pub type WorldMaterial = ExtendedMaterial<StandardMaterial, WorldLook>;

/// The textures a material draws with, as a glTF model sets them.
pub fn textures(material: &StandardMaterial) -> impl Iterator<Item = &Handle<Image>> {
    [
        &material.base_color_texture,
        &material.emissive_texture,
        &material.metallic_roughness_texture,
        &material.normal_map_texture,
        &material.occlusion_texture,
    ]
    .into_iter()
    .flatten()
}

/// How the world draws a mesh beyond its standard material: its fade, and the game's shader when one draws the mesh, with its parameters, the weather it
/// reads and the site its prelude reads, which a material has none of.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(WorldLookKey)]
pub struct WorldLook {
    #[uniform(100)]
    pub style: FadeStyle,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub site: SiteUniform,
    pub shader: Option<Handle<Shader>>,
}

/// A material of a scene drawn by a shader of the game's: the material by the name its model gives
/// it, the shader and the parameters it is filled with, and the share of the light on either side
/// of a face that shines through to the other.
#[derive(Clone, Debug)]
pub struct MaterialLook {
    pub material: String,
    pub shader: Handle<Shader>,
    pub params: ParamsUniform,
    pub translucency: f32,
}

/// The materials of a scene the game's shaders draw, on the scene's root.
#[derive(Component, Clone, Debug, Default)]
pub struct MaterialLooks(pub Vec<MaterialLook>);

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

/// What a world material's shaders are built for: the game's shader when one draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldLookKey {
    shader: Option<VfxShaderKey>,
}

impl From<&WorldLook> for WorldLookKey {
    fn from(look: &WorldLook) -> WorldLookKey {
        WorldLookKey {
            shader: look.shader.as_ref().map(VfxShaderKey::of),
        }
    }
}

impl MaterialExtension for WorldLook {
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
        key: MaterialExtensionKey<WorldLook>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let WorldLookKey { shader } = key.bind_group_data;
        if let Some(shader) = shader {
            let shader = shader.handle();
            descriptor.vertex.shader = shader.clone();
            descriptor
                .vertex
                .shader_defs
                .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = shader;
                fragment
                    .shader_defs
                    .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
            }
        }
        Ok(())
    }
}

/// What the meshes of the world are drawn with, as loaded or as dressed for the world.
#[derive(SystemParam)]
pub struct Surfaces<'w, 's> {
    loaded: Query<'w, 's, &'static MeshMaterial3d<StandardMaterial>>,
    dressed: Query<'w, 's, &'static MeshMaterial3d<WorldMaterial>>,
    standard: Res<'w, Assets<StandardMaterial>>,
    world: Res<'w, Assets<WorldMaterial>>,
}

impl Surfaces<'_, '_> {
    /// Whether `mesh` is cut out by its texture's alpha, as foliage is: its outline is drawn by
    /// its texture rather than by its triangles.
    pub fn cut_out(&self, mesh: Entity) -> bool {
        let dressed = self
            .dressed
            .get(mesh)
            .ok()
            .and_then(|material| self.world.get(&material.0))
            .map(|material| material.base.alpha_mode);
        let loaded = || {
            self.loaded
                .get(mesh)
                .ok()
                .and_then(|material| self.standard.get(&material.0))
                .map(|material| world_alpha(material.alpha_mode))
        };
        matches!(dressed.or_else(loaded), Some(AlphaMode::Mask(cutoff)) if cutoff > 0.0)
    }
}

fn world_alpha(alpha: AlphaMode) -> AlphaMode {
    match alpha {
        AlphaMode::Mask(cutoff) => AlphaMode::Mask(cutoff),
        AlphaMode::Opaque => AlphaMode::Mask(0.0),
        _ => AlphaMode::Mask(0.5),
    }
}

pub(super) fn embedded(shader: &str) -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(embedded_path!(shader)).with_source("embedded"))
}

/// How every surface of the world takes the light.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SurfaceFinish {
    /// How rough every surface is, and how much light it reflects head-on: a rough dielectric
    /// keeps a faint sheen where a face turns toward the light or away from the eye.
    pub roughness: f32,
    pub reflectance: f32,
    /// How smooth and how metallic a painted metal is: smooth enough that a facet turned to the
    /// light flashes and its neighbours darken, so plate reads as metal beside cloth and skin.
    pub metal_roughness: f32,
    pub metallic: f32,
}

/// The world material a mesh gets for the standard material it was loaded with, every surface
/// alike a rough dielectric over the model's own colours. Nothing in the world is blended: a
/// material cut out by its texture's alpha stays cut at its own cutoff, a blended one is cut
/// where it is mostly clear, and any other is masked at a cutoff of zero, which drops nothing by
/// itself: bevy lets a material's shaders drop pixels only when its alpha mode says they may, and
/// the fade drops them in every pass.
pub fn world_material(base: StandardMaterial, finish: &SurfaceFinish) -> WorldMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            alpha_mode: world_alpha(base.alpha_mode),
            ..dielectric(base, finish)
        },
        extension: WorldLook {
            style: FadeStyle::default(),
            params: ParamsUniform::default(),
            weather: Handle::default(),
            site: SiteUniform::default(),
            shader: None,
        },
    }
}

/// `base` as every surface of the world is: a rough dielectric over its own colours.
pub(super) fn dielectric(base: StandardMaterial, finish: &SurfaceFinish) -> StandardMaterial {
    StandardMaterial {
        perceptual_roughness: finish.roughness,
        metallic: 0.0,
        reflectance: finish.reflectance,
        ..base
    }
}

/// The world material a mesh gets for `base` when the game's shader draws it, reading `weather`:
/// both of a face's sides drawn and lit alike by the normal the mesh carries, as foliage, which a
/// look is mostly for, points its normals where its light should fall rather than off its faces,
/// and as translucent as the look says.
pub fn looked_material(
    base: StandardMaterial,
    finish: &SurfaceFinish,
    look: &MaterialLook,
    weather: Handle<Image>,
) -> WorldMaterial {
    let mut material = world_material(base, finish);
    material.base.double_sided = false;
    material.base.cull_mode = None;
    material.base.diffuse_transmission = look.translucency;
    material.extension.shader = Some(look.shader.clone());
    material.extension.params = look.params;
    material.extension.weather = weather;
    material
}

/// What tells a look apart from another: its shader, its parameters and its translucency bit for
/// bit.
type LookKey = (AssetId<Shader>, [u32; PARAMS_BYTES / 4], u32);

/// The world materials standing in for each standard material under each look, shared by every
/// mesh loaded with it.
#[derive(Resource, Default)]
pub(super) struct MaterialCache(
    HashMap<(AssetId<StandardMaterial>, Option<LookKey>), Handle<WorldMaterial>>,
);

/// Forgets the world materials made for standard materials that are gone, so the pictures they
/// draw with go with them rather than piling up each time an area's scenes load anew.
pub(super) fn forget_dropped(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    mut cache: ResMut<MaterialCache>,
) {
    let gone = super::dropped(&mut events);
    if !gone.is_empty() {
        cache.0.retain(|(standard, _), _| !gone.contains(standard));
    }
}

/// The materials a scene's meshes are dressed from.
#[derive(SystemParam)]
pub(super) struct Wardrobe<'w> {
    standard: Res<'w, Assets<StandardMaterial>>,
    materials: ResMut<'w, Assets<WorldMaterial>>,
    cache: ResMut<'w, MaterialCache>,
    weather: Option<Res<'w, WeatherImage>>,
    look: Option<Res<'w, Look>>,
}

impl Wardrobe<'_> {
    /// The world material standing in for `standard` under `look`, which every mesh with that
    /// standard material and look shares.
    fn material_for(
        &mut self,
        standard: &Handle<StandardMaterial>,
        look: Option<&MaterialLook>,
    ) -> Option<Handle<WorldMaterial>> {
        let base = self.standard.get(standard)?.clone();
        let finish = self.look.as_ref()?.materials;
        let weather = self
            .weather
            .as_ref()
            .map(|weather| weather.0.clone())
            .unwrap_or_default();
        let make = |base: StandardMaterial| match look {
            Some(look) => looked_material(base, &finish, look, weather),
            None => world_material(base, &finish),
        };
        let key = (
            standard.id(),
            look.map(|look| {
                (
                    look.shader.id(),
                    look.params.bits(),
                    look.translucency.to_bits(),
                )
            }),
        );
        Some(
            self.cache
                .0
                .entry(key)
                .or_insert_with(|| self.materials.add(make(base)))
                .clone(),
        )
    }
}

/// Every scene that finishes spawning trades its standard materials for world ones, each drawn
/// by the game's shader when the scene's looks name its material.
pub(super) fn dress_scenes(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(
        Entity,
        &MeshMaterial3d<StandardMaterial>,
        Option<&GltfMaterialName>,
    )>,
    looks: Query<&MaterialLooks>,
    mut wardrobe: Wardrobe,
    mut commands: Commands,
) {
    let root = ready.entity;
    let looks = looks.get(root).ok();
    for entity in children.iter_descendants(root) {
        let Ok((entity, material, name)) = meshes.get(entity) else {
            continue;
        };
        let look = looks
            .zip(name)
            .and_then(|(looks, name)| looks.0.iter().find(|look| look.material == name.0));
        let Some(dressed) = wardrobe.material_for(&material.0, look) else {
            continue;
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(dressed));
    }
}
