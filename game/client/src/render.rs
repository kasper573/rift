use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite::Anchor;
use bevy::sprite_render::{Material2d, Material2dPlugin};
use bevy::window::PrimaryWindow;
use world::actors;
use world::area::{self, AreaId};
use world::math::{CellPos, Offset, Pos, Seconds, Size, Tiles, WorldPx};
use world::protocol::{Actor, AreaTag, Owner, Position, Rgba, action_name};
use world::session::{self, MyClient};

use crate::user_settings::ScreenPx;

/// World pixels per tile; the actor and tile sheets are authored at this scale.
pub const TILE: WorldPx = WorldPx(16.0);
const VIEW_TILES: Size<Tiles> = Size::new(24.0, 18.0);
const VIEW: Size<WorldPx> = Size::new(VIEW_TILES.width * TILE.0, VIEW_TILES.height * TILE.0);
/// The presentation quad lives on its own layer so only the presentation camera draws it.
const PRESENT_LAYER: usize = 1;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<DeadTint>::default())
            .init_resource::<Animator>()
            .init_resource::<SpawnedArea>()
            .init_resource::<Viewport>()
            .add_systems(Startup, setup)
            .add_observer(attach_sprite)
            .add_systems(Update, letterbox)
            .add_systems(
                Update,
                (
                    sync_actors,
                    follow_camera,
                    spawn_area_tiles,
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

#[derive(Component)]
struct Present;

/// The letterboxed placement of the view in the window — integer scale and centered offset —
/// shared with input mapping.
#[derive(Resource, Default, Clone, Copy)]
pub struct Viewport {
    /// Window logical pixels per world-render pixel.
    pub scale: f32,
    /// Where the letterboxed view's top-left sits in the window.
    pub offset: Offset<ScreenPx>,
}

/// Tints the presented frame toward red on death: additive red, green and blue at a third.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct DeadTint {
    #[texture(0)]
    #[sampler(1)]
    world: Handle<Image>,
    #[uniform(2)]
    dead: f32,
}

impl Material2d for DeadTint {
    fn fragment_shader() -> ShaderRef {
        "shaders/dead_tint.wgsl".into()
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DeadTint>>,
) {
    let mut target = Image::new_target_texture(
        VIEW.width as u32,
        VIEW.height as u32,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target.sampler = ImageSampler::nearest();
    let target = images.add(target);

    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: VIEW.width,
                height: VIEW.height,
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
        Mesh2d(meshes.add(Rectangle::new(VIEW.width, VIEW.height))),
        MeshMaterial2d(materials.add(DeadTint {
            world: target,
            dead: 0.0,
        })),
        RenderLayers::layer(PRESENT_LAYER),
        Present,
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

fn letterbox(
    window: Single<&Window, With<PrimaryWindow>>,
    mut quad: Query<&mut Transform, With<Present>>,
    mut viewport: ResMut<Viewport>,
) {
    let (width, height) = (window.resolution.width(), window.resolution.height());
    let scale = (width / VIEW.width)
        .min(height / VIEW.height)
        .floor()
        .max(1.0);
    viewport.scale = scale;
    viewport.offset = Offset::new(
        (width - VIEW.width * scale) / 2.0,
        (height - VIEW.height * scale) / 2.0,
    );
    for mut transform in &mut quad {
        transform.scale = Vec3::splat(scale);
    }
}

fn dead_tint(world: &mut World) {
    let dead = if session::is_dead(world) { 1.0 } else { 0.0 };
    let Ok(handle) = world
        .query_filtered::<&MeshMaterial2d<DeadTint>, With<Present>>()
        .single(world)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let mut materials = world.resource_mut::<Assets<DeadTint>>();
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
        sprite.rect = Some(Rect::new(
            region.origin.x,
            region.origin.y,
            region.origin.x + region.size.width,
            region.origin.y + region.size.height,
        ));
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
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    let Some(center) = camera_center(position.pos, tag.area) else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation.x = center.x * TILE.0;
        transform.translation.y = -center.y * TILE.0;
    }
}

/// The clamped, pixel-snapped point the camera centers on: the player, kept inside the area edges.
fn camera_center(at: Pos<Tiles>, area_id: AreaId) -> Option<Pos<Tiles>> {
    let area = area::areas().get(area_id.0 as usize)?;
    let half = VIEW_TILES * 0.5;
    let lo = Pos::new(half.width, half.height);
    let hi = Pos::new(
        (area.width.0 - half.width).max(half.width),
        (area.height.0 - half.height).max(half.height),
    );
    Some(snap(at.clamp(lo, hi)))
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<AreaId>);

#[derive(Component)]
struct AreaTile;

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
                let Some(sprite) = area.resolve(layer.at(c), Seconds(0.0)) else {
                    continue;
                };
                commands.spawn((
                    AreaTile,
                    tile_sprite(&assets, &sprite, Vec2::splat(TILE.0)),
                    sprite_transform(cell_center(c), z),
                ));
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
                commands.spawn((
                    AreaTile,
                    tile_sprite(&assets, &sprite, Vec2::splat(TILE.0)),
                    sprite_transform(cell_center(c), z),
                ));
            }
        }
        for &(pos, cell) in &area.objects {
            let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                continue;
            };
            let size = Vec2::new(sprite.region.size.width, sprite.region.size.height);
            commands.spawn((
                AreaTile,
                tile_sprite(&assets, &sprite, size),
                Anchor::BOTTOM_LEFT,
                sprite_transform(pos, dynamic_z(area, z, Tiles(pos.y))),
            ));
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
    let region = sprite.region;
    Sprite {
        image: assets.load(sprite.sheet.to_owned()),
        rect: Some(Rect::new(
            region.origin.x,
            region.origin.y,
            region.origin.x + region.size.width,
            region.origin.y + region.size.height,
        )),
        custom_size: Some(size),
        flip_x: sprite.flip.0,
        flip_y: sprite.flip.1,
        ..default()
    }
}

fn rgba(tint: Rgba) -> Color {
    let [r, g, b, a] = tint.0.to_be_bytes();
    Color::srgba_u8(r, g, b, a)
}

fn snap(p: Pos<Tiles>) -> Pos<Tiles> {
    let axis = |t: f32| (t * TILE.0).round() / TILE.0;
    Pos::new(axis(p.x), axis(p.y))
}
