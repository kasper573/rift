use client::render::{self, Screen, TILE_SIZE, VIEW};
use client::sfx::SfxTracker;
use client::ui::Hud;
use macroquad::prelude::*;
use std::collections::HashMap;
use world::actors::SfxId;
use world::math::{Offset, Pixels, Pos, Rect, Size, Tiles};
use world::{
    Actor, ClientId, Entity, Hitbox, ItemConsumed, LinkStatus, MmoClient, Position, Vitals, With,
};
use world::{area, assets};

mod audio;
mod platform;

#[macroquad::main(window_conf)]
async fn main() {
    let Some(mut client) = platform::open_session().await else {
        return;
    };
    // The one play/spectate switch: every mode-specific input and UI lives in its frame handler.
    let spectating = platform::spectate_mode();

    show_mouse(false);
    let cursors = Cursors::load();
    let highlight = texture_png(png_bytes("icons/crosshairs/white/crosshair026.png"));
    let mut announced = false;
    let mut clock = 0.0f32;
    let mut debug = DebugMode::None;
    let mut hud = Hud::new();
    let mut view = render::WorldView::new();
    let mut audio = audio::Audio::load(world::sfx::sfx_table());
    let sfx_index: HashMap<&'static SfxId, usize> = world::sfx::sfx_table()
        .iter()
        .enumerate()
        .map(|(index, def)| (&def.id, index))
        .collect();
    let mut sfx_tracker = SfxTracker::new();
    loop {
        if platform::exit_requested() {
            break;
        }
        client.poll();

        match client.status() {
            LinkStatus::Connecting => {
                overlay_text("Connecting...");
                cursors.draw(Cursor::Default);
                next_frame().await;
                continue;
            }
            LinkStatus::Closed => {
                overlay_text("Connection lost");
                cursors.draw(Cursor::Default);
                next_frame().await;
                continue;
            }
            LinkStatus::Open => {}
        }
        if !announced {
            if spectating {
                client.spectate(None);
            } else {
                client.join();
            }
            announced = true;
        }

        let listener = client
            .my_position()
            .or_else(|| render::camera(&mut client).map(|cam| cam.center));

        for ItemConsumed { item, actor } in client.drain::<ItemConsumed>() {
            let Some(listener) = listener else { continue };
            if let Some(id) = world::items::items()[item.0 as usize].sfx.as_ref()
                && let Some(source) = world::protocol::position(client.world(), actor)
            {
                let volume = render::proximity_volume(listener, source);
                let pan = render::proximity_pan(listener, source);
                if volume > 0.0 && sfx_tracker.ready(id, clock) {
                    play_cue(&mut audio, &sfx_index, id, volume, pan);
                }
            }
        }

        let screen = Screen::fit();
        draw_world(&mut client, clock, screen, &mut view);
        if let Some(listener) = listener {
            let area = client
                .my_area()
                .and_then(|id| area::areas().get(id.0 as usize));
            for (id, volume, pan) in sfx_tracker.cues(
                client.world_mut(),
                area,
                &mut view.animator,
                listener,
                clock,
            ) {
                play_cue(&mut audio, &sfx_index, id, volume, pan);
            }
        }
        if is_key_pressed(KeyCode::F1) {
            debug = debug.next();
        }
        debug_frame(&mut client, screen, debug);

        let cursor = if spectating {
            spectate_frame(&mut client)
        } else {
            egui_macroquad::ui(|ctx| hud.frame(ctx, &mut client, screen));
            let cursor = play_frame(&mut client, screen, &hud, &highlight);
            egui_macroquad::draw();
            cursor
        };

        hud_text(&format!("{} fps", get_fps()), screen_height() - 12.0);
        cursors.draw(cursor);

        clock += get_frame_time();
        next_frame().await;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DebugMode {
    None,
    All,
    Tile,
    ProximityNode,
    Obscured,
}

impl DebugMode {
    fn next(self) -> DebugMode {
        match self {
            DebugMode::None => DebugMode::All,
            DebugMode::All => DebugMode::Tile,
            DebugMode::Tile => DebugMode::ProximityNode,
            DebugMode::ProximityNode => DebugMode::Obscured,
            DebugMode::Obscured => DebugMode::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            DebugMode::None => "none",
            DebugMode::All => "all",
            DebugMode::Tile => "tile",
            DebugMode::ProximityNode => "proximity node",
            DebugMode::Obscured => "obscured",
        }
    }
}

#[derive(Clone, Copy)]
enum Cursor {
    Default,
    Attack,
    Move,
    MoveHeld,
}

struct Cursors {
    default: Texture2D,
    attack: Texture2D,
    movable: Texture2D,
    movable_held: Texture2D,
}

impl Cursors {
    fn load() -> Cursors {
        Cursors {
            default: texture_png(png_bytes("icons/cursors/pointer003.png")),
            attack: texture_png(png_bytes("icons/cursors/swords002.png")),
            movable: texture_png(png_bytes("icons/cursors/pointer010.png")),
            movable_held: texture_png(png_bytes("icons/cursors/pointer011.png")),
        }
    }

    fn draw(&self, cursor: Cursor) {
        // The default pointer's tip sits at the image's top-left; the others are centered motifs.
        let (texture, centered) = match cursor {
            Cursor::Default => (&self.default, false),
            Cursor::Attack => (&self.attack, true),
            Cursor::Move => (&self.movable, true),
            Cursor::MoveHeld => (&self.movable_held, true),
        };
        let hotspot = if centered {
            Offset::new(texture.width() / 2.0, texture.height() / 2.0)
        } else {
            Offset::zero()
        };
        let (mx, my) = mouse_position();
        let at: Pos<Pixels> = Pos::new(mx, my) - hotspot;
        draw_texture(texture, at.x, at.y, WHITE);
    }
}

fn play_frame(client: &mut MmoClient, screen: Screen, hud: &Hud, highlight: &Texture2D) -> Cursor {
    if client.is_dead() {
        banner_text("You died! Press any key to respawn");
        if get_last_key_pressed().is_some() {
            client.respawn();
        }
        return Cursor::Default;
    }
    let Some(camera) = render::camera(client) else {
        return Cursor::Default;
    };
    if hud.pointer_captured {
        return Cursor::Default;
    }

    let (mx, my) = mouse_position();
    let mouse = Pos::new(mx, my);
    let world = render::frame_to_world(camera, screen.to_frame(mouse));
    let hover = world.floor();
    let enemy = enemy_at(client, world);
    let in_view = mx >= screen.offset.x && my >= screen.offset.y;

    if is_mouse_button_pressed(MouseButton::Left) && in_view {
        match enemy {
            Some(target) => client.attack(target),
            None => client.move_to(hover.x + 0.5, hover.y + 0.5),
        }
    }

    if enemy.is_some() {
        return Cursor::Attack;
    }
    let movable = in_view
        && client
            .my_area()
            .and_then(|id| area::areas().get(id.0 as usize))
            .is_some_and(|area| area.grid.walkable(hover));
    if movable {
        let tile = TILE_SIZE.width * screen.scale;
        let p = screen.to_window(camera, hover);
        draw_texture_ex(
            highlight,
            p.x,
            p.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(tile, tile)),
                ..Default::default()
            },
        );
    }
    match (movable, is_mouse_button_down(MouseButton::Left)) {
        (true, false) => Cursor::Move,
        (true, true) => Cursor::MoveHeld,
        (false, _) => Cursor::Default,
    }
}

