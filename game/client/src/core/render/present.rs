use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::Material2d;
use bevy::window::PrimaryWindow;

use super::TILE;
use super::camera::WorldCamera;

const TILE_SCREEN: f32 = 96.0;
pub(crate) const SCALE: f32 = TILE_SCREEN / TILE.0;
const PRESENT_LAYER: usize = 1;

#[derive(Component)]
pub(super) struct Screen;

#[derive(Resource)]
pub(super) struct WorldTarget(Handle<Image>);

#[derive(Resource, Default, Clone, Copy)]
pub struct Viewport {
    pub scale: f32,
}

#[derive(Resource, Default)]
pub struct ScreenTint(pub Vec4);

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(super) struct Present {
    #[texture(0)]
    #[sampler(1)]
    world: Handle<Image>,
    #[uniform(2)]
    tint: Vec4,
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
                width: target_w as f32 / SCALE,
                height: target_h as f32 / SCALE,
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
            tint: Vec4::ZERO,
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

pub(super) fn match_display(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    crate::core::platform::sync_window(&mut window);
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
                width: target_w as f32 / SCALE,
                height: target_h as f32 / SCALE,
            };
        }
    }
    if let Ok(mut transform) = quad.single_mut() {
        transform.scale = Vec3::new(width, height, 1.0);
    }
}

pub(super) fn apply_tint(
    tint: Res<ScreenTint>,
    screen: Query<&MeshMaterial2d<Present>, With<Screen>>,
    mut materials: ResMut<Assets<Present>>,
) {
    let Ok(handle) = screen.single().map(|material| material.0.clone()) else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&handle) {
        material.tint = tint.0;
    }
}

pub(crate) fn target_size(window: &Window) -> (u32, u32) {
    let scaled = |logical: f32| {
        let px = logical.round().max(2.0) as u32;
        px + (px & 1)
    };
    let res = &window.resolution;
    (scaled(res.width()), scaled(res.height()))
}
