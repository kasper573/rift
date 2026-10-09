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
use super::transition::{ActiveTransition, CutUniform, ScreenTransitionPhase};
use crate::core::time::Seconds;

const TILE_SCREEN: f32 = 96.0;
pub(crate) const SCALE: f32 = TILE_SCREEN / TILE.0;
const PRESENT_LAYER: usize = 1;

#[derive(Component)]
pub(super) struct Screen;

#[derive(Resource)]
pub(super) struct WorldTargets {
    live: Handle<Image>,
    held: Handle<Image>,
}

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
    #[texture(3)]
    snapshot: Handle<Image>,
    #[uniform(4)]
    cut: CutUniform,
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
    let target = images.add(world_target(target_w, target_h));
    let held = images.add(world_target(target_w, target_h));
    commands.insert_resource(WorldTargets {
        live: target.clone(),
        held: held.clone(),
    });

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
            snapshot: held,
            cut: CutUniform::IDLE,
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

pub(super) fn match_display(
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    platform: Res<crate::core::platform::ClientPlatform>,
) {
    platform.0.sync_window(&mut window);
}

pub(super) fn fit(
    window: Single<&Window, With<PrimaryWindow>>,
    targets: Res<WorldTargets>,
    mut images: ResMut<Assets<Image>>,
    mut projection: Query<&mut Projection, With<WorldCamera>>,
    mut quad: Query<&mut Transform, With<Screen>>,
    mut viewport: ResMut<Viewport>,
) {
    let (width, height) = (window.resolution.width(), window.resolution.height());
    let (target_w, target_h) = target_size(&window);
    viewport.scale = height / target_h as f32;
    let stale = images.get(&targets.live).is_some_and(|image| {
        image.texture_descriptor.size.width != target_w
            || image.texture_descriptor.size.height != target_h
    });
    if stale {
        for target in [&targets.live, &targets.held] {
            if let Some(mut image) = images.get_mut(target) {
                image.resize(Extent3d {
                    width: target_w,
                    height: target_h,
                    depth_or_array_layers: 1,
                });
            }
        }
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

pub(super) fn apply_present(
    time: Res<Time>,
    tint: Res<ScreenTint>,
    mut transition: ResMut<ActiveTransition>,
    mut phase: ResMut<ScreenTransitionPhase>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    screen: Query<&MeshMaterial2d<Present>, With<Screen>>,
    mut materials: ResMut<Assets<Present>>,
) {
    let (current, cut) = transition.advance(Seconds(time.delta_secs()), camera.single().ok());
    phase.set_if_neq(current);
    let Ok(handle) = screen.single().map(|material| material.0.clone()) else {
        return;
    };
    let unchanged = materials
        .get(&handle)
        .is_some_and(|material| material.tint == tint.0 && material.cut == cut);
    if unchanged {
        return;
    }
    if let Some(mut material) = materials.get_mut(&handle) {
        material.tint = tint.0;
        material.cut = cut;
    }
}

pub(super) fn hold_frame(world: &mut World) {
    let (live, held) = {
        let mut targets = world.resource_mut::<WorldTargets>();
        let targets = &mut *targets;
        std::mem::swap(&mut targets.live, &mut targets.held);
        (targets.live.clone(), targets.held.clone())
    };
    let mut cameras = world.query_filtered::<&mut RenderTarget, With<WorldCamera>>();
    for mut target in cameras.iter_mut(world) {
        *target = RenderTarget::Image(live.clone().into());
    }
    let screens: Vec<Handle<Present>> = world
        .query_filtered::<&MeshMaterial2d<Present>, With<Screen>>()
        .iter(world)
        .map(|material| material.0.clone())
        .collect();
    let mut materials = world.resource_mut::<Assets<Present>>();
    for handle in screens {
        if let Some(mut material) = materials.get_mut(&handle) {
            material.world = live.clone();
            material.snapshot = held.clone();
        }
    }
}

fn world_target(width: u32, height: u32) -> Image {
    let mut target = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    target.sampler = ImageSampler::linear();
    target
}

pub(crate) fn target_size(window: &Window) -> (u32, u32) {
    let scaled = |logical: f32| {
        let px = logical.round().max(2.0) as u32;
        px + (px & 1)
    };
    let res = &window.resolution;
    (scaled(res.width()), scaled(res.height()))
}
