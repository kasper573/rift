use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite::Anchor;
use bevy::sprite_render::{Material2d, Material2dPlugin};
use bevy::window::PrimaryWindow;
use world::area::{self, AreaDef, TileRef};
use world::math::{Pos, Size, WorldPx};
use world::protocol::{Actor, AreaTag, Owner, Position, Rgba, Vitals, action_name};
use world::session::{self, MyClient};
use world::table::Id;
use world::tiling::{Cell, GridDims, TileSize, Tiles};
use world::time::Seconds;

use crate::screen::ToScreen;

pub const TILE: WorldPx = WorldPx(16.0);
const VIEW_TILES_TALL: f32 = 18.0;
const VIEW_TALL: f32 = VIEW_TILES_TALL * TILE.0;
const PRESENT_LAYER: usize = 1;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<Present>::default())
            .init_resource::<Animator>()
            .init_resource::<SpawnedArea>()
            .init_resource::<Viewport>()
            .add_systems(Startup, setup)
            .add_observer(attach_sprite)
            .add_systems(Update, (track_canvas_size, fit).chain())
            .add_systems(
                Update,
                (
                    sync_actors,
                    follow_camera,
                    spawn_area_tiles,
                    animate_tiles,
                    dead_tint,
                    healthbar,
                )
                    .run_if(in_state(crate::Screen::Playing)),
            );
    }
}

#[derive(Component)]
pub struct WorldCamera;

#[derive(Component)]
struct Screen;

#[derive(Resource)]
struct WorldTarget(Handle<Image>);

