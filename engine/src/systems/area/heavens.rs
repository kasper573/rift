//! An area's heavens as shader stages: the sky, a dome round the camera at the far end of every ray,
//! laid over the air, painted by a content shader the area names with what lies beyond the air
//! and in it (the weather's clouds, from the field of cloud drifting over the area, the moon and
//! the stars),
//! told where the sun or the moon stands, how deep the night is and the weather the covers read;
//! and the rain, shells round the eye, each a sphere the rain is drawn on, blended over the world
//! in front of which it falls, painted by a content shader the area's climate names.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use crate::core::render::Look;
use crate::core::render::world::clouds::{self, CLOUD_TILE, CloudLook, SECOND_SHIFT};
use crate::core::render::world::sky::AIR_WAYS;

use crate::core::render::camera::NEAR_PLANE;
use crate::core::units::Meters;
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};

/// The def a sky's shader is compiled under to be drawn into the sky's picture.
pub const SKY_PICTURE: &str = "SKY_PICTURE";

/// A sort distance past any the view holds: a blended mesh biased by it draws before every other.
const BEHIND_EVERYTHING: f32 = 1.0e9;

/// A shader painting the sky, with the parameters it is filled with, the weather it reads, the
/// sky's lights, and the field of cloud drifting over the area: on the dome round the camera,
/// or, as `picture`, spread over the face the sky's picture is drawn on.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(SkyShaderKey)]
pub struct SkyMaterial {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub heavens: Heavens,
    #[texture(104)]
    #[sampler(105)]
    pub clouds: Handle<Image>,
    pub shader: Handle<Shader>,
    pub picture: bool,
}

/// Which compiled shader a sky draws with, and which of its two ways it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SkyShaderKey {
    shader: VfxShaderKey,
    picture: bool,
}

/// The sky's lights as the sky header reads them: the unit way toward the light the world is lit
/// by (the sun, or by night the full moon), how much of it shines (none below the horizon), its
/// colour as it reaches the ground, the unit way toward the moon, how far into the night the sky
/// is, 0 day to 1 full night, how wide the moon's disc is, in radians, the unit way toward the
/// sun and its colour as it reaches the
/// clouds, how the sky has turned with its sun (what carries a way of the world to the way of
/// the sky's own it looks along), whether the stars twinkle, 1 or 0, how much air a light from
/// beyond it crosses along each of the air's ways up from the horizon, and the field of cloud:
/// the height it lies at, the ground a tile of it covers, how far the wind has carried it, where
/// in its noise cloud begins and how far past that it is thick, and how its second read is
/// carried, turned and scaled from its first; where in the noise cloud holds water, and how far
/// the wind carries the rain sideways for every metre it falls.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct Heavens {
    pub toward: Vec3,
    pub shining: f32,
    pub color: Vec4,
    pub moon: Vec3,
    pub night: f32,
    pub sun: Vec3,
    pub sun_color: Vec4,
    pub turned: Mat3,
    pub twinkling: f32,
    pub moon_size: f32,
    pub air: [Vec4; AIR_WAYS],
    pub cloud_base: f32,
    pub cloud_tile: f32,
    pub drift: Vec2,
    pub cloud_edge: f32,
    pub cloud_softness: f32,
    pub cloud_shift: Vec2,
    pub cloud_second: Mat2,
    pub water_edge: f32,
    pub rain_lean: Vec2,
}

impl Heavens {
    /// The world lit from `toward` in `color`, `strength` times as strong as the full sun, shining
    /// fully once the light clears the horizon; the sun standing toward `sun` and reaching the
    /// clouds in `sun_color`, `sun_strength` times as strong as at its fullest, the moon standing
    /// opposite it, and `night` of the way into the night, under clouds that look as `look` says.
    pub fn new(
        (toward, color, strength): (Vec3, Color, f32),
        (sun, sun_color, sun_strength): (Vec3, Color, f32),
        night: f32,
        look: &Look,
    ) -> Heavens {
        let shed = |color: Color, strength: f32| {
            let linear = color.to_linear() * strength;
            Vec4::new(linear.red, linear.green, linear.blue, 1.0)
        };
        Heavens {
            toward,
            shining: (toward.y * 10.0 + 0.5).clamp(0.0, 1.0),
            color: shed(color, strength),
            moon: -sun,
            night,
            sun,
            sun_color: shed(sun_color, sun_strength),
            ..Heavens::clear(look)
        }
    }

    /// These heavens turned as `turned` has their sky, their stars twinkling or held still.
    pub fn turned(self, turned: Mat3, twinkling: bool) -> Heavens {
        Heavens {
            turned,
            twinkling: f32::from(u8::from(twinkling)),
            ..self
        }
    }

    /// These heavens seen through air a light from beyond it crosses `crossed` of along each of
    /// the air's ways up from the horizon.
    pub fn through(self, crossed: [Vec3; AIR_WAYS]) -> Heavens {
        Heavens {
            air: crossed.map(|crossed| crossed.extend(0.0)),
            ..self
        }
    }

