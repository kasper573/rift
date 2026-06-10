//! Renders the replicated world with macroquad into a fixed-size pixel view, then presents it
//! integer-scaled and letterboxed; the geometry helpers double as the input mapping.

use std::collections::HashMap;
use std::sync::LazyLock;

use macroquad::prelude::*;
use world::Entity;
use world::core::actors::ActorModelId;
use world::core::area::AreaId;
use world::core::math::{GridSize, Offset, Pixels, Pos, Rect, Size, Tiles};
use world::core::protocol::Rgba;
use world::core::protocol::{Actor, Position, Vitals, action_name};
use world::core::session::MmoClient;
use world::core::{actors, area};

type ActorQuery = (
    Entity,
    &'static Actor,
    &'static Position,
    Option<&'static Vitals>,
);

/// One tile, in pixels.
pub const TILE_SIZE: Size<Pixels> = Size::new(16.0, 16.0);
/// The rendered viewport, in tiles.
pub const VIEW_TILES: Size<Tiles> = Size::new(24.0, 18.0);
/// The rendered viewport, in pixels — one tile's worth of pixels across each tile of the view.
pub const VIEW: Size<Pixels> = Size::new(
    TILE_SIZE.width * VIEW_TILES.width,
    TILE_SIZE.height * VIEW_TILES.height,
);
static HALF_VIEW: LazyLock<Pos<Pixels>> = LazyLock::new(|| (VIEW * 0.5).to_vector().to_point());
static HALF_VIEW_TILES: LazyLock<Size<Tiles>> = LazyLock::new(|| VIEW_TILES * 0.5);

// Inventory layout, anchored top-right: a grid of square slots.
pub const INV_GRID: GridSize = GridSize::new(4, 6);
pub const INV_SLOT: f32 = 36.0;
pub const INV_PAD: f32 = 8.0;

/// A sound source's volume for a listener, both in tiles: 1 at the listener, falling linearly to
/// 0 at the edge of the rendered view and staying 0 beyond it, so off-screen sources are silent.
pub fn proximity_volume(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    let half = *HALF_VIEW_TILES;
    let offset = source - listener;
    let dx = offset.x.abs() / half.width;
    let dy = offset.y.abs() / half.height;
    (1.0 - dx.max(dy)).clamp(0.0, 1.0)
}

/// A sound source's stereo pan for a listener: -1 (full left) at the left edge of the view, 0 at
/// the listener's column, +1 (full right) at the right edge; clamped, vertical offset ignored.
pub fn proximity_pan(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    ((source - listener).x / HALF_VIEW_TILES.width).clamp(-1.0, 1.0)
}

/// Integer-scales the view into a screen and centers it: the scale, and the view's top-left offset.
pub fn letterbox(screen: Size<Pixels>) -> (f32, Pos<Pixels>) {
    let scale = (screen.width / VIEW.width)
        .min(screen.height / VIEW.height)
        .floor()
        .max(1.0);
    (
        scale,
        ((screen - VIEW * scale) * 0.5).to_vector().to_point(),
    )
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub center: Pos<Tiles>,
}

pub fn camera(client: &mut MmoClient) -> Option<Camera> {
    camera_for(client.my_position()?, client.my_area()?)
}