#[derive(Resource, Default, Clone, Copy)]
pub struct Viewport {
    pub scale: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct Present {
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

fn setup(
    window: Single<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Present>>,
) {
    let mut target = Image::new_target_texture(
        target_width(&window),
        VIEW_TALL as u32,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
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
                width: target_width(&window) as f32,
                height: VIEW_TALL,
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

    commands.spawn((
        Bar::Border,
        bar_sprite(0x14_0A_28, BAR),
        Anchor::CENTER,
        hidden(),
    ));
    let inner = BAR - Size::splat(2.0);
    commands.spawn((
        Bar::Background,
        bar_sprite(0x2A_1C_5C, inner),
        Anchor::CENTER,
        hidden(),
    ));
    commands.spawn((
        Bar::Fill,
        bar_sprite(0x00_FF_00, inner),
        Anchor::CENTER_LEFT,
        hidden(),
    ));
}

#[derive(Component, Clone, Copy)]
enum Bar {
    Border,
    Background,
    Fill,
}

const BAR: Size<WorldPx> = Size::new(20.0, 4.0);
const BAR_DROP: WorldPx = WorldPx(5.0);

fn bar_sprite(rgb: u32, size: Size<WorldPx>) -> Sprite {
    let [_, r, g, b] = rgb.to_be_bytes();
    Sprite {
        color: Color::srgb_u8(r, g, b),
        custom_size: Some(Vec2::new(size.width, size.height)),
        ..default()
    }
}

fn hidden() -> (Transform, Visibility) {
    (Transform::from_xyz(0.0, 0.0, 200.0), Visibility::Hidden)
}

fn healthbar(world: &mut World) {
    let shown = session::me(world).and_then(|me| {
        // Whole pixels: avoid floor/ceil alternation on fractional offsets during movement.
        let at = me.get::<Position>()?.pos;
        let vitals = me.get::<Vitals>()?;
        (!vitals.is_dead() && vitals.max > 0.0).then(|| {
            (
                (at.to_screen() - Vec2::new(0.0, BAR_DROP.0)).round(),
                vitals.fraction(),
            )
        })
    });
    let mut bars = world.query::<(&Bar, &mut Transform, &mut Visibility, &mut Sprite)>();
    for (bar, mut transform, mut visibility, mut sprite) in bars.iter_mut(world) {
        let Some((center, fraction)) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;
        match bar {
            Bar::Border => transform.translation = center.extend(200.0),
            Bar::Background => transform.translation = center.extend(200.1),
            Bar::Fill => {
                let inner = BAR.width - 2.0;
                sprite.custom_size = Some(Vec2::new((inner * fraction).floor(), BAR.height - 2.0));
                transform.translation = Vec3::new(center.x - inner / 2.0, center.y, 200.2);
            }
        }
    }
}

/// Keeps the render resolution matched to the canvas's displayed size. Bevy binds to the existing
/// canvas but never sizes its backing buffer, so without this it stays the 300x150 HTML default and
/// the browser stretches it (blurry); driving the backing to the displayed pixels lets the pixel art
/// upscale crisply (and picks up window resizes).
fn track_canvas_size(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    use wasm_bindgen::JsCast;
    let Some(canvas) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector("#glcanvas").ok().flatten())
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        return;
    };
    let (width, height) = (canvas.client_width(), canvas.client_height());
    if width <= 0 || height <= 0 {
        return;
    }
    // winit doesn't resize a canvas we hand it, so set the backing buffer to the displayed pixels
    // ourselves; the matching `window.resolution` keeps bevy's render surface and camera in step.
    if canvas.width() != width as u32 {
        canvas.set_width(width as u32);
    }
    if canvas.height() != height as u32 {
        canvas.set_height(height as u32);
    }
    let (width, height) = (width as f32, height as f32);
    if window.resolution.width() != width || window.resolution.height() != height {
        window.resolution.set(width, height);
    }
}

fn fit(
    window: Single<&Window, With<PrimaryWindow>>,
    target: Res<WorldTarget>,
    mut images: ResMut<Assets<Image>>,
    mut projection: Query<&mut Projection, With<WorldCamera>>,
    mut quad: Query<&mut Transform, With<Screen>>,
    mut viewport: ResMut<Viewport>,
) {
    let (width, height) = (window.resolution.width(), window.resolution.height());
    viewport.scale = height / VIEW_TALL;
    let target_w = target_width(&window);
    if let Some(mut image) = images.get_mut(&target.0)
        && image.texture_descriptor.size.width != target_w
    {
        image.resize(Extent3d {
            width: target_w,
            height: VIEW_TALL as u32,
            depth_or_array_layers: 1,
        });
        if let Ok(mut proj) = projection.single_mut()
            && let Projection::Orthographic(ortho) = proj.as_mut()
        {
            ortho.scaling_mode = ScalingMode::Fixed {
                width: target_w as f32,
                height: VIEW_TALL,
            };
        }
    }
    if let Ok(mut transform) = quad.single_mut() {
        transform.scale = Vec3::new(width, height, 1.0);
    }
}

fn dead_tint(world: &mut World) {
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

#[derive(Resource, Default)]
pub struct Animator {
    anchors: HashMap<Entity, (u8, Seconds)>,
}

impl Animator {
    pub fn elapsed(&mut self, entity: Entity, action: u8, time: Seconds) -> Seconds {
        match self.anchors.get(&entity) {
            Some(&(seen, start)) if seen == action => time - start,
            _ => {
                self.anchors.insert(entity, (action, time));
                Seconds(0.0)
            }
        }
    }
}

fn attach_sprite(
    add: On<Add, Actor>,
    actors: Query<&Actor>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(actor) = actors.get(add.entity) else {
        return;
    };
    let image = assets.load(actor.model.get().sheet().to_owned());
    commands.entity(add.entity).insert((
        Sprite { image, ..default() },
        Anchor(Vec2::new(0.0, -1.0 / 6.0)),
        Transform::default(),
        Visibility::default(),
    ));
}

fn sync_actors(
    time: Res<Time>,
    mut animator: ResMut<Animator>,
    mut actors: Query<(
        Entity,
        &Actor,
        &Position,
        &AreaTag,
        &mut Sprite,
        &mut Transform,
    )>,
) {
    let clock = Seconds(time.elapsed_secs());
    animator
        .anchors
        .retain(|entity, _| actors.contains(*entity));
    for (entity, actor, position, tag, mut sprite, mut transform) in &mut actors {
        let elapsed = animator.elapsed(entity, actor.action, clock);
        let region = actor.model.get().frame(
            action_name(actor.action),
            actor.dir,
            elapsed,
            actor.attack_rate,
        );
        sprite.rect = Some(atlas_rect(region));
        sprite.custom_size = Some(Vec2::new(region.size.width, region.size.height));
        sprite.color = rgba(actor.color);
        let Some(area) = area::areas().get(tag.area.index()) else {
            continue;
        };
        *transform = sprite_transform(
            position.pos,
            dynamic_z(area, area.dynamic_layer() as f32, Tiles(position.pos.y)),
        );
    }
}

fn follow_camera(
    me: Res<MyClient>,
    players: Query<(&Owner, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    let Some(center) = camera_center(position.pos, tag.area, view_half(&window)) else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        let p = center.to_screen();
        transform.translation.x = p.x;
        transform.translation.y = p.y;
    }
}

fn camera_center(at: Pos<Tiles>, area_id: Id<AreaDef>, half: Vec2) -> Option<Pos<Tiles>> {
    let area = area::areas().get(area_id.index())?;
    let bounds = area.size.bounds();
    let lo = Pos::new(bounds.min().x + half.x, bounds.min().y + half.y);
    let hi = Pos::new(
        (bounds.max().x - half.x).max(lo.x),
        (bounds.max().y - half.y).max(lo.y),
    );
    Some(snap(at.clamp(lo, hi)))
}

// Even width keeps tile edges on whole texels; odd width would draw seams between tiles.
fn target_width(window: &Window) -> u32 {
    let aspect = window.resolution.width() / window.resolution.height();
    let width = (VIEW_TALL * aspect).round().max(1.0) as u32;
    width + (width & 1)
}

fn view_half(window: &Window) -> Vec2 {
    let aspect = window.resolution.width() / window.resolution.height();
    Vec2::new(0.5 * VIEW_TILES_TALL * aspect, 0.5 * VIEW_TILES_TALL)
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<Id<AreaDef>>);

#[derive(Component)]
struct AreaTile;

#[derive(Component)]
struct Animated(TileRef);

fn spawn_area_tiles(
    me: Res<MyClient>,
    players: Query<(&Owner, &AreaTag)>,
    assets: Res<AssetServer>,
    mut spawned: ResMut<SpawnedArea>,
    tiles: Query<Entity, With<AreaTile>>,
    mut commands: Commands,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some(area_id) = players
        .iter()
        .find(|(owner, _)| owner.client == my)
        .map(|(_, tag)| tag.area)
    else {
        return;
    };
    if spawned.0 == Some(area_id) {
        return;
    }
    for tile in &tiles {
        commands.entity(tile).despawn();
    }
    spawned.0 = Some(area_id);

    let area = &area::areas()[area_id.index()];
    for (index, layer) in area.layers.iter().enumerate() {
        let z = index as f32;
        for c in area.size.grid().cells() {
            if layer.dynamic && area.grouped_cells.contains(&c) {
                continue;
            }
            let cell = layer.at(c);
            let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                continue;
            };
            let mut tile = commands.spawn((
                AreaTile,
                tile_sprite(&assets, &sprite, Vec2::splat(TILE.0)),
                sprite_transform(c.center(), z),
            ));
            if area.animated(cell) {
                tile.insert(Animated(cell));
            }
        }
        if !layer.dynamic {
            continue;
        }
        for group in &area.groups {
            let z = dynamic_z(area, z, group.bottom);
            for &(c, cell) in &group.tiles {
                let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                    continue;
                };
                let mut tile = commands.spawn((
                    AreaTile,
                    tile_sprite(&assets, &sprite, Vec2::splat(TILE.0)),
                    sprite_transform(c.center(), z),
                ));
                if area.animated(cell) {
                    tile.insert(Animated(cell));
                }
            }
        }
        for &(pos, cell) in &area.objects {
            let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                continue;
            };
            let size = Vec2::new(sprite.region.size.width, sprite.region.size.height);
            let mut tile = commands.spawn((
                AreaTile,
                tile_sprite(&assets, &sprite, size),
                Anchor::BOTTOM_LEFT,
                sprite_transform(pos, dynamic_z(area, z, Tiles(pos.y))),
            ));
            if area.animated(cell) {
                tile.insert(Animated(cell));
            }
        }
    }
}

