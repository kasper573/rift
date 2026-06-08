//! Software-rasterizes the replicated world into an RGBA frame: macroquad-free, shared by
//! the game binary and the browser e2e (which derives click coordinates from this geometry).

use std::collections::HashMap;
use std::sync::LazyLock;

use image::{Image, Region};
use world::Entity;
use world::core::actors::ActorModelId;
use world::core::area::AreaId;
use world::core::math::{Pixels, Pos, Size, Tiles};
use world::core::protocol::Rgba;
use world::core::protocol::{Actor, Position, Vitals, action_name};
use world::core::session::MmoClient;
use world::core::{actors, area};

/// One tile, in pixels.
pub const TILE_SIZE: Size<Pixels> = Size::splat(Pixels(16.0));
/// The rendered viewport, in tiles.
pub const VIEW_TILES: Size<Tiles> = Size::new(Tiles(24.0), Tiles(18.0));
/// The rendered viewport, in pixels — one tile's worth of pixels across each tile of the view.
pub static VIEW: LazyLock<Size<Pixels>> = LazyLock::new(|| TILE_SIZE.mult(VIEW_TILES));
static HALF_VIEW: LazyLock<Pos<Pixels>> = LazyLock::new(|| VIEW.scale(0.5));
static HALF_VIEW_TILES: LazyLock<Size<Tiles>> = LazyLock::new(|| VIEW_TILES.scale(0.5));

// Inventory layout, anchored top-right: a grid of square slots.
pub const INV_GRID: Size<u32> = Size::new(4, 6);
pub const INV_SLOT: f32 = 36.0;
pub const INV_PAD: f32 = 8.0;

/// A sound source's volume for a listener, both in tiles: 1 at the listener, falling linearly to
/// 0 at the edge of the rendered view and staying 0 beyond it, so off-screen sources are silent.
pub fn proximity_volume(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    let half = *HALF_VIEW_TILES;
    let offset = source - listener;
    let dx = offset.x.0.abs() / half.x.0;
    let dy = offset.y.0.abs() / half.y.0;
    (1.0 - dx.max(dy)).clamp(0.0, 1.0)
}

/// A sound source's stereo pan for a listener: -1 (full left) at the left edge of the view, 0 at
/// the listener's column, +1 (full right) at the right edge; clamped, vertical offset ignored.
pub fn proximity_pan(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    ((source - listener).x.0 / HALF_VIEW_TILES.x.0).clamp(-1.0, 1.0)
}

/// Integer-scales the view into a screen and centers it: the scale, and the view's top-left offset.
pub fn letterbox(screen: Size<Pixels>) -> (f32, Pos<Pixels>) {
    let scale = (screen.x.0 / VIEW.x.0)
        .min(screen.y.0 / VIEW.y.0)
        .floor()
        .max(1.0);
    (scale, (screen - VIEW.scale(scale)).scale(0.5))
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub center: Pos<Tiles>,
}

pub fn camera(client: &MmoClient) -> Option<Camera> {
    camera_for(client.my_position()?, client.my_area()?)
}

pub fn camera_for(at: Pos<Tiles>, area: AreaId) -> Option<Camera> {
    let area = area::areas().get(area.0 as usize)?;
    let half = *HALF_VIEW_TILES;
    let extent = Size::new(area.width, area.height);
    let center = at.clamp(half, (extent - half).max(half)).map(snap);
    Some(Camera { center })
}

/// Anchors each entity's animation to the moment its replicated action last changed, so
/// [`ActorModel::frame`] receives time-into-action rather than the global clock.
///
/// [`ActorModel::frame`]: actor::ActorModel::frame
#[derive(Default)]
pub struct Animator {
    pub anchors: HashMap<Entity, (u8, f32)>,
}

impl Animator {
    pub fn elapsed(&mut self, entity: Entity, action: u8, time: f32) -> f32 {
        match self.anchors.get(&entity) {
            Some(&(seen, start)) if seen == action => time - start,
            _ => {
                self.anchors.insert(entity, (action, time));
                0.0
            }
        }
    }
}