pub fn camera_for(at: Pos<Tiles>, area: AreaId) -> Option<Camera> {
    let area = area::areas().get(area.0 as usize)?;
    let half = *HALF_VIEW_TILES;
    let lo = Pos::new(half.width, half.height);
    let hi = Pos::new(
        (area.width.0 - half.width).max(half.width),
        (area.height.0 - half.height).max(half.height),
    );
    let center = at.clamp(lo, hi);
    Some(Camera {
        center: Pos::new(snap(center.x), snap(center.y)),
    })
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

pub fn build_scene(client: &mut MmoClient, time: f32, animator: &mut Animator) -> Scene {
    let camera = camera(client);
    let area = client.my_area().unwrap_or(AreaId(0));
    let me = client.my_entity();
    let dead = client.is_dead();
    let world = client.world_mut();
    animator
        .anchors
        .retain(|&entity, _| world.get_entity(entity).is_ok());
    let mut query = world.query::<ActorQuery>();
    let actors = query
        .iter(world)
        .map(|(entity, actor, position, vitals)| {
            let elapsed = animator.elapsed(entity, actor.action, time);
            ActorDraw {
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
                    .then_some(vitals)
                    .flatten()
                    .map(|v| (v.health / v.max).clamp(0.0, 1.0)),
            }
        })
        .collect();
    Scene {
        camera,
        area,
        time,
        actors,
        dead,
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
        let target = render_target(VIEW.width as u32, VIEW.height as u32);
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
    let mut camera = Camera2D::from_display_rect(macroquad::math::Rect::new(
        0.0,
        0.0,
        VIEW.width,
        VIEW.height,
    ));
    camera.render_target = Some(view.target.clone());
    set_camera(&camera);
    clear_background(Color::from_rgba(0, 0, 0, 0));
    draw_scene(scene, &mut view.textures);
    set_default_camera();

    clear_background(BLACK);
    if scene.dead {
        gl_use_material(&view.dead_tint);
    }
    let dest = VIEW * scale;
    draw_texture_ex(
        &view.target.texture,
        offset.x,
        offset.y,
        WHITE,
        DrawTextureParams {
            dest_size: Some(vec2(dest.width, dest.height)),
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
    let margin = Size::splat(1.0);
    let lo = (camera.center - half).floor() - margin;
    let hi = (camera.center + half).ceil() + margin;
    let (min_x, max_x) = (lo.x as i32, hi.x as i32);
    let (min_y, max_y) = (lo.y as i32, hi.y as i32);

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
        let (tile_x, bottom_y) = (at.x, at.y);
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
                top_left: Pos::new(at.x, bottom_y - 1.0),
            },
        ));
    }
    for (index, actor) in scene.actors.iter().enumerate() {
        draws.push((actor.pos.y, Draw::Actor { index }));
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
                let anchor = Offset::new(
                    (actor.region.size.width / 2.0).round(),
                    (actor.region.size.height * 2.0 / 3.0).round(),
                );
                let dst = center.round() - anchor;
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
    let offset = world - camera.center;
    *HALF_VIEW + Offset::new(offset.x * TILE_SIZE.width, offset.y * TILE_SIZE.height)
}

pub fn frame_to_world(camera: Camera, frame: Pos<Pixels>) -> Pos<Tiles> {
    let offset = frame - *HALF_VIEW;
    camera.center + Offset::new(offset.x / TILE_SIZE.width, offset.y / TILE_SIZE.height)
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
        dst.x,
        dst.y,
        tint,
        DrawTextureParams {
            dest_size: Some(vec2(dst_size.width, dst_size.height)),
            source: Some(macroquad::math::Rect::new(
                region.origin.x,
                region.origin.y,
                region.size.width,
                region.size.height,
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
        let dst = to_frame(camera, Pos::new(tx as f32, ty as f32));
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

const BAR_SIZE: Size<Pixels> = Size::new(20.0, 4.0);
const BAR_BORDER: u32 = 0x140A_28FF;
const BAR_BG: u32 = 0x2A1C_5CFF;
const BAR_FILL: u32 = 0x00FF_00FF;

fn health_bar(center: Pos<Pixels>, fraction: f32) {
    let top_left = center.round() + Offset::new(-BAR_SIZE.width / 2.0, 3.0);
    fill(top_left, BAR_SIZE, BAR_BORDER);
    let inner = top_left + Offset::splat(1.0);
    let inner_size = BAR_SIZE - Size::splat(2.0);
    fill(inner, inner_size, BAR_BG);
    fill(
        inner,
        Size::new((inner_size.width * fraction).floor(), inner_size.height),
        BAR_FILL,
    );
}

fn fill(top_left: Pos<Pixels>, size: Size<Pixels>, rgba: u32) {
    let [r, g, b, a] = rgba.to_be_bytes();
    draw_rectangle(
        top_left.x,
        top_left.y,
        size.width,
        size.height,
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
    to_frame_f(camera, world).round()
}

fn snap(tiles: f32) -> f32 {
    (tiles * TILE_SIZE.width).round() / TILE_SIZE.width
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
