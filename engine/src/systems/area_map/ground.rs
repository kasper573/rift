use std::collections::HashMap;

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use ui::egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use ui::ink;

use crate::core::content::Content;
use crate::core::ground::{SAMPLES_PER_METER, TILE_PIXELS, TILE_SIDE, TileAt, TileVersion};
use crate::core::gui;
use crate::core::math::{Pos, Rect};
use crate::core::units::Meters;
use crate::systems::area::{self, Area, AreaId};

#[derive(Resource, Default)]
pub(super) struct MapGround {
    area: Option<AreaId>,
    tiles: HashMap<TileAt, TilePicture>,
    making: Option<Making>,
}

pub(super) struct TilePicture {
    pub texture: TextureHandle,
    pub rect: Rect<Meters>,
    of: Drawn,
}

impl MapGround {
    // A map never shows the ground of the place just left.
    pub fn pictures(&self, area: AreaId) -> impl Iterator<Item = &TilePicture> {
        self.tiles.values().filter(move |_| self.area == Some(area))
    }
}

pub(super) fn keep(world: &mut World, area_id: AreaId, shown: &[Rect<Meters>]) {
    let content = world.resource::<Content>().clone();
    let Some(area) = area::loaded(&content, area_id).cloned() else {
        return;
    };
    let Some(gui) = gui::context(world) else {
        return;
    };
    let mut ground = world.resource_mut::<MapGround>();
    if ground.area != Some(area_id) {
        *ground = MapGround {
            area: Some(area_id),
            ..default()
        };
    }
    if let Some(making) = ground.making.as_mut()
        && let Some(picture) = check_ready(&mut making.task)
    {
        let tile = making.tile;
        ground.making = None;
        let texture = gui.load_texture(
            format!("minimap {}", tile.key()),
            ColorImage::new(picture.size, picture.pixels),
            TextureOptions::LINEAR,
        );
        ground.tiles.insert(
            tile,
            TilePicture {
                texture,
                rect: tile.rect(area.ground_size()),
                of: picture.of,
            },
        );
    }
    // A picture is a quarter of a megabyte, and an area may hold thousands of tiles.
    ground.tiles.retain(|_, picture| {
        shown.iter().any(|shown| {
            picture
                .rect
                .intersects(&shown.inflate(TILE_SIDE.0, TILE_SIDE.0))
        })
    });
    if ground.making.is_some() {
        return;
    }
    let tiles = shown
        .iter()
        .flat_map(|shown| area.ground_size().tiles_within(*shown));
    let stale = tiles.into_iter().find(|tile| {
        ground
            .tiles
            .get(tile)
            .is_none_or(|picture| picture.of != Drawn::of(&area, *tile))
    });
    if let Some(tile) = stale {
        let colors = terrain_colors(&content, &area);
        let task = AsyncComputeTaskPool::get().spawn(async move { picture(&area, &colors, tile) });
        ground.making = Some(Making { tile, task });
    }
}

struct Making {
    tile: TileAt,
    task: Task<Picture>,
}

struct Picture {
    size: [usize; 2],
    pixels: Vec<Color32>,
    of: Drawn,
}

/// Which values a picture was made from: a tile not read yet answers from the area's overview,
/// and its picture is made anew once the tile is read.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    paint: TileVersion,
    heights: TileVersion,
    water: TileVersion,
}

impl Drawn {
    fn of(area: &Area, tile: TileAt) -> Drawn {
        Drawn {
            paint: area.paint.version(tile),
            heights: area.heights.version(tile),
            water: area.water.version(tile),
        }
    }
}

const NO_GROUND: Color32 = ink::FRAME;
const WATER: Color32 = Color32::from_rgb(0x86, 0xAE, 0xC6);
const BUILT: Color32 = Color32::from_rgb(0x8C, 0x7A, 0x68);
const BUILT_EDGE: Color32 = Color32::from_rgb(0x4E, 0x40, 0x34);
const HIDES_THE_GROUND: Meters = Meters(1.0);
const SHALLOW_SHARE: f32 = 0.55;
const SHADE: (f32, f32) = (0.7, 1.2);

fn terrain_colors(content: &Content, area: &Area) -> Vec<Color32> {
    area.terrains
        .iter()
        .map(|terrain| {
            let [r, g, b, _] = terrain.get(content).color.0.to_be_bytes();
            Color32::from_rgb(r, g, b)
        })
        .collect()
}

