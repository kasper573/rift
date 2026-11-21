//! A body of water as it is drawn: a sheet lying on the water's surface wherever it stands over
//! the ground, drawn after everything solid over a copy of the picture behind it, through the
//! content shader the water names. What the water is like from within, how fast it takes each
//! colour out of the light crossing it and what it sends back, is worked out here from the few
//! plain things said of it, and handed to the sheet and to the ground under it as a table.

use bevy::asset::RenderAssetUsages;
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

use crate::core::units::Meters;
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};

/// A water's material: the engine's lit surface under the content shader the water names.
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterVfx>;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct WaterVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub board: Handle<Image>,
    pub shader: Handle<Shader>,
}

impl From<&WaterVfx> for VfxShaderKey {
    fn from(vfx: &WaterVfx) -> VfxShaderKey {
        VfxShaderKey::of(&vfx.shader)
    }
}

impl MaterialExtension for WaterVfx {
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

/// A water's material: solid, since its sheet draws over a copy of what lies behind it and
/// shows that through itself, and writing its depth, so what is drawn after it lies on the
/// surface.
pub fn material(
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
    board: Handle<Image>,
) -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            cull_mode: None,
            alpha_mode: AlphaMode::Opaque,
            ..default()
        },
        extension: WaterVfx {
            site,
            params,
            board,
            shader,
        },
    }
}

/// How far from the shore a sheet on water tells its points they lie at most: past it, every
/// point lies this far out.
pub const SHORE_REACH: Meters = Meters(16.0);

/// What a body of water is like from within: how deep a white bed still shows through it looking
/// straight down, the colour of its depths under a white light, and how much light it sends
/// back from within, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterLook {
    pub clarity: Meters,
    pub depths: LinearRgba,
    pub murk: f32,
}

/// How many lengths of a water's fading leave a twentieth of a thing's contrast, which is where
/// it stops showing: the measure visibility is told by, in water as in air.
pub const SEEN_LENGTHS: f32 = 3.0;

/// The least a colour of a water's depths counts as, of its strongest: a colour the depths hold
/// none of still crosses a little water.
const LEAST_KEPT: f32 = 1.0 / 64.0;

impl WaterLook {
    /// How fast the water takes each colour out of the light crossing it, a metre: the colour its
    /// depths hold most of is taken slowest, just fast enough that a white bed `clarity` under
    /// it, the light going down and coming back, is left a twentieth of its contrast. The
    /// others are taken faster the less the depths hold of them.
    pub fn fading(&self) -> Vec3 {
        let depths = Vec3::new(self.depths.red, self.depths.green, self.depths.blue);
        let slowest = SEEN_LENGTHS / (2.0 * self.clarity.0.max(f32::EPSILON));
        let strongest = depths.max_element().max(f32::EPSILON);
        let kept = (depths / strongest).max(Vec3::splat(LEAST_KEPT));
        Vec3::splat(slowest) / Vec3::new(kept.x.sqrt(), kept.y.sqrt(), kept.z.sqrt())
    }

    /// What is left of a white light after `path` metres of the water, each colour.
    pub fn left_after(&self, path: Meters) -> Vec3 {
        (-self.fading() * path.0.max(0.0)).exp()
    }
}

/// Each water's fading and murk in the first row, and the colour of its depths with its clarity
/// in the second: a clear, colourless water where there is none.
pub fn looks_image(waters: &[WaterLook]) -> Image {
    let none = [WaterLook {
        clarity: Meters(f32::MAX),
        depths: LinearRgba::WHITE,
        murk: 0.0,
    }];
    let waters = if waters.is_empty() { &none } else { waters };
    let fading = waters.iter().map(|water| {
        let fading = water.fading();
        [fading.x, fading.y, fading.z, water.murk]
    });
    let depths = waters.iter().map(|water| {
        [
            water.depths.red,
            water.depths.green,
            water.depths.blue,
            water.clarity.0,
        ]
    });
    Image::new(
        Extent3d {
            width: waters.len() as u32,
            height: 2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        fading
            .chain(depths)
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect(),
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}