pub struct Scene {
    camera: Option<Camera>,
    area: AreaId,
    time: f32,
    actors: Vec<ActorDraw>,
    dead: bool,
}

pub fn build_scene(client: &MmoClient, time: f32, animator: &mut Animator) -> Scene {
    let camera = camera(client);
    let area = client.my_area().unwrap_or(AreaId(0));
    let me = client.my_entity();
    let world = client.world();
    animator.anchors.retain(|&entity, _| world.alive(entity));
    let actors = world
        .iter::<Actor>()
        .filter_map(|(entity, actor)| {
            let position = world.get::<Position>(entity)?;
            let elapsed = animator.elapsed(entity, actor.action, time);
            Some(ActorDraw {
                pos: position.pos,
                region: model(actor.model).frame(
                    action_name(actor.action),
                    actor.dir,
                    elapsed,
                    actor.attack_rate.0,
                ),
                tint: actor.color,
                model: actor.model,
                health: (me == Some(entity))
                    .then(|| world.get::<Vitals>(entity))
                    .flatten()
                    .map(|v| (v.health / v.max).clamp(0.0, 1.0)),
            })
        })
        .collect();
    Scene {
        camera,
        area,
        time,
        actors,
        dead: client.is_dead(),
    }
}

/// Renders the scene into `frame`, reusing its allocation across frames.
pub fn rasterize(scene: &Scene, frame: &mut Image) {
    if (frame.width, frame.height) != (VIEW.x.0 as u32, VIEW.y.0 as u32) {
        *frame = Image::new(VIEW.x.0 as u32, VIEW.y.0 as u32);
    } else {
        frame.rgba.fill(0);
    }
    let Some(camera) = scene.camera else {
        return;
    };
    let area = &area::areas()[scene.area.0 as usize];

    let half = *HALF_VIEW_TILES;
    let margin = Size::splat(Tiles(1.0));
    let lo = (camera.center - half).map(f32::floor) - margin;
    let hi = (camera.center + half).map(f32::ceil) + margin;
    let (min_x, max_x) = (lo.x.0 as i32, hi.x.0 as i32);
    let (min_y, max_y) = (lo.y.0 as i32, hi.y.0 as i32);

    for tiles in area.map.tile_layers() {
        let is_dynamic = tiles.name.eq_ignore_ascii_case("Dynamic");
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                if is_dynamic && area.grouped_cells.contains(&(tx, ty)) {
                    continue;
                }
                let raw = tiles.at(tx, ty);
                if let Some((image, region, flip)) = area.tilesets.resolve(raw, scene.time) {
                    let dst = to_frame(camera, Pos::new(Tiles(tx as f32), Tiles(ty as f32)));
                    image::blit(frame, image, region, dst, TILE_SIZE, 0xFFFF_FFFF, flip);
                }
            }
        }
    }

    let mut draws: Vec<(f32, Draw)> = Vec::new();
    for &(at, gid) in &area.objects {
        let (tile_x, bottom_y) = (at.x.0, at.y.0);
        if tile_x < min_x as f32
            || tile_x > max_x as f32
            || bottom_y < min_y as f32
            || bottom_y > (max_y + 2) as f32
        {
            continue;
        }
        if let Some((image, region, flip)) = area.tilesets.resolve(gid, scene.time) {
            draws.push((
                bottom_y,
                Draw::Tile {
                    image,
                    region,
                    top_left: Pos::new(at.x, Tiles(bottom_y - 1.0)),
                    flip,
                },
            ));
        }
    }
    for actor in &scene.actors {
        draws.push((
            actor.pos.y.0,
            Draw::Actor {
                region: actor.region,
                pos: actor.pos,
                tint: actor.tint,
                model: actor.model,
                health: actor.health,
            },
        ));
    }
    for (index, group) in area.groups.iter().enumerate() {
        if group.z >= (min_y - 2) as f32 && group.z <= (max_y + 2) as f32 {
            draws.push((group.z, Draw::Group { group: index }));
        }
    }
    draws.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut bars: Vec<(Pos<Pixels>, f32)> = Vec::new();
    for (_, draw) in &draws {
        match draw {
            Draw::Tile {
                image,
                region,
                top_left,
                flip,
            } => {
                let dst = to_frame(camera, *top_left);
                image::blit(frame, image, *region, dst, TILE_SIZE, 0xFFFF_FFFF, *flip);
            }
            Draw::Actor {
                region,
                pos,
                tint,
                model,
                health,
            } => {
                let center = to_frame_f(camera, *pos);
                let anchor = Pos::new(
                    Pixels((region.size.x.0 / 2.0).round()),
                    Pixels((region.size.y.0 * 2.0 / 3.0).round()),
                );
                let dst = center.map(f32::round) - anchor;
                image::blit(
                    frame,
                    self::model(*model).image(),
                    *region,
                    dst,
                    region.size,
                    tint.0,
                    (false, false),
                );
                if let Some(fraction) = health {
                    bars.push((center, *fraction));
                }
            }
            Draw::Group { group } => {
                for &(tx, ty, raw) in &area.groups[*group].tiles {
                    if let Some((image, region, flip)) = area.tilesets.resolve(raw, scene.time) {
                        let dst = to_frame(camera, Pos::new(Tiles(tx as f32), Tiles(ty as f32)));
                        image::blit(frame, image, region, dst, TILE_SIZE, 0xFFFF_FFFF, flip);
                    }
                }
            }
        }
    }

    // Health bars are a HUD element: drawn after every world layer so an actor or object in front
    // (a higher z) can never hide them.
    for (center, fraction) in bars {
        health_bar(frame, center, fraction);
    }

    if scene.dead {
        tint_dead(frame);
    }
}

