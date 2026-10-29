//! A ground's cover as a shader stage: what the material grows or lies under, laid out by the
//! engine from the paint, moved and painted by a content shader the ground names, which reads the
//! weather the engine publishes: the wind, and the bodies treading through.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::Shader;

use super::render::{ParamsUniform, SiteUniform, VfxShaderKey};
use crate::core::render::CameraRig;
use crate::core::render::world::sky::Daylight;

pub type CoverMaterial = ExtendedMaterial<StandardMaterial, CoverVfx>;

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct CoverVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    pub shader: Handle<Shader>,
}

/// The weather as every cover reads it: a picture a texel per value, painted over in place each
/// frame. The covers' materials stand unchanged while the wind blows and bodies tread through
/// them, so none of their meshes is drawn up again for it.
#[derive(Resource)]
pub struct WeatherImage(pub Handle<Image>);

impl FromWorld for WeatherImage {
    fn from_world(world: &mut World) -> WeatherImage {
        WeatherImage(
            world
                .resource_mut::<Assets<Image>>()
                .add(Weather::default().image()),
        )
    }
}

/// The weather as the cover header's `weather` reads it: the wind as a direction scaled by its
/// strength, the moment, how far through its day the game is and how clouded the sky, and the
/// bodies treading through, each a ground point and the radius it flattens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Weather {
    wind: Vec2,
    time: f32,
    treads: u32,
    daylight: f32,
    clouds: f32,
    water: f32,
    tread: [Vec4; TREADS],
}

impl Default for Weather {
    fn default() -> Weather {
        Weather {
            wind: Vec2::ZERO,
            time: 0.0,
            treads: 0,
            daylight: 0.5,
            clouds: 0.0,
            water: 0.0,
            tread: [Vec4::ZERO; TREADS],
        }
    }
}

impl Weather {
    fn image(&self) -> Image {
        Image::new(
            Extent3d {
                width: (2 + TREADS) as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.texels(),
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        )
    }

    /// The weather laid out as the header's `read_weather` reads it: the wind, the moment and
    /// the count of treads in the first texel, the daylight, the cloud and its water in the
    /// second, then a texel a tread.
    fn texels(&self) -> Vec<u8> {
        let mut texels = vec![
            Vec4::new(self.wind.x, self.wind.y, self.time, self.treads as f32),
            Vec4::new(self.daylight, self.clouds, self.water, 0.0),
        ];
        texels.extend(self.tread);
        texels
            .iter()
            .flat_map(Vec4::to_array)
            .flat_map(f32::to_le_bytes)
            .collect()
    }
}

/// The weather over the shown area as the client draws it: the wind as a direction over the
/// ground (x east, y south) scaled by its strength, from calm at 0 to a gale at 1, how much cloud
/// covers the sky and how much water the cloud holds.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Climate {
    pub wind: Vec2,
    pub clouds: f32,
    pub water: f32,
}

/// A body that bends the cover it walks through, `radius` metres around its feet.
#[derive(Component, Clone, Copy, Debug)]
pub struct Treads {
    pub radius: f32,
}

impl From<&CoverVfx> for VfxShaderKey {
    fn from(vfx: &CoverVfx) -> VfxShaderKey {
        VfxShaderKey::of(&vfx.shader)
    }
}

impl MaterialExtension for CoverVfx {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        Ok(())
    }
}

/// A cover's material, coloured by its vertices: blended, for a sheet or blades that write no
/// depth and take no ink, or masked, for strewn props that stand solid like any other. Both of a
/// face's sides are drawn and lit alike by the normal its piece carries, never turned over on the
/// far side, since foliage points its normals where its light should fall rather than off its faces.
pub fn material(
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
    weather: Handle<Image>,
    alpha_mode: AlphaMode,
) -> CoverMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            double_sided: false,
            cull_mode: None,
            alpha_mode,
            ..default()
        },
        extension: CoverVfx {
            site,
            params,
            weather,
            shader,
        },
    }
}

/// What the weather is made of: the moment, the shown area's climate and daylight, where the camera
/// looks and who treads the ground.
#[derive(SystemParam)]
pub(super) struct Sky<'w, 's> {
    time: Res<'w, Time>,
    climate: Option<Res<'w, Climate>>,
    daylight: Option<Res<'w, Daylight>>,
    rig: Option<Res<'w, CameraRig>>,
    treading: Query<'w, 's, (&'static GlobalTransform, &'static Treads)>,
}

/// Every frame the covers learn the wind, the moment and who treads through them.
pub(super) fn blow(
    sky: Sky,
    picture: Res<WeatherImage>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Weather>,
) {
    let Sky {
        time,
        climate,
        daylight,
        rig,
        treading,
    } = sky;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    let mut treads: Vec<(f32, Vec4)> = treading
        .iter()
        .map(|(transform, treads)| {
            let at = transform.translation();
            (
                at.distance_squared(subject),
                Vec4::new(at.x, at.z, treads.radius, 0.0),
            )
        })
        .collect();
    treads.sort_by(|a, b| a.0.total_cmp(&b.0));
    treads.truncate(TREADS);
    let mut weather = Weather::default();
    for (slot, (_, tread)) in treads.iter().enumerate() {
        weather.tread[slot] = *tread;
    }
    weather.treads = treads.len() as u32;
    if let Some(daylight) = daylight {
        weather.daylight = daylight.share();
    }
    if let Some(climate) = climate {
        weather.clouds = climate.clouds;
        weather.water = climate.water;
        weather.wind = climate.wind;
    }
    weather.time = time.elapsed_secs();
    if *last == weather {
        return;
    }
    *last = weather;
    if let Some(mut image) = images.get_mut(&picture.0) {
        image.data = Some(weather.texels());
    }
}
