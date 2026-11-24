//! The material every mesh of the world is drawn with: bevy's standard lighting over the model's
//! own colours, a rough dielectric with a faint sheen, and a face marked as glowing shining unlit.
//! A material its model makes let light through is glass, and keeps the model's own finish.
//! A prop may have the game draw some of its materials in place of that look, each named by its
//! material: in pictures of the game's own, by a shader of the game's, or both.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path};
use bevy::ecs::system::SystemParam;
use bevy::ecs::system::SystemState;
use bevy::gltf::{Gltf, GltfAssetLabel, GltfMaterial, GltfMaterialName};
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy::world_serialization::WorldInstanceReady;

use image::imageops::{self, FilterType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::render::Look;
use crate::core::vfx::board::ShaderBoardImage;
use crate::core::vfx::shader::PARAMS_BYTES;
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};

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

/// How the world draws a mesh beyond its standard material, which has none of these: the game's
/// shader where one draws the mesh, with its parameters, the shader board it reads and
/// the site its prelude reads.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(WorldLookKey)]
pub struct WorldLook {
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub board: Handle<Image>,
    #[uniform(103)]
    pub site: SiteUniform,
    pub shader: Option<Handle<Shader>>,
}

/// A material of a scene drawn the game's way: the material by the name its model gives it, the
/// pictures its colours and its relief are read from in place of the model's own, the shader that
/// draws it with the parameters it is filled with, and the share of the light on either side of a
/// face that shines through to the other.
#[derive(Clone, Debug)]
pub struct MaterialLook {
    pub material: String,
    pub color: Option<Handle<Image>>,
    pub normal: Option<Handle<Image>>,
    pub shader: Option<Handle<Shader>>,
    pub params: ParamsUniform,
    pub translucency: f32,
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
        embedded("material.wesl")
    }

    fn prepass_fragment_shader() -> ShaderRef {
        embedded("material_prepass.wesl")
    }

    /// The game's shader draws the mesh where one is given.
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<WorldLook>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let WorldLookKey { shader } = key.bind_group_data;
        let Some(shader) = shader else {
            return Ok(());
        };
        let shader = shader.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        Ok(())
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

/// How glass is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glass {
    /// Seen through: what lies behind a pane shows through it, bent as the model's glass bends
    /// light.
    Clear,
    /// A smooth face of the pane's own colour: how what stands in numbers draws it, in passes
    /// that cut pixels and never read what lies behind them.
    Solid,
}

/// Whether a model makes `material` let light through.
pub fn is_glass(material: &StandardMaterial) -> bool {
    material.specular_transmission > 0.0
}

/// The world material a mesh gets for the standard material it was loaded with, every surface
/// alike a rough dielectric over the model's own colours. Nothing else in the world is blended: a
/// material cut out by its texture's alpha stays cut at its own cutoff, a blended one is cut
/// where it is mostly clear, and any other is masked at a cutoff of zero, which drops nothing by
/// itself: bevy lets a material's shaders drop pixels only when its alpha mode says they may, and
/// a prop clearing from the camera's subject drops them in every pass. Glass keeps the finish its
/// model gives it, and is drawn as `glass` says. Clear glass is blended, though it covers all
/// that lies behind it, to leave the depth as it was: what is drawn after it, a flame behind its
/// pane, is cut by the depth.
pub fn world_material(
    base: StandardMaterial,
    finish: &SurfaceFinish,
    glass: Glass,
) -> WorldMaterial {
    let base = match (is_glass(&base), glass) {
        (true, Glass::Clear) => StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..base
        },
        (true, Glass::Solid) => StandardMaterial {
            alpha_mode: world_alpha(base.alpha_mode),
            specular_transmission: 0.0,
            ..base
        },
        (false, _) => StandardMaterial {
            alpha_mode: world_alpha(base.alpha_mode),
            ..dielectric(base, finish)
        },
    };
    ExtendedMaterial {
        base,
        extension: WorldLook {
            params: ParamsUniform::default(),
            board: Handle::default(),
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

/// The world material a mesh gets for `base` under `look`, as translucent as the look says: its
/// colours and its relief read from the look's pictures where it has them. Where the look's
/// shader draws it, reading `board`, both of a face's sides are drawn and lit alike by the normal
/// the mesh carries, as foliage, which a shader is mostly for, points its normals where its light
/// should fall rather than off its faces.
pub fn looked_material(
    base: StandardMaterial,
    finish: &SurfaceFinish,
    look: &MaterialLook,
    board: Handle<Image>,
) -> WorldMaterial {
    let base = StandardMaterial {
        base_color_texture: look.color.clone().or(base.base_color_texture),
        normal_map_texture: look.normal.clone().or(base.normal_map_texture),
        ..base
    };
    let mut material = world_material(base, finish, Glass::Solid);
    material.base.diffuse_transmission = look.translucency;
    if let Some(shader) = &look.shader {
        material.base.double_sided = false;
        material.base.cull_mode = None;
        material.extension.shader = Some(shader.clone());
        material.extension.params = look.params;
        material.extension.board = board;
    }
    material
}

/// What tells a look apart from another: its pictures, its shader, its parameters and its
/// translucency bit for bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LookKey {
    color: Option<AssetId<Image>>,
    normal: Option<AssetId<Image>>,
    shader: Option<AssetId<Shader>>,
    params: [u32; PARAMS_BYTES / 4],
    translucency: u32,
}

/// The world materials standing in for each standard material under each look, its glass drawn
/// one way or the other, shared by every mesh loaded with it and dressed alike.
#[derive(Resource, Default)]
pub(super) struct MaterialCache(
    HashMap<(AssetId<StandardMaterial>, Option<LookKey>, Glass), Handle<WorldMaterial>>,
);

/// Forgets the world materials made for standard materials that are gone, so the pictures they
/// draw with go with them rather than piling up each time an area's scenes load anew.
pub(super) fn forget_dropped(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    mut cache: ResMut<MaterialCache>,
) {
    let gone = super::dropped(&mut events);
    if !gone.is_empty() {
        cache.0.retain(|(standard, ..), _| !gone.contains(standard));
    }
}

/// The materials a scene's meshes are dressed from.
#[derive(SystemParam)]
pub(super) struct Wardrobe<'w> {
    standard: Res<'w, Assets<StandardMaterial>>,
    materials: ResMut<'w, Assets<WorldMaterial>>,
    cache: ResMut<'w, MaterialCache>,
    board: Option<Res<'w, ShaderBoardImage>>,
    look: Option<Res<'w, Look>>,
}

