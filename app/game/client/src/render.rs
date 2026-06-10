//! Renders the replicated world with macroquad into a fixed-size pixel view, then presents it
//! integer-scaled and letterboxed; the geometry helpers double as the input mapping.

use std::collections::HashMap;
use std::sync::LazyLock;

use macroquad::prelude::*;
use world::Entity;
use world::core::actors::ActorModelId;
use world::core::area::AreaId;
use world::core::math::{Pixels, Pos, Rect, Size, Tiles};
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
/// [`ActorModel::frame`]: actors::ActorModel::frame
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

/// The view's drawing state, reused across frames: the offscreen pixel target the world renders
/// into, the GPU textures of every atlas, and the death tint.
pub struct WorldView {
    pub animator: Animator,
    target: RenderTarget,
    textures: Textures,
    dead_tint: Material,
}

impl WorldView {
    pub fn new() -> WorldView {
        let target = render_target(VIEW.x.0 as u32, VIEW.y.0 as u32);
        target.texture.set_filter(FilterMode::Nearest);
        WorldView {
            animator: Animator::default(),
            target,
            textures: Textures::default(),
            dead_tint: dead_tint_material(),
        }
    }
}

impl Default for WorldView {
    fn default() -> WorldView {
        WorldView::new()
    }
}

/// Renders the scene into the view's pixel target, then presents it onto the screen at the
/// letterboxed integer scale; dead scenes pass through the red death tint.
pub fn present(scene: &Scene, view: &mut WorldView, scale: f32, offset: Pos<Pixels>) {
    let mut camera =
        Camera2D::from_display_rect(macroquad::math::Rect::new(0.0, 0.0, VIEW.x.0, VIEW.y.0));
    camera.render_target = Some(view.target.clone());
    set_camera(&camera);
    clear_background(Color::from_rgba(0, 0, 0, 0));
    draw_scene(scene, &mut view.textures);
    set_default_camera();

    clear_background(BLACK);
    if scene.dead {
        gl_use_material(&view.dead_tint);
    }
    let dest = VIEW.scale(scale);
    draw_texture_ex(
        &view.target.texture,
        offset.x.0,
        offset.y.0,
        WHITE,
        DrawTextureParams {
            dest_size: Some(vec2(dest.x.0, dest.y.0)),
            // The render-target camera writes rows bottom-up; presenting flips them back.
            flip_y: true,
            ..Default::default()
        },
    );
    if scene.dead {
        gl_use_default_material();
    }
}

fn draw_scene(scene: &Scene, textures: &mut Textures) {
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

    for layer in &area.layers {
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                if layer.dynamic && area.grouped_cells.contains(&(tx, ty)) {
                    continue;
                }
                draw_map_tile(textures, area, layer.at(tx, ty), scene.time, camera, tx, ty);
            }
        }
    }

    let mut draws: Vec<(f32, Draw)> = Vec::new();
    for &(at, cell) in &area.objects {
        let (tile_x, bottom_y) = (at.x.0, at.y.0);
        if tile_x < min_x as f32
            || tile_x > max_x as f32
            || bottom_y < min_y as f32
            || bottom_y > (max_y + 2) as f32
        {
            continue;
        }
        draws.push((
            bottom_y,
            Draw::Tile {
                cell,
                top_left: Pos::new(at.x, Tiles(bottom_y - 1.0)),
            },
        ));
    }
    for (index, actor) in scene.actors.iter().enumerate() {
        draws.push((actor.pos.y.0, Draw::Actor { index }));
    }
    for (index, group) in area.groups.iter().enumerate() {
        if group.z >= (min_y - 2) as f32 && group.z <= (max_y + 2) as f32 {
            draws.push((group.z, Draw::Group { group: index }));
        }
    }
    draws.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut bars: Vec<(Pos<Pixels>, f32)> = Vec::new();
    for (_, draw) in &draws {
        match *draw {
            Draw::Tile { cell, top_left } => {
                if let Some(sprite) = area.resolve(cell, scene.time) {
                    let dst = to_frame(camera, top_left);
                    textures.draw_png(
                        sprite.sheet,
                        sprite.region,
                        dst,
                        TILE_SIZE,
                        WHITE,
                        sprite.flip,
                    );
                }
            }
            Draw::Actor { index } => {
                let actor = &scene.actors[index];
                let center = to_frame_f(camera, actor.pos);
                let anchor = Pos::new(
                    Pixels((actor.region.size.x.0 / 2.0).round()),
                    Pixels((actor.region.size.y.0 * 2.0 / 3.0).round()),
                );
                let dst = center.map(f32::round) - anchor;
                textures.draw_png(
                    model(actor.model).sheet(),
                    actor.region,
                    dst,
                    actor.region.size,
                    rgba_color(actor.tint),
                    (false, false),
                );
                if let Some(fraction) = actor.health {
                    bars.push((center, fraction));
                }
            }
            Draw::Group { group } => {
                for &(tx, ty, cell) in &area.groups[group].tiles {
                    draw_map_tile(textures, area, cell, scene.time, camera, tx, ty);
                }
            }
        }
    }

    // Health bars are a HUD element: drawn after every world layer so an actor or object in front
    // (a higher z) can never hide them.
    for (center, fraction) in bars {
        health_bar(center, fraction);
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
    region: Rect<Pixels>,
    tint: Rgba,
    model: ActorModelId,
    health: Option<f32>,
}

