use client::render::{self, INV_GRID, INV_PAD, INV_SLOT, TILE_SIZE, VIEW};
use client::sfx::SfxTracker;
use macroquad::prelude::*;
use std::collections::HashMap;
use world::core::actors::SfxId;
use world::core::math::{Pixels, Pos, Rect, Size, Tiles};
use world::core::{area, assets};
use world::{
    Actor, ClientId, Entity, Hitbox, ItemConsumed, LinkStatus, MmoClient, Position, Vitals, With,
};

mod audio;
mod platform;

const DOUBLE_CLICK_SECONDS: f64 = 0.4;

#[macroquad::main(window_conf)]
async fn main() {
    let Some(mut client) = platform::open_session().await else {
        return;
    };
    // The one play/spectate switch: every mode-specific input and UI lives in its frame handler.
    let spectating = platform::spectate_mode();
    let frame: fn(&mut MmoClient, Screen, &mut Ui) -> Cursor = if spectating {
        spectate_frame
    } else {
        play_frame
    };

    show_mouse(false);
    let cursors = Cursors::load();
    let mut announced = false;
    let mut clock = 0.0f32;
    let mut ui = Ui::new();
    let mut view = render::WorldView::new();
    let mut audio = audio::Audio::load(world::features::sfx::sfx_table());
    let sfx_index: HashMap<&'static SfxId, usize> = world::features::sfx::sfx_table()
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
            if let Some(id) = world::features::items::items()[item.0 as usize]
                .sfx
                .as_ref()
                && let Some(source) = world::core::protocol::position(client.world(), actor)
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
            ui.debug = ui.debug.next();
        }
        debug_frame(&mut client, screen, ui.debug);
        let cursor = frame(&mut client, screen, &mut ui);

        hud_text(&format!("{} fps", get_fps()), 20.0);
        cursors.draw(cursor);

        clock += get_frame_time();
        next_frame().await;
    }
}

struct Ui {
    icons: Vec<Texture2D>,
    highlight: Texture2D,
    inventory_scroll: u32,
    last_inventory_click: Option<(u32, f64)>,
    debug: DebugMode,
}

impl Ui {
    fn new() -> Ui {
        Ui {
            icons: world::features::items::items()
                .iter()
                .map(|item| texture_png(item.icon.0))
                .collect(),
            highlight: texture_png(png_bytes("icons/crosshairs/white/crosshair026.png")),
            inventory_scroll: 0,
            last_inventory_click: None,
            debug: DebugMode::None,
        }
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
            Size::new(
                Pixels(texture.width() / 2.0),
                Pixels(texture.height() / 2.0),
            )
        } else {
            Size::splat(Pixels(0.0))
        };
        let (mx, my) = mouse_position();
        let at = Pos::new(Pixels(mx), Pixels(my)) - hotspot;
        draw_texture(texture, at.x.0, at.y.0, WHITE);
    }
}

#[derive(Clone, Copy)]
struct Screen {
    scale: f32,
    offset: Pos<Pixels>,
}

impl Screen {
    fn fit() -> Screen {
        let (scale, offset) =
            render::letterbox(Size::new(Pixels(screen_width()), Pixels(screen_height())));
        Screen { scale, offset }
    }

    /// A world position to its on-screen window position.
    fn to_window(self, camera: render::Camera, world: Pos<Tiles>) -> Pos<Pixels> {
        render::to_frame_f(camera, world).scale(self.scale) + self.offset
    }

    /// A window pixel position back to a world-frame pixel position.
    fn to_frame(self, window: Pos<Pixels>) -> Pos<Pixels> {
        (window - self.offset).scale(1.0 / self.scale)
    }
}

