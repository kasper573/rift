//! The sky's picture: the sky as the eye sees it, the air's own under whatever is painted over
//! it, drawn small into one round picture of everything above the horizon. The air's light on
//! the world holds the air alone, so whatever must show the sky itself, a sheet of water that
//! mirrors it or a bank of fog lit by it, reads this picture: a cloud is in it, and the moon.
//!
//! A camera of the picture's own draws it, seeing nothing but a face on a layer of its own,
//! which whoever paints the sky gives a material that draws the sky spread over the picture
//! (`sky_way` in `sky_picture.wesl`). The camera stands where the eye it draws for stands and is
//! lit as that eye is, so what is painted reads the same light there as on the screen.
//!
//! The picture is kept at every coarser size too, each made from the one finer as soon as the
//! camera has drawn, so a reader takes the sky as widely round a way as it likes in one read:
//! all of a quarter of the sky for a fog bank's light, a blur as wide as a rough water throws
//! its mirror.

use bevy::app::PropagateOver;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::mip_generation::{
    MipGenerationJobs, MipGenerationPhaseId, MipGenerationPipelines, generate_mips_for_phase,
};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::core_pipeline::upscaling::upscaling;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::image::ImageSampler;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    PipelineCache, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, RenderApp};

pub struct SkyPicturePlugin;

impl Plugin for SkyPicturePlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "sky_picture.wesl");
        app.add_plugins(ExtractComponentPlugin::<SkyPicture>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(ExtractSchedule, blur_drawn)
            .add_systems(
                Core3d,
                blur_picture
                    .after(upscaling)
                    .after(Core3dSystems::PostProcess),
            );
    }
}

/// How many pixels across the picture is.
const SIDE: u32 = 256;
/// The picture is drawn once in this many frames: the sky is slow, and a camera's frame costs
/// the processor the same however little it draws. No fewer, since the renderer lets go of a
/// camera's own targets once it has stood still three frames, and would make them anew each time.
const DRAWN_EVERY: u8 = 2;
/// The first of the layers the pictures' faces stand on, past every layer a world is seen on.
const FIRST_LAYER: usize = 1024;
/// Before every camera that reads the picture.
const ORDER: isize = -64;

/// A camera drawing a sky's picture into `image`.
#[derive(Component, Clone, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct SkyPicture {
    pub image: Handle<Image>,
    since_drawn: u8,
}

/// The whole of a sky's picture as a reader binds it, every size of it: the image's own view is
/// the finest alone, which is all its camera may draw into.
pub fn every_size(picture: &GpuImage) -> TextureView {
    picture
        .texture
        .create_view(&TextureViewDescriptor::default())
}

/// The layers a picture of the sky seen on `seen_on` is drawn on: the camera's, and its face's.
pub fn layers(seen_on: &RenderLayers) -> RenderLayers {
    RenderLayers::from_layers(
        &seen_on
            .iter()
            .map(|layer| FIRST_LAYER + layer)
            .collect::<Vec<_>>(),
    )
}

/// The camera that draws a sky's picture of what stands on `layers`.
pub fn camera(images: &mut Assets<Image>, layers: RenderLayers) -> impl Bundle + use<> {
    let mut image = Image::new_target_texture(SIDE, SIDE, TextureFormat::Rgba16Float, None);
    image.sampler = ImageSampler::linear();
    image.data = None;
    image.texture_descriptor.mip_level_count = SIDE.ilog2() + 1;
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        mip_level_count: Some(1),
        ..default()
    });
    let image = images.add(image);
    (
        Camera3d::default(),
        Camera {
            order: ORDER,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        Hdr,
        Msaa::Off,
        Tonemapping::None,
        Transform::default(),
        layers,
        PropagateOver::<RenderLayers>::default(),
        SkyPicture {
            image,
            since_drawn: 0,
        },
    )
}

/// The face a sky's picture is drawn on, which covers the whole of it, for a material that
/// spreads the sky over it.
pub fn face(meshes: &mut Assets<Mesh>, layers: RenderLayers) -> impl Bundle + use<> {
    (
        Mesh3d(meshes.add(Rectangle::new(2.0, 2.0))),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
        layers,
        PropagateOver::<RenderLayers>::default(),
    )
}

/// Has `picture` draw the sky as `eye` sees it: from where it stands, under the light it is lit
/// by.
pub fn follow(world: &mut World, picture: Entity, eye: Entity) {
    let Some(stands) = world
        .get::<GlobalTransform>(eye)
        .map(GlobalTransform::translation)
    else {
        return;
    };
    let sky_light = world.get::<EnvironmentMapLight>(eye).cloned();
    let ambient = world.get::<AmbientLight>(eye).cloned();
    let Ok(mut picture) = world.get_entity_mut(picture) else {
        return;
    };
    if let Some(mut placed) = picture.get_mut::<Transform>() {
        placed.set_if_neq(Transform::from_translation(stands));
    }
    let lit = picture.get::<EnvironmentMapLight>();
    let same_light = match (lit, &sky_light) {
        (Some(lit), Some(light)) => {
            lit.diffuse_map == light.diffuse_map
                && lit.specular_map == light.specular_map
                && lit.intensity == light.intensity
        }
        (None, None) => true,
        _ => false,
    };
    if !same_light && let Some(light) = sky_light {
        picture.insert(light);
    }
    if let Some(ambient) = ambient {
        let same = picture.get::<AmbientLight>().is_some_and(|held| {
            held.color == ambient.color && held.brightness == ambient.brightness
        });
        if !same {
            picture.insert(ambient);
        }
    }
    let due = picture.get_mut::<SkyPicture>().is_some_and(|mut drawing| {
        drawing.since_drawn = (drawing.since_drawn + 1) % DRAWN_EVERY;
        drawing.since_drawn == 0
    });
    if let Some(mut camera) = picture.get_mut::<Camera>()
        && camera.is_active != due
    {
        camera.is_active = due;
    }
}

/// The pictures drawn this frame, whose coarser sizes are made once their cameras have drawn.
const BLURRED: MipGenerationPhaseId = MipGenerationPhaseId(0x736b_7970);

fn blur_drawn(
    pictures: Extract<Query<(&SkyPicture, &Camera)>>,
    mut jobs: ResMut<MipGenerationJobs>,
) {
    for (picture, camera) in &pictures {
        if camera.is_active {
            jobs.add(BLURRED, picture.image.id());
        }
    }
}

fn blur_picture(
    _picture: ViewQuery<&SkyPicture>,
    jobs: Res<MipGenerationJobs>,
    cache: Res<PipelineCache>,
    pipelines: Option<Res<MipGenerationPipelines>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    generate_mips_for_phase(BLURRED, &jobs, &cache, &pipelines, &images, &mut ctx);
}
