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
use world::actors;
use world::area::{self, AreaId, TileRef};
use world::math::{CellPos, Pos, Seconds, Size, Tiles, WorldPx};
use world::protocol::{Actor, AreaTag, Owner, Position, Rgba, action_name};
use world::session::{self, MyClient};

/// World pixels per tile; the actor and tile sheets are authored at this scale.
pub const TILE: WorldPx = WorldPx(16.0);
/// The view is locked to this many tiles tall on every display; the width fills the window, so a
/// larger or higher-resolution screen zooms in rather than revealing more of the map.
const VIEW_TILES_TALL: f32 = 18.0;
const VIEW_TALL: f32 = VIEW_TILES_TALL * TILE.0;
/// The present quad lives on its own layer so only the presentation camera draws it.
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
            .add_systems(Update, fit)
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

/// The camera that renders the world into the offscreen target; it follows the player.
#[derive(Component)]
pub struct WorldCamera;

/// The full-window quad that draws the offscreen target, upscaled, into the window.
#[derive(Component)]
struct Screen;

/// The offscreen target the world is rendered into, resized to the window's aspect.
#[derive(Resource)]
struct WorldTarget(Handle<Image>);

/// The zoom that fits the fixed-height world view to the window, shared with cursor mapping.
#[derive(Resource, Default, Clone, Copy)]
pub struct Viewport {
    /// Window pixels per world-render pixel: the on-screen size of one world pixel.
    pub scale: f32,
}

/// Presents the offscreen world render to the window: a sharp-bilinear upscale (crisp pixels at any
/// zoom) plus a death tint — additive red, green and blue cut to a third.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct Present {
    #[texture(0)]
    #[sampler(1)]
    world: Handle<Image>,
    #[uniform(2)]
    dead: f32,
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
            dead: 0.0,
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

/// The healthbar geometry in world pixels; it hangs just below the player's position point.
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

/// Tracks the local player's healthbar below them; hidden while dead or unspawned.
fn healthbar(world: &mut World) {
    let shown = match (session::my_position(world), session::my_vitals(world)) {
        // Whole pixels: at fractional offsets the fractional-width fill would alternate between
        // floor and ceil pixel coverage as the player moves.
        (Some(at), Some((health, max))) if health > 0.0 && max > 0.0 => Some((
            Vec2::new(at.x * TILE.0, -at.y * TILE.0 - BAR_DROP.0).round(),
            (health / max).clamp(0.0, 1.0),
        )),
        _ => None,
    };
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

/// Keeps the offscreen target at the window's aspect (fixed height, fill width) and the present
/// quad covering the window, so the world fills the viewport at a single resolution-driven zoom.
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

/// Anchors each entity's animation to the moment its replicated action last changed, so the model
/// receives time-into-action rather than the global clock.
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

/// Replicated actors are render entities: attach a sprite as the [`Actor`] component lands.
fn attach_sprite(
    add: On<Add, Actor>,
    actors: Query<&Actor>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(actor) = actors.get(add.entity) else {
        return;
    };
    let image = assets.load(actors::model(actor.model).sheet().to_owned());
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
        let region = actors::model(actor.model).frame(
            action_name(actor.action),
            actor.dir,
            elapsed,
            actor.attack_rate,
        );
        sprite.rect = Some(atlas_rect(region));
        sprite.custom_size = Some(Vec2::new(region.size.width, region.size.height));
        sprite.color = rgba(actor.color);
        let Some(area) = area::areas().get(tag.area.0 as usize) else {
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
        transform.translation.x = center.x * TILE.0;
        transform.translation.y = -center.y * TILE.0;
    }
}

/// The clamped, pixel-snapped point the camera centers on: the player, kept inside the area edges.
fn camera_center(at: Pos<Tiles>, area_id: AreaId, half: Vec2) -> Option<Pos<Tiles>> {
    let area = area::areas().get(area_id.0 as usize)?;
    let lo = Pos::new(half.x, half.y);
    let hi = Pos::new(
        (area.width.0 - half.x).max(half.x),
        (area.height.0 - half.y).max(half.y),
    );
    Some(snap(at.clamp(lo, hi)))
}

/// The offscreen target's width in world pixels: fixed height, window aspect, rounded to even. An
/// odd width centers the camera on a half-pixel, knocking the world render off the texel grid and
/// drawing seams between tiles; an even width keeps tile edges on whole texels.
fn target_width(window: &Window) -> u32 {
    let aspect = window.resolution.width() / window.resolution.height();
    let width = (VIEW_TALL * aspect).round().max(1.0) as u32;
    width + (width & 1)
}

/// Half the visible world extent in tiles — vertical fixed, horizontal tracking the window.
fn view_half(window: &Window) -> Vec2 {
    let aspect = window.resolution.width() / window.resolution.height();
    Vec2::new(0.5 * VIEW_TILES_TALL * aspect, 0.5 * VIEW_TILES_TALL)
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<AreaId>);

#[derive(Component)]
struct AreaTile;

/// An area tile whose tileset animates; [`animate_tiles`] re-resolves its frame each tick.
#[derive(Component)]
struct Animated(TileRef);

/// Spawns the static sprites of the player's area once, replacing them when the area changes.
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

    let area = &area::areas()[area_id.0 as usize];
    for (index, layer) in area.layers.iter().enumerate() {
        let z = index as f32;
        for y in 0..area.height.0 as i32 {
            for x in 0..area.width.0 as i32 {
                let c = CellPos::new(x, y);
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
                    sprite_transform(cell_center(c), z),
                ));
                if area.animated(cell) {
                    tile.insert(Animated(cell));
                }
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
                    sprite_transform(cell_center(c), z),
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

/// Advances each animated area tile's frame against the world clock; static tiles carry no
/// [`Animated`] and are left untouched.
fn animate_tiles(
    time: Res<Time>,
    spawned: Res<SpawnedArea>,
    mut tiles: Query<(&Animated, &mut Sprite)>,
) {
    let Some(area_id) = spawned.0 else {
        return;
    };
    let area = &area::areas()[area_id.0 as usize];
    let now = Seconds(time.elapsed_secs());
    for (animated, mut sprite) in &mut tiles {
        if let Some(resolved) = area.resolve(animated.0, now) {
            sprite.rect = Some(atlas_rect(resolved.region));
        }
    }
}

fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_xyz(pos.x * TILE.0, -pos.y * TILE.0, z)
}

fn cell_center(c: CellPos) -> Pos<Tiles> {
    Pos::new(c.x as f32 + 0.5, c.y as f32 + 0.5)
}

/// The z of a y-sorted child of the dynamic layer at `base`: strictly inside the band
/// `(base, base + 1)`, above the layer's flat cells and below the next layer, ordered by `y` —
/// an actor's position, a tile group's bottom row, or a tile object's bottom edge.
fn dynamic_z(area: &area::Area, base: f32, y: Tiles) -> f32 {
    base + (y.0 + 1.0) / (area.height.0 + 2.0)
}

fn tile_sprite(assets: &AssetServer, sprite: &area::TileSprite, size: Vec2) -> Sprite {
    Sprite {
        image: assets.load(sprite.sheet.to_owned()),
        rect: Some(atlas_rect(sprite.region)),
        custom_size: Some(size),
        flip_x: sprite.flip.0,
        flip_y: sprite.flip.1,
        ..default()
    }
}

/// A tileset/sheet pixel region as the texture sub-rect a `Sprite` samples.
fn atlas_rect(region: world::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.origin.x,
        region.origin.y,
        region.origin.x + region.size.width,
        region.origin.y + region.size.height,
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