pub fn to_frame_f(camera: Camera, world: Pos<Tiles>) -> Pos<Pixels> {
    TILE_SIZE.mult(world - camera.center) + *HALF_VIEW
}

pub fn frame_to_world(camera: Camera, frame: Pos<Pixels>) -> Pos<Tiles> {
    (frame - *HALF_VIEW).convert(|pixels| pixels / TILE_SIZE.x.0) + camera.center
}

struct ActorDraw {
    pos: Pos<Tiles>,
    region: Region,
    tint: Rgba,
    model: ActorModelId,
    health: Option<f32>,
}

enum Draw {
    Tile {
        image: &'static Image,
        region: Region,
        top_left: Pos<Tiles>,
        flip: (bool, bool),
    },
    Actor {
        region: Region,
        pos: Pos<Tiles>,
        tint: Rgba,
        model: ActorModelId,
        health: Option<f32>,
    },

    Group {
        group: usize,
    },
}

const BAR_SIZE: Size<Pixels> = Size::new(Pixels(20.0), Pixels(4.0));
const BAR_BORDER: u32 = 0x140A_28FF;
const BAR_BG: u32 = 0x2A1C_5CFF;
const BAR_FILL: u32 = 0x00FF_00FF;

fn health_bar(frame: &mut Image, center: Pos<Pixels>, fraction: f32) {
    let top_left = center.map(f32::round) + Pos::new(Pixels(-BAR_SIZE.x.0 / 2.0), Pixels(3.0));
    image::fill(frame, top_left, BAR_SIZE, BAR_BORDER);
    let inner = top_left + Pos::splat(Pixels(1.0));
    let inner_size = BAR_SIZE - Size::splat(Pixels(2.0));
    image::fill(frame, inner, inner_size, BAR_BG);
    image::fill(
        frame,
        inner,
        Size::new(Pixels(inner_size.x.0 * fraction), inner_size.y),
        BAR_FILL,
    );
}

fn model(index: ActorModelId) -> &'static actor::ActorModel {
    &actors::models()[index.0 as usize]
}

fn to_frame(camera: Camera, world: Pos<Tiles>) -> Pos<Pixels> {
    to_frame_f(camera, world).map(f32::round)
}

fn snap(tiles: f32) -> f32 {
    (tiles * TILE_SIZE.x.0).round() / TILE_SIZE.x.0
}

fn tint_dead(frame: &mut Image) {
    for pixel in frame.rgba.chunks_mut(4) {
        pixel[0] = (pixel[0] as u32 + 160).min(255) as u8;
        pixel[1] /= 3;
        pixel[2] /= 3;
    }
}