fn picture(area: &Area, colors: &[Color32], tile: TileAt) -> Picture {
    let of = Drawn::of(area, tile);
    let rect = tile.rect(area.ground_size());
    let per_meter = SAMPLES_PER_METER as f32;
    let columns = (rect.size.width * per_meter).round() as u32;
    let rows = (rect.size.height * per_meter).round() as u32;
    let first = (tile.column * TILE_PIXELS, tile.row * TILE_PIXELS);
    let mut paint = area.paint.reader();
    let mut heights = area.heights.reader();
    let mut water = area.water.reader();
    let mut pixels = Vec::with_capacity((columns * rows) as usize);
    for row in first.1..first.1 + rows {
        for column in first.0..first.0 + columns {
            let terrain = paint.index(column, row);
            let Some(&base) = colors.get(terrain as usize) else {
                pixels.push(NO_GROUND);
                continue;
            };
            // The four height samples round a paint pixel stand at its corners.
            let corners = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .map(|(right, down)| heights.sample(column + right, row + down).0);
            let ground = corners.iter().sum::<f32>() / 4.0;
            let east = (corners[1] + corners[3] - corners[0] - corners[2]) * per_meter / 2.0;
            let south = (corners[2] + corners[3] - corners[0] - corners[1]) * per_meter / 2.0;
            let lit = lit_from_north_west(base, east, south);
            let depth = water.sample(column, row).0 - ground;
            pixels.push(if depth > 0.0 {
                let share =
                    SHALLOW_SHARE + (1.0 - SHALLOW_SHARE) * (depth / HIDES_THE_GROUND.0).min(1.0);
                ink::on(lit, WATER, share)
            } else {
                lit
            });
        }
    }
    let mut picture = Picture {
        size: [columns as usize, rows as usize],
        pixels,
        of,
    };
    for footprint in area.footprints_within(rect) {
        stamp(&mut picture, rect, footprint);
    }
    picture
}

fn lit_from_north_west(color: Color32, east: f32, south: f32) -> Color32 {
    let light = Vec3::new(-1.0, 2.0, -1.0).normalize();
    let normal = Vec3::new(-east, 1.0, -south).normalize();
    let shade = (normal.dot(light) / light.y).clamp(SHADE.0, SHADE.1);
    let lit = |channel: u8| (f32::from(channel) * shade).round().min(255.0) as u8;
    Color32::from_rgb(lit(color.r()), lit(color.g()), lit(color.b()))
}

// The edge is darker so a building reads against ground of its own colour.
fn stamp(picture: &mut Picture, rect: Rect<Meters>, footprint: &[Pos<Meters>]) {
    let per_meter = SAMPLES_PER_METER as f32;
    let [columns, rows] = picture.size;
    let pixel = |at: Pos<Meters>| {
        (
            (at.x - rect.origin.x) * per_meter,
            (at.y - rect.origin.y) * per_meter,
        )
    };
    let corners: Vec<(f32, f32)> = footprint.iter().copied().map(pixel).collect();
    let span = |pick: fn(&(f32, f32)) -> f32, most: usize| {
        let low = corners.iter().map(pick).fold(f32::INFINITY, f32::min);
        let high = corners.iter().map(pick).fold(f32::NEG_INFINITY, f32::max);
        (low.floor().max(0.0) as usize)..(high.ceil().max(0.0) as usize + 1).min(most)
    };
    let (across, down) = (
        span(|corner| corner.0, columns),
        span(|corner| corner.1, rows),
    );
    let within = |column: i64, row: i64| inside(&corners, column as f32 + 0.5, row as f32 + 0.5);
    for row in down {
        for column in across.clone() {
            let (x, y) = (column as i64, row as i64);
            if !within(x, y) {
                continue;
            }
            let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|(right, down)| !within(x + right, y + down));
            picture.pixels[row * columns + column] = if edge { BUILT_EDGE } else { BUILT };
        }
    }
}

fn inside(corners: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut last = match corners.last() {
        Some(last) => *last,
        None => return false,
    };
    for &corner in corners {
        let crosses = (corner.1 > y) != (last.1 > y)
            && x < (last.0 - corner.0) * (y - corner.1) / (last.1 - corner.1) + corner.0;
        if crosses {
            inside = !inside;
        }
        last = corner;
    }
    inside
}