impl Wardrobe<'_> {
    /// The world material standing in for `standard` under `look`, its glass drawn as `glass`
    /// where no look draws it, which every mesh with that standard material dressed alike shares.
    pub(super) fn material_for(
        &mut self,
        standard: &Handle<StandardMaterial>,
        look: Option<&MaterialLook>,
        glass: Glass,
    ) -> Option<Handle<WorldMaterial>> {
        let base = self.standard.get(standard)?.clone();
        let finish = self.look.as_ref()?.materials;
        let board = self
            .board
            .as_ref()
            .map(|board| board.image.clone())
            .unwrap_or_default();
        let make = |base: StandardMaterial| match look {
            Some(look) => looked_material(base, &finish, look, board),
            None => world_material(base, &finish, glass),
        };
        let looked = look.map(|look| LookKey {
            color: look.color.as_ref().map(Handle::id),
            normal: look.normal.as_ref().map(Handle::id),
            shader: look.shader.as_ref().map(Handle::id),
            params: look.params.bits(),
            translucency: look.translucency.to_bits(),
        });
        let glass = look.map_or(glass, |_| Glass::Solid);
        let key = (standard.id(), looked, glass);
        Some(
            self.cache
                .0
                .entry(key)
                .or_insert_with(|| self.materials.add(make(base)))
                .clone(),
        )
    }
}

/// What a picture a material is drawn in holds: the colours of a surface, or its relief as the
/// way each point of it faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PictureKind {
    Color,
    Relief,
}

/// The pictures the game's materials are drawn in, each read once however many materials and
/// models draw in it, and kept with its smaller copies down to a pixel: a picture repeats across
/// a wall or a roof, and without them it glitters wherever it is seen from far off.
#[derive(Resource, Default)]
pub struct MaterialPictures(HashMap<(String, PictureKind), Pictured>);

struct Pictured {
    /// What the file held when it was read, so a picture changed since is read again.
    held: String,
    state: PictureState,
}

enum PictureState {
    Reading(Handle<Image>),
    Scaling(Task<Option<Image>>),
    Ready(Handle<Image>),
    Unreadable,
}

/// How far a picture a material asked for has come.
pub enum PictureRead {
    /// Still being read and scaled: ask again next frame.
    Waiting,
    Ready(Handle<Image>),
    /// Its file can't be read as a picture, which the log says once: what asked for it draws
    /// without it rather than wait for what never comes.
    Unreadable,
}