    /// These heavens under `weather`'s clouds, carried `drift` along: covering as much of the
    /// sky as they do, as much of them holding water, their rain carried as far.
    pub fn covered(self, weather: SkyWeather, drift: Vec2, look: &CloudLook) -> Heavens {
        Heavens {
            drift,
            cloud_edge: clouds::edge(weather.clouds, look),
            water_edge: clouds::water_edge(weather.water),
            rain_lean: weather.rain_lean,
            ..self
        }
    }

    /// A sky of no light under clouds that look as `look` says, covering none of it.
    pub fn clear(look: &Look) -> Heavens {
        let clouds = &look.clouds;
        Heavens {
            toward: Vec3::Y,
            shining: 0.0,
            color: Vec4::ZERO,
            moon: Vec3::NEG_Y,
            night: 0.0,
            sun: Vec3::Y,
            sun_color: Vec4::ZERO,
            turned: Mat3::IDENTITY,
            twinkling: 1.0,
            moon_size: look.sky.moon_size.radians().0,
            air: [Vec4::ZERO; AIR_WAYS],
            cloud_base: clouds.height.0,
            cloud_tile: CLOUD_TILE.0,
            drift: Vec2::ZERO,
            cloud_edge: clouds::edge(0.0, clouds),
            cloud_softness: clouds.softness,
            cloud_shift: SECOND_SHIFT,
            cloud_second: clouds::second_read(),
            water_edge: clouds::water_edge(0.0),
            rain_lean: Vec2::ZERO,
        }
    }
}

/// The weather a sky paints: how much of it cloud covers and how much of the cloud holds water,
/// each 0 to 1, and how far the wind carries the rain sideways for every metre it falls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyWeather {
    pub clouds: f32,
    pub water: f32,
    pub rain_lean: Vec2,
}

impl From<&SkyMaterial> for SkyShaderKey {
    fn from(sky: &SkyMaterial) -> SkyShaderKey {
        SkyShaderKey {
            shader: VfxShaderKey::of(&sky.shader),
            picture: sky.picture,
        }
    }
}

impl Material for SkyMaterial {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    /// The dome and the rain's shells both stand round the eye, so by distance alone their order
    /// is a toss-up, and the sky drawn second covers the rain wherever it shows. The sky lies
    /// behind everything blended, and is sorted so.
    fn depth_bias(&self) -> f32 {
        BEHIND_EVERYTHING
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let SkyShaderKey { shader, picture } = key.bind_group_data;
        let shader = shader.handle();
        descriptor.vertex.shader = shader.clone();
        if picture {
            descriptor.vertex.shader_defs.push(SKY_PICTURE.into());
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
            if picture {
                fragment.shader_defs.push(SKY_PICTURE.into());
            }
        }
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The dome the sky drawn with `material` is painted on, round whatever camera looks at it.
pub fn dome(meshes: &mut Assets<Mesh>, material: Handle<SkyMaterial>) -> impl Bundle {
    (
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 24))),
        MeshMaterial3d(material),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    )
}

/// How near the eye a shell may lie: wholly beyond the camera's near plane, which clips it, though
/// the facets of its sphere dip inside its radius.
pub const NEAREST_SHELL: Meters = Meters(NEAR_PLANE.0 * 2.0);

/// A shader drawing the rain on its shells, with the parameters it is filled with, the weather it
/// reads, the sky's lights and the field of cloud drifting over the area, as the sky's dome has.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(VfxShaderKey)]
pub struct RainMaterial {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub heavens: Heavens,
    #[texture(104)]
    #[sampler(105)]
    pub clouds: Handle<Image>,
    pub shader: Handle<Shader>,
}

impl From<&RainMaterial> for VfxShaderKey {
    fn from(rain: &RainMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&rain.shader)
    }
}

impl Material for RainMaterial {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The shells the rain drawn with `material` falls on, a sphere at each of `shells` round
/// whatever camera looks at them, in one mesh from the farthest in, so each pixel blends the
/// nearer rain over the farther.
pub fn shells(
    meshes: &mut Assets<Mesh>,
    shells: &[Meters],
    material: Handle<RainMaterial>,
) -> impl Bundle {
    (
        Mesh3d(meshes.add(shell_mesh(shells))),
        MeshMaterial3d(material),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    )
}

fn shell_mesh(shells: &[Meters]) -> Mesh {
    let mut farthest_first = shells.to_vec();
    farthest_first.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sphere = Sphere::new(1.0).mesh().uv(32, 16);
    let unit: Vec<[f32; 3]> = sphere
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|positions| positions.as_float3())
        .map(<[[f32; 3]]>::to_vec)
        .unwrap_or_default();
    let unit_indices: Vec<u32> = sphere
        .indices()
        .map(|indices| indices.iter().map(|index| index as u32).collect())
        .unwrap_or_default();
    let mut positions = Vec::with_capacity(unit.len() * farthest_first.len());
    let mut indices = Vec::with_capacity(unit_indices.len() * farthest_first.len());
    for radius in farthest_first {
        let first = positions.len() as u32;
        positions.extend(
            unit.iter()
                .map(|at| (Vec3::from_array(*at) * radius.0).to_array()),
        );
        indices.extend(unit_indices.iter().map(|index| first + index));
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices))
}