fn spectate_frame(client: &mut MmoClient) -> Cursor {
    if is_key_pressed(KeyCode::N) {
        let next = next_watch(client);
        client.spectate(next);
    }
    let status = match client.watching() {
        Some(id) => format!("spectating {} — N: next", watched_name(client, id)),
        None => "spectating — press N to watch a player".to_owned(),
    };
    hud_text(&status, 44.0);
    Cursor::Default
}

fn window_conf() -> Conf {
    Conf {
        window_title: "rift mmo".to_owned(),
        window_width: VIEW.width as i32 * 3,
        window_height: VIEW.height as i32 * 3,
        window_resizable: true,

        platform: macroquad::miniquad::conf::Platform {
            swap_interval: Some(0),
            // Render targets need glReadBuffer, which the web shim only has on a WebGL2 context.
            webgl_version: macroquad::miniquad::conf::WebGLVersion::WebGL2,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn draw_world(client: &mut MmoClient, clock: f32, screen: Screen, view: &mut render::WorldView) {
    let scene = render::build_scene(client, clock, &mut view.animator);
    render::present(&scene, view, screen.scale, screen.offset);
}

/// Walkability overlays for the current area: the `-` key cycles through the modes.
fn debug_frame(client: &mut MmoClient, screen: Screen, mode: DebugMode) {
    if mode == DebugMode::None {
        return;
    }
    hud_text(&format!("debug: {}", mode.label()), 68.0);
    let Some(camera) = render::camera(client) else {
        return;
    };
    let Some(area) = client
        .my_area()
        .and_then(|id| area::areas().get(id.0 as usize))
    else {
        return;
    };
    let (mx, my) = mouse_position();
    let mouse = Pos::new(mx, my);
    let pointer = render::frame_to_world(camera, screen.to_frame(mouse));
    match mode {
        DebugMode::None => {}
        DebugMode::All => {
            for &node in &area.walkable_nodes {
                draw_node_links(area, camera, screen, node);
            }
        }
        DebugMode::Tile => {
            if let Some(node) = proximity_node(area, pointer) {
                draw_node_links(area, camera, screen, node);
            }
        }
        DebugMode::ProximityNode => {
            if let Some(node) = proximity_node(area, pointer) {
                draw_debug_line(
                    screen.to_window(camera, pointer),
                    screen.to_window(camera, tile_center(node)),
                    screen.scale,
                );
            }
        }
        DebugMode::Obscured => {
            let tile = TILE_SIZE.width * screen.scale;
            for rect in &area.obscuring_rects {
                let p = screen.to_window(camera, rect.origin);
                let size: Size<Pixels> = rect.size.cast_unit() * tile;
                draw_rectangle(p.x, p.y, size.width, size.height, color_u8!(255, 0, 0, 128));
            }
            let amount = area.obscured_amount(pointer.x.floor() as i32, pointer.y.floor() as i32);
            let label = format!("{:.2}% obscured", amount * 100.0);
            let at = mouse + Offset::new(5.0 * screen.scale, 0.0);
            draw_text(&label, at.x + 1.0, at.y + 1.0, 7.0 * screen.scale, BLACK);
            draw_text(&label, at.x, at.y, 7.0 * screen.scale, WHITE);
        }
    }
}

fn draw_node_links(area: &area::Area, camera: render::Camera, screen: Screen, node: Pos<Tiles>) {
    let from = screen.to_window(camera, tile_center(node));
    for (dx, dy) in [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ] {
        let neighbor = Pos::new(node.x + dx as f32, node.y + dy as f32);
        if area.grid.walkable(neighbor) {
            draw_debug_line(
                from,
                screen.to_window(camera, tile_center(neighbor)),
                screen.scale,
            );
        }
    }
}

fn draw_debug_line(from: Pos<Pixels>, to: Pos<Pixels>, scale: f32) {
    draw_line(
        from.x,
        from.y,
        to.x,
        to.y,
        2.0 * scale,
        color_u8!(255, 0, 0, 255),
    );
}

/// A tile cell's center position.
fn tile_center(cell: Pos<Tiles>) -> Pos<Tiles> {
    cell + Offset::splat(0.5)
}

/// The walkable tile at the pointer, or the nearest adjacent one.
fn proximity_node(area: &area::Area, p: Pos<Tiles>) -> Option<Pos<Tiles>> {
    let (tx, ty) = (p.x.floor() as i32, p.y.floor() as i32);
    [
        (0, 0),
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ]
    .into_iter()
    .map(|(dx, dy)| Pos::new((tx + dx) as f32, (ty + dy) as f32))
    .find(|&node| area.grid.walkable(node))
}

fn enemy_at(client: &mut MmoClient, point: Pos<Tiles>) -> Option<Entity> {
    let me = client.my_entity();
    let world = client.world_mut();
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(|v| v.health <= 0.0) {
            return None;
        }
        let bottom = at.pos.y + 0.5;
        let bounds = Rect::new(
            Pos::new(
                at.pos.x - hitbox.size.width / 2.0,
                bottom - hitbox.size.height,
            ),
            hitbox.size,
        );
        bounds.contains(point).then_some(entity)
    })
}

fn next_watch(client: &mut MmoClient) -> Option<ClientId> {
    let players = client.players();
    let current = client.watching().and_then(|current| {
        players
            .iter()
            .position(|(candidate, _)| *candidate == current)
    });
    match current {
        Some(index) => players.get((index + 1) % players.len()),
        None => players.first(),
    }
    .map(|(id, _)| *id)
}

fn watched_name(client: &mut MmoClient, id: ClientId) -> String {
    client
        .players()
        .into_iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, name)| name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("player {}", id.0))
}