fn animate_tiles(
    time: Res<Time>,
    spawned: Res<SpawnedArea>,
    mut tiles: Query<(&Animated, &mut Sprite)>,
) {
    let Some(area_id) = spawned.0 else {
        return;
    };
    let area = &area::areas()[area_id.index()];
    let now = Seconds(time.elapsed_secs());
    for (animated, mut sprite) in &mut tiles {
        if let Some(resolved) = area.resolve(animated.0, now) {
            sprite.rect = Some(atlas_rect(resolved.region));
        }
    }
}

fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_translation(pos.to_screen().extend(z))
}

fn dynamic_z(area: &area::Area, base: f32, y: Tiles) -> f32 {
    base + (y + Tiles(1.0)).ratio(Tiles(area.size.height + 2.0))
}

fn tile_sprite(assets: &AssetServer, sprite: &area::TileSprite, size: Vec2) -> Sprite {
    Sprite {
        image: assets.load(sprite.sheet.to_owned()),
        rect: Some(atlas_rect(sprite.region)),
        custom_size: Some(size),
        flip_x: sprite.flip.x,
        flip_y: sprite.flip.y,
        ..default()
    }
}

fn atlas_rect(region: world::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.min().x,
        region.min().y,
        region.max().x,
        region.max().y,
    )
}

fn rgba(tint: Rgba) -> Color {
    let [r, g, b, a] = tint.0.to_be_bytes();
    Color::srgba_u8(r, g, b, a)
}

fn snap(p: Pos<Tiles>) -> Pos<Tiles> {
    let axis = |t: f32| (t * TILE.0).round() / TILE.0;
    Pos::new(axis(p.x), axis(p.y))
}
