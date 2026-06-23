//! The pixel-art present pipeline: the world renders to a fixed-zoom offscreen texture, which a
//! fullscreen quad then upscales to the window. `fit` keeps the texture sized to the window, and the
//! `Present` material applies the death tint.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::Material2d;
use bevy::window::PrimaryWindow;
use world::systems::player::session;

use super::TILE;
use super::camera::WorldCamera;

// The fixed zoom: every tile is drawn this many logical pixels across on every device, so the world
// looks the same size to every player. A larger display just frames more of the map — never bigger
// tiles — and network AOI culling bounds what's actually streamed. (48 keeps a ~900px-tall view near
// the game's long-standing 18-tiles-tall look, and being a multiple of TILE upscales crisply.)
const TILE_SCREEN: f32 = 48.0;
const PRESENT_LAYER: usize = 1;

#[derive(Component)]
pub(super) struct Screen;

#[derive(Resource)]
pub(super) struct WorldTarget(Handle<Image>);

#[derive(Resource, Default, Clone, Copy)]
pub struct Viewport {
    pub scale: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(super) struct Present {
    #[texture(0)]
    #[sampler(1)]
    world: Handle<Image>,
    // WebGL2 requires uniform buffer bindings to be 16-byte aligned, so this death-tint flag rides in
    // `.x` of a Vec4 rather than a bare f32 (which the browser's GL backend rejects at pipeline creation).
    #[uniform(2)]
    dead: Vec4,
}

impl Material2d for Present {
    fn fragment_shader() -> ShaderRef {
        "shaders/present.wgsl".into()
    }
}

pub(super) fn setup(
    window: Single<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Present>>,
) {
    let (target_w, target_h) = target_size(&window);
    let mut target =
        Image::new_target_texture(target_w, target_h, TextureFormat::Rgba8UnormSrgb, None);
    target.sampler = ImageSampler::linear();
    let target = images.add(target);
    commands.insert_resource(WorldTarget(target.clone()));

    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: target_w as f32,
                height: target_h as f32,
            },
            ..OrthographicProjection::default_2d()
        }),
        Msaa::Off,
        WorldCamera,
    ));

    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        RenderLayers::layer(PRESENT_LAYER),
        IsDefaultUiCamera,
    ));

    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(materials.add(Present {
            world: target,
            dead: Vec4::ZERO,
        })),
        Transform::from_scale(Vec3::new(
            window.resolution.width(),
            window.resolution.height(),
            1.0,
        )),
        RenderLayers::layer(PRESENT_LAYER),
        Screen,
    ));
}

/// Re-runs the platform's window sync each frame so the render target tracks canvas resizes; `fit`
/// then resizes the render target from the updated window size.
pub(super) fn match_display(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    crate::platform::sync_window(&mut window);
}

pub(super) fn fit(
    window: Single<&Window, With<PrimaryWindow>>,
    target: Res<WorldTarget>,
    mut images: ResMut<Assets<Image>>,
    mut projection: Query<&mut Projection, With<WorldCamera>>,
    mut quad: Query<&mut Transform, With<Screen>>,
    mut viewport: ResMut<Viewport>,
) {
    let (width, height) = (window.resolution.width(), window.resolution.height());
    let (target_w, target_h) = target_size(&window);
    viewport.scale = height / target_h as f32;
    if let Some(mut image) = images.get_mut(&target.0)
        && (image.texture_descriptor.size.width != target_w
            || image.texture_descriptor.size.height != target_h)
    {
        image.resize(Extent3d {
            width: target_w,
            height: target_h,
            depth_or_array_layers: 1,
        });
        if let Ok(mut proj) = projection.single_mut()
            && let Projection::Orthographic(ortho) = proj.as_mut()
        {
            ortho.scaling_mode = ScalingMode::Fixed {
                width: target_w as f32,
                height: target_h as f32,
            };
        }
    }
    if let Ok(mut transform) = quad.single_mut() {
        transform.scale = Vec3::new(width, height, 1.0);
    }
}

pub(super) fn dead_tint(world: &mut World) {
    let dead = if session::is_dead(world) { 1.0 } else { 0.0 };
    let dead = Vec4::new(dead, 0.0, 0.0, 0.0);
    let Ok(handle) = world
        .query_filtered::<&MeshMaterial2d<Present>, With<Screen>>()
        .single(world)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let mut materials = world.resource_mut::<Assets<Present>>();
    if let Some(mut material) = materials.get_mut(&handle) {
        material.dead = dead;
    }
}

// The render target is the visible world in native pixels: the window's logical size scaled down by
// the fixed per-tile zoom, so the present step upscales each source pixel by the same factor on every
// device. Even dimensions keep tile edges on whole texels; an odd one would draw seams between tiles.
pub(super) fn target_size(window: &Window) -> (u32, u32) {
    let scaled = |logical: f32| {
        let px = (logical * TILE.0 / TILE_SCREEN).round().max(2.0) as u32;
        px + (px & 1)
    };
    let res = &window.resolution;
    (scaled(res.width()), scaled(res.height()))
}