fn png_bytes(path: &str) -> &'static [u8] {
    assets::bytes(path).unwrap_or_else(|| panic!("missing embedded asset {path}"))
}

fn texture_png(png: &[u8]) -> Texture2D {
    let texture = Texture2D::from_file_with_format(png, Some(ImageFormat::Png));
    texture.set_filter(FilterMode::Nearest);
    texture
}

fn hud_text(text: &str, y: f32) {
    draw_text(text, 9.0, y + 1.0, 24.0, BLACK);
    draw_text(text, 8.0, y, 24.0, WHITE);
}

fn banner_text(text: &str) {
    let size = 32.0;
    let dimensions = measure_text(text, None, size as u16, 1.0);
    let x = (screen_width() - dimensions.width) / 2.0;
    let y = screen_height() / 2.0;
    draw_text(text, x + 1.0, y + 1.0, size, BLACK);
    draw_text(text, x, y, size, WHITE);
}

// Resolves the table row's random base volume + pitch, folds in the distance attenuation, and
// plays the shot positioned by `pan`.
fn play_cue(
    audio: &mut audio::Audio,
    index: &HashMap<&'static SfxId, usize>,
    id: &SfxId,
    proximity: f32,
    pan: f32,
) {
    let Some(&row) = index.get(id) else {
        return;
    };
    let def = &world::sfx::sfx_table()[row];
    let base = def.volume.resolve(macroquad::rand::gen_range(0.0f32, 1.0));
    let pitch = def.pitch.resolve(macroquad::rand::gen_range(0.0f32, 1.0));
    audio.play(row, base * proximity, pitch, pan);
}

fn overlay_text(text: &str) {
    clear_background(BLACK);
    banner_text(text);
}
