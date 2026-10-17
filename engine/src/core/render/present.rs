use bevy::asset::{AssetPath, embedded_path};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::Material2d;
use bevy::window::PrimaryWindow;

use super::TILE;
use super::camera::WorldCamera;
use super::transition::{ActiveTransition, CutUniform};
use crate::core::platform::ClientPlatform;
use crate::core::time::Seconds;

const TILE_SCREEN: f32 = 96.0;
pub(crate) const SCALE: f32 = TILE_SCREEN / TILE.0;
pub(crate) const PRESENT_LAYER: usize = 1;

#[derive(Component)]
pub(super) struct Screen;

#[derive(Resource, Clone)]
pub(super) struct WorldTargets {
    live: Handle<Image>,
    held: Handle<Image>,
    size: (u32, u32),
    held_size: (u32, u32),
}

impl WorldTargets {
    fn new(images: &mut Assets<Image>, (width, height): (u32, u32)) -> WorldTargets {
        WorldTargets {
            live: images.add(world_target(width, height)),
            held: images.add(world_target(width, height)),
            size: (width, height),
            held_size: (width, height),
        }
    }

    fn bind(
        &self,
        target: &mut RenderTarget,
        screen: &Handle<Present>,
        materials: &mut Assets<Present>,
    ) {
        *target = RenderTarget::Image(self.live.clone().into());
        if let Some(mut material) = materials.get_mut(screen) {
            material.world = self.live.clone();
            material.snapshot = self.held.clone();
        }
    }

    fn swap_held(&mut self, images: &mut Assets<Image>) {
        std::mem::swap(&mut self.live, &mut self.held);
        let held_was = std::mem::replace(&mut self.held_size, self.size);
        if held_was != self.size {
            self.live = images.add(world_target(self.size.0, self.size.1));
        }
    }
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
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("present.wesl")).with_source("embedded"),
        )
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
    let targets = WorldTargets::new(&mut images, (target_w, target_h));
    let screen = materials.add(Present {
        world: Handle::default(),
        tint: Vec4::ZERO,
        snapshot: Handle::default(),
        cut: CutUniform::IDLE,
    });
    let mut camera_target = RenderTarget::default();
    targets.bind(&mut camera_target, &screen, &mut materials);
    commands.insert_resource(targets);

    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        camera_target,
        world_projection(target_w, target_h),
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
        Msaa::Off,
        bevy_egui::PrimaryEguiContext,
    ));

    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(screen),
        Transform::from_scale(Vec3::new(
            window.resolution.width(),
            window.resolution.height(),
            1.0,
        )),
        RenderLayers::layer(PRESENT_LAYER),
        Screen,
    ));
}

pub(crate) fn world_texture(world: &World) -> Option<Handle<Image>> {
    world
        .get_resource::<WorldTargets>()
        .map(|targets| targets.live.clone())
}

pub(super) fn match_display(
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    platform: Res<ClientPlatform>,
) {
    platform.0.sync_window(&mut window);
}

pub(super) fn fit(
    window: Single<&Window, With<PrimaryWindow>>,
    mut targets: ResMut<WorldTargets>,
    mut images: ResMut<Assets<Image>>,
    mut camera: Query<(&mut RenderTarget, &mut Projection), With<WorldCamera>>,
    mut quad: Query<(&mut Transform, &MeshMaterial2d<Present>), With<Screen>>,
    mut materials: ResMut<Assets<Present>>,
    mut viewport: ResMut<Viewport>,
) {
    let (width, height) = (window.resolution.width(), window.resolution.height());
    let (target_w, target_h) = target_size(&window);
    viewport.scale = height / target_h as f32;
    if targets.size != (target_w, target_h) {
        // A fresh live texture rather than a resized one: the screen's bind group would keep the old
        // texture otherwise. The held frame keeps its size and is stretched over the window, so a
        // transition in progress plays on from the frame it covered.
        targets.size = (target_w, target_h);
        targets.live = images.add(world_target(target_w, target_h));
        if let Ok((mut target, mut projection)) = camera.single_mut() {
            *projection = world_projection(target_w, target_h);
            if let Ok((_, screen)) = quad.single() {
                targets.bind(&mut target, &screen.0, &mut materials);
            }
        }
    }
    if let Ok((mut transform, _)) = quad.single_mut() {
        transform.scale = Vec3::new(width, height, 1.0);
    }
}

pub(super) fn apply_present(
    time: Res<Time>,
    tint: Res<ScreenTint>,
    mut transition: ResMut<ActiveTransition>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    screen: Query<&MeshMaterial2d<Present>, With<Screen>>,
    mut materials: ResMut<Assets<Present>>,
) {
    let cut = transition.advance(Seconds(time.delta_secs()), camera.single().ok());
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
    world.resource_scope(|world, mut targets: Mut<WorldTargets>| {
        let mut images = world.resource_mut::<Assets<Image>>();
        targets.swap_held(&mut images);
    });
    let targets = world.resource::<WorldTargets>().clone();
    let screens: Vec<Handle<Present>> = world
        .query_filtered::<&MeshMaterial2d<Present>, With<Screen>>()
        .iter(world)
        .map(|material| material.0.clone())
        .collect();
    world.resource_scope(|world, mut materials: Mut<Assets<Present>>| {
        let mut cameras = world.query_filtered::<&mut RenderTarget, With<WorldCamera>>();
        for mut target in cameras.iter_mut(world) {
            for screen in &screens {
                targets.bind(&mut target, screen, &mut materials);
            }
        }
    });
}

fn world_projection(width: u32, height: u32) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::Fixed {
            width: width as f32 / SCALE,
            height: height as f32 / SCALE,
        },
        ..OrthographicProjection::default_2d()
    })
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