fn play_frame(client: &mut MmoClient, screen: Screen, ui: &mut Ui) -> Cursor {
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

    hud_text(&format!("xp {}", client.my_xp().unwrap_or(0)), 44.0);
    if inventory_frame(client, ui) {
        return Cursor::Default;
    }

    let (mx, my) = mouse_position();
    let mouse = Pos::new(Pixels(mx), Pixels(my));
    let world = render::frame_to_world(camera, screen.to_frame(mouse));
    let hover = world.map(f32::floor);
    let enemy = enemy_at(client, world);
    let in_view = mx >= screen.offset.x.0 && my >= screen.offset.y.0;

    if is_mouse_button_pressed(MouseButton::Left) && in_view {
        match enemy {
            Some(target) => client.attack(target),
            None => client.move_to(hover.x.0 + 0.5, hover.y.0 + 0.5),
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
        let tile = TILE_SIZE.x.0 * screen.scale;
        let p = screen.to_window(camera, hover);
        draw_texture_ex(
            &ui.highlight,
            p.x.0,
            p.y.0,
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

// Draws the inventory grid (fixed viewport over the unbounded inventory, wheel-scrolled) and
// applies its clicks; returns whether the pointer is over the grid, which swallows world input.
fn inventory_frame(client: &mut MmoClient, ui: &mut Ui) -> bool {
    let items = client.my_inventory();
    let grid = INV_GRID.convert::<Pixels>(|slots| slots * INV_SLOT);
    let origin = Pos::new(Pixels(screen_width() - INV_PAD - grid.x.0), Pixels(INV_PAD));
    let grid_rect = Rect::new(origin, grid);

    let total_rows = (items.len() as u32).div_ceil(INV_GRID.x);
    let max_scroll = total_rows.saturating_sub(INV_GRID.y);
    ui.inventory_scroll = ui.inventory_scroll.min(max_scroll);

    let (mx, my) = mouse_position();
    let mouse = Pos::new(Pixels(mx), Pixels(my));
    let hovering = grid_rect.contains(mouse);
    if hovering {
        let wheel = mouse_wheel().1;
        if wheel > 0.0 {
            ui.inventory_scroll = ui.inventory_scroll.saturating_sub(1);
        } else if wheel < 0.0 {
            ui.inventory_scroll = (ui.inventory_scroll + 1).min(max_scroll);
        }
    }

    let slot_size = Size::splat(Pixels(INV_SLOT));
    let inner = slot_size - Size::splat(Pixels(2.0));
    let mut hovered: Option<u32> = None;
    for row in 0..INV_GRID.y {
        for col in 0..INV_GRID.x {
            let at = origin + Size::new(col, row).convert::<Pixels>(|slots| slots * INV_SLOT);
            let slot = (ui.inventory_scroll + row) * INV_GRID.x + col;
            let occupied = (slot as usize) < items.len();
            let over = Rect::new(at, slot_size).contains(mouse);
            if over && occupied {
                hovered = Some(slot);
            }
            draw_rectangle(
                at.x.0,
                at.y.0,
                inner.x.0,
                inner.y.0,
                color_u8!(0, 0, 0, 160),
            );
            let outline = if over { WHITE } else { GRAY };
            draw_rectangle_lines(at.x.0, at.y.0, inner.x.0, inner.y.0, 2.0, outline);
            if occupied {
                let icon = &ui.icons[items[slot as usize].0 as usize];
                let icon_at = at + Pos::splat(Pixels(1.0));
                draw_texture(icon, icon_at.x.0, icon_at.y.0, WHITE);
            }
        }
    }

    if total_rows > INV_GRID.y {
        let track = origin + Size::new(Pixels(grid.x.0 + 2.0), Pixels(0.0));
        draw_rectangle(track.x.0, track.y.0, 4.0, grid.y.0, color_u8!(0, 0, 0, 160));
        let thumb_h = grid.y.0 * INV_GRID.y as f32 / total_rows as f32;
        let thumb_y =
            track.y.0 + (grid.y.0 - thumb_h) * ui.inventory_scroll as f32 / max_scroll as f32;
        draw_rectangle(track.x.0, thumb_y, 4.0, thumb_h, GRAY);
    }

    if let Some(slot) = hovered {
        let name = &world::features::items::item(items[slot as usize]).display_name;
        let width = measure_text(name, None, 20, 1.0).width;
        let label = origin + Size::new(Pixels(grid.x.0 - width), Pixels(grid.y.0 + 16.0));
        draw_text(name, label.x.0, label.y.0, 20.0, WHITE);
    }

    if hovering
        && is_mouse_button_pressed(MouseButton::Left)
        && let Some(slot) = hovered
    {
        let now = get_time();
        let double = ui
            .last_inventory_click
            .is_some_and(|(last, at)| last == slot && now - at < DOUBLE_CLICK_SECONDS);
        if double {
            client.use_item(slot);
            ui.last_inventory_click = None;
        } else {
            ui.last_inventory_click = Some((slot, now));
        }
    }
    hovering
}

fn spectate_frame(client: &mut MmoClient, _screen: Screen, _ui: &mut Ui) -> Cursor {
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
        window_width: VIEW.x.0 as i32 * 3,
        window_height: VIEW.y.0 as i32 * 3,
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
    let mouse = Pos::new(Pixels(mx), Pixels(my));
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
            let tile = TILE_SIZE.x.0 * screen.scale;
            for rect in &area.obscuring_rects {
                let p = screen.to_window(camera, rect.pos);
                let size = rect.size.convert::<Pixels>(|t| t * tile);
                draw_rectangle(p.x.0, p.y.0, size.x.0, size.y.0, color_u8!(255, 0, 0, 128));
            }
            let amount =
                area.obscured_amount(pointer.x.0.floor() as i32, pointer.y.0.floor() as i32);
            let label = format!("{:.2}% obscured", amount * 100.0);
            let at = mouse + Size::new(Pixels(5.0 * screen.scale), Pixels(0.0));
            draw_text(
                &label,
                at.x.0 + 1.0,
                at.y.0 + 1.0,
                7.0 * screen.scale,
                BLACK,
            );
            draw_text(&label, at.x.0, at.y.0, 7.0 * screen.scale, WHITE);
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
        let neighbor = Pos::new(Tiles(node.x.0 + dx as f32), Tiles(node.y.0 + dy as f32));
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
        from.x.0,
        from.y.0,
        to.x.0,
        to.y.0,
        2.0 * scale,
        color_u8!(255, 0, 0, 255),
    );
}

/// A tile cell's center position.
fn tile_center(cell: Pos<Tiles>) -> Pos<Tiles> {
    cell.map(|t| t + 0.5)
}

/// The walkable tile at the pointer, or the nearest adjacent one.
fn proximity_node(area: &area::Area, p: Pos<Tiles>) -> Option<Pos<Tiles>> {
    let (tx, ty) = (p.x.0.floor() as i32, p.y.0.floor() as i32);
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
    .map(|(dx, dy)| Pos::new(Tiles((tx + dx) as f32), Tiles((ty + dy) as f32)))
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
        let bottom = at.pos.y.0 + 0.5;
        let bounds = Rect::new(
            Pos::new(
                Tiles(at.pos.x.0 - hitbox.size.x.0 / 2.0),
                Tiles(bottom - hitbox.size.y.0),
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
    let def = &world::features::sfx::sfx_table()[row];
    let base = def.volume.resolve(macroquad::rand::gen_range(0.0f32, 1.0));
    let pitch = def.pitch.resolve(macroquad::rand::gen_range(0.0f32, 1.0));
    audio.play(row, base * proximity, pitch, pan);
}

fn overlay_text(text: &str) {
    clear_background(BLACK);
    banner_text(text);
}