/// The picture at `path` of the game folder, holding `held`, as materials draw in it: ready once
/// it is read and scaled, which takes a few frames from the first time it is asked for.
pub fn material_picture(
    world: &mut World,
    path: &str,
    held: &str,
    kind: PictureKind,
) -> PictureRead {
    world.resource_scope(|world, mut pictures: Mut<MaterialPictures>| {
        let key = (path.to_owned(), kind);
        if pictures.0.get(&key).is_none_or(|known| known.held != held) {
            let reading = world
                .resource::<AssetServer>()
                .load_builder()
                .with_settings(|settings: &mut ImageLoaderSettings| {
                    settings.asset_usage = RenderAssetUsages::MAIN_WORLD;
                })
                .load(path.to_owned());
            pictures.0.insert(
                key.clone(),
                Pictured {
                    held: held.to_owned(),
                    state: PictureState::Reading(reading),
                },
            );
        }
        let Some(known) = pictures.0.get_mut(&key) else {
            return PictureRead::Waiting;
        };
        let state = &mut known.state;
        match state {
            PictureState::Reading(reading) => {
                let read = world.resource::<Assets<Image>>().get(&*reading).cloned();
                if let Some(read) = read {
                    let scaling =
                        AsyncComputeTaskPool::get().spawn(async move { scaled(read, kind) });
                    *state = PictureState::Scaling(scaling);
                } else if world
                    .resource::<AssetServer>()
                    .load_state(&*reading)
                    .is_failed()
                {
                    warn!("the picture {path} of a material can't be read");
                    *state = PictureState::Unreadable;
                    return PictureRead::Unreadable;
                }
                PictureRead::Waiting
            }
            PictureState::Scaling(scaling) => match check_ready(scaling) {
                None => PictureRead::Waiting,
                Some(Some(scaled)) => {
                    let picture = world.resource_mut::<Assets<Image>>().add(scaled);
                    *state = PictureState::Ready(picture.clone());
                    PictureRead::Ready(picture)
                }
                Some(None) => {
                    warn!("the picture {path} of a material can't be scaled");
                    *state = PictureState::Unreadable;
                    PictureRead::Unreadable
                }
            },
            PictureState::Ready(picture) => PictureRead::Ready(picture.clone()),
            PictureState::Unreadable => PictureRead::Unreadable,
        }
    })
}

/// `read` with every smaller copy of itself down to a pixel, repeating every way.
fn scaled(read: Image, kind: PictureKind) -> Option<Image> {
    let full = read.try_into_dynamic().ok()?.to_rgba8();
    let (width, height) = full.dimensions();
    let levels = width.max(height).max(1).ilog2() + 1;
    let data = (0..levels)
        .scan(full, |level, _| {
            let bytes = level.as_raw().clone();
            *level = imageops::resize(
                level,
                (level.width() / 2).max(1),
                (level.height() / 2).max(1),
                FilterType::Triangle,
            );
            Some(bytes)
        })
        .flatten()
        .collect();
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        match kind {
            PictureKind::Color => TextureFormat::Rgba8UnormSrgb,
            PictureKind::Relief => TextureFormat::Rgba8Unorm,
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        anisotropy_clamp: PICTURE_SHARPNESS,
        ..ImageSamplerDescriptor::linear()
    });
    Some(image)
}

/// How many samples a picture seen at a slant is read with.
const PICTURE_SHARPNESS: u16 = 16;

/// The world material a part of `model` drawn in its glTF `material` gets under `look`, its glass
/// solid, shared with every mesh loaded with that material and dressed alike: once the model has
/// loaded.
pub fn part_material(
    world: &mut World,
    model: &Handle<Gltf>,
    material: Option<&Handle<GltfMaterial>>,
    look: Option<&MaterialLook>,
) -> Option<Handle<WorldMaterial>> {
    let label = match material {
        Some(material) => format!("{}/std", material.path()?.label()?),
        None => format!("{}/std", GltfAssetLabel::DefaultMaterial),
    };
    let standard = world
        .resource::<AssetServer>()
        .get_handle::<StandardMaterial>(model.path()?.clone().with_label(label))?;
    let mut wardrobe = SystemState::<Wardrobe>::new(world);
    let made = wardrobe
        .get_mut(world)
        .ok()?
        .material_for(&standard, look, Glass::Solid);
    wardrobe.apply(world);
    made
}

/// The looks a scene's materials are drawn in, each by the name its model gives the material: set
/// on the scene's root before the scene spawns, so its meshes are dressed in them from the first.
#[derive(Component, Clone, Debug, Default)]
pub struct SceneLooks(pub Vec<MaterialLook>);

/// Every scene that finishes spawning trades its standard materials for world ones, each in the
/// look its root's [`SceneLooks`] gives its material, and otherwise its glass clear and casting no
/// shadow: light passes through a pane.
pub(super) fn dress_scenes(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(&MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialName>)>,
    looks: Query<&SceneLooks>,
    mut wardrobe: Wardrobe,
    mut commands: Commands,
) {
    let looks = looks.get(ready.entity).ok();
    for entity in children.iter_descendants(ready.entity) {
        let Ok((material, name)) = meshes.get(entity) else {
            continue;
        };
        let look = looks
            .zip(name)
            .and_then(|(looks, name)| looks.0.iter().find(|look| look.material == name.0));
        let Some(dressed) = wardrobe.material_for(&material.0, look, Glass::Clear) else {
            continue;
        };
        let glass = look.is_none() && wardrobe.standard.get(&material.0).is_some_and(is_glass);
        let mut mesh = commands.entity(entity);
        mesh.remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(dressed));
        if glass {
            mesh.insert(NotShadowCaster);
        }
    }
}