enum Draw {
    Tile {
        cell: area::TileRef,
        top_left: Pos<Tiles>,
    },
    Actor {
        index: usize,
    },
    Group {
        group: usize,
    },
}

/// GPU textures by source atlas, uploaded on first use.
#[derive(Default)]
struct Textures {
    cache: HashMap<usize, Texture2D>,
}

impl Textures {
    fn draw_png(
        &mut self,
        png: &'static [u8],
        region: Rect<Pixels>,
        dst: Pos<Pixels>,
        dst_size: Size<Pixels>,
        tint: Color,
        flip: (bool, bool),
    ) {
        let texture = self
            .cache
            .entry(png.as_ptr() as usize)
            .or_insert_with(|| {
                let texture = Texture2D::from_file_with_format(png, Some(ImageFormat::Png));
                texture.set_filter(FilterMode::Nearest);
                texture
            })
            .clone();
        draw_region(&texture, region, dst, dst_size, tint, flip);
    }
}

fn draw_region(
    texture: &Texture2D,
    region: Rect<Pixels>,
    dst: Pos<Pixels>,
    dst_size: Size<Pixels>,
    tint: Color,
    flip: (bool, bool),
) {
    draw_texture_ex(
        texture,
        dst.x.0,
        dst.y.0,
        tint,
        DrawTextureParams {
            dest_size: Some(vec2(dst_size.x.0, dst_size.y.0)),
            source: Some(macroquad::math::Rect::new(
                region.pos.x.0,
                region.pos.y.0,
                region.size.x.0,
                region.size.y.0,
            )),
            flip_x: flip.0,
            flip_y: flip.1,
            ..Default::default()
        },
    );
}

fn draw_map_tile(
    textures: &mut Textures,
    area: &'static area::Area,
    cell: area::TileRef,
    time: f32,
    camera: Camera,
    tx: i32,
    ty: i32,
) {
    if let Some(sprite) = area.resolve(cell, time) {
        let dst = to_frame(camera, Pos::new(Tiles(tx as f32), Tiles(ty as f32)));
        textures.draw_png(
            sprite.sheet,
            sprite.region,
            dst,
            TILE_SIZE,
            WHITE,
            sprite.flip,
        );
    }
}

const BAR_SIZE: Size<Pixels> = Size::new(Pixels(20.0), Pixels(4.0));
const BAR_BORDER: u32 = 0x140A_28FF;
const BAR_BG: u32 = 0x2A1C_5CFF;
const BAR_FILL: u32 = 0x00FF_00FF;

fn health_bar(center: Pos<Pixels>, fraction: f32) {
    let top_left = center.map(f32::round) + Pos::new(Pixels(-BAR_SIZE.x.0 / 2.0), Pixels(3.0));
    fill(top_left, BAR_SIZE, BAR_BORDER);
    let inner = top_left + Pos::splat(Pixels(1.0));
    let inner_size = BAR_SIZE - Size::splat(Pixels(2.0));
    fill(inner, inner_size, BAR_BG);
    fill(
        inner,
        Size::new(Pixels((inner_size.x.0 * fraction).floor()), inner_size.y),
        BAR_FILL,
    );
}

fn fill(top_left: Pos<Pixels>, size: Size<Pixels>, rgba: u32) {
    let [r, g, b, a] = rgba.to_be_bytes();
    draw_rectangle(
        top_left.x.0,
        top_left.y.0,
        size.x.0,
        size.y.0,
        Color::from_rgba(r, g, b, a),
    );
}

fn rgba_color(tint: Rgba) -> Color {
    let [r, g, b, a] = tint.0.to_be_bytes();
    Color::from_rgba(r, g, b, a)
}

fn model(index: ActorModelId) -> &'static actors::ActorModel {
    &actors::models()[index.0 as usize]
}

fn to_frame(camera: Camera, world: Pos<Tiles>) -> Pos<Pixels> {
    to_frame_f(camera, world).map(f32::round)
}

fn snap(tiles: f32) -> f32 {
    (tiles * TILE_SIZE.x.0).round() / TILE_SIZE.x.0
}

// The software renderer shifted dead frames toward red exactly this way; the shader keeps the
// look: additive red, green and blue at a third.
fn dead_tint_material() -> Material {
    load_material(
        ShaderSource::Glsl {
            vertex: DEAD_VERTEX,
            fragment: DEAD_FRAGMENT,
        },
        MaterialParams::default(),
    )
    .expect("dead-tint shader compiles")
}

const DEAD_VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
varying lowp vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    uv = texcoord;
}"#;

const DEAD_FRAGMENT: &str = r#"#version 100
varying lowp vec2 uv;
uniform sampler2D Texture;
void main() {
    lowp vec4 c = texture2D(Texture, uv);
    gl_FragColor = vec4(min(c.r + 160.0 / 255.0, 1.0), c.g / 3.0, c.b / 3.0, c.a);
}"#;
