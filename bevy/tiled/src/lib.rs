//! A small, general-purpose renderer that draws a [`tiled::Map`] as Bevy sprites — finite tile layers
//! (stacked by file order), tile-objects, flips, and per-tile animation. It is deliberately
//! game-agnostic: it knows nothing about any particular world model. Callers plug in the two things
//! that *are* theirs — how to load a tileset's image, and what depth (`z`) each tile and object gets —
//! through [`MapHooks`], so a game can layer its own depth model (e.g. grouped occluders) and asset
//! source (e.g. an embed) on top without this crate ever depending on the game. [`Files`] is a ready
//! hook for tools that read maps straight off disk.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::sprite::Anchor;

/// Logical pixels per tile, fixed at the art's native size so nearest-neighbour magnification keeps
/// each art-pixel a whole number of screen pixels.
pub const TILE: f32 = 16.0;

/// Tags every sprite a map spawns, so a caller can despawn the lot to swap maps.
#[derive(Component)]
pub struct MapTile;

/// The caller-supplied integration points: this crate draws the map and asks the hook for the bits
/// that are the caller's business. The `tile_z`/`object_z` defaults stack layers by index and y-sort
/// objects just above them — enough for previews; a game overrides them with its own depth model.
pub trait MapHooks {
    /// Resolve a tileset's image to a loaded handle. The caller owns the asset source (filesystem,
    /// an embed, a network reader…); `images` is offered for callers that decode into it directly.
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>>;

    /// Depth of a tile-layer cell. `layer` is the tile layer's index in file order.
    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> f32 {
        let _ = (x, y);
        layer as f32
    }

    /// Depth of a tile-object at its foot. `above` is the number of tile layers drawn before it; `y`
    /// is the foot in pixels (downward); `map_height` is the map height in tiles.
    fn object_z(&mut self, above: usize, x: f32, y: f32, map_height: f32) -> f32 {
        let _ = x;
        above as f32 + (y / TILE + 1.0) / (map_height + 2.0)
    }
}

/// Draws every tile-layer cell and tile-object of `map` as a [`MapTile`] sprite, taking image handles
/// and depths from `hooks`. Animated tiles get an [`Animated`] component that [`TileAnimationPlugin`]
/// then drives.
pub fn spawn_map(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    map: &tiled::Map,
    hooks: &mut impl MapHooks,
) {
    let map_height = map.height as f32;
    let mut layer = 0;
    for tiled_layer in map.layers() {
        match tiled_layer.layer_type() {
            tiled::LayerType::Tiles(tiles) => {
                for y in 0..map.height as i32 {
                    for x in 0..map.width as i32 {
                        let Some(tile) = tiles.get_tile(x, y) else {
                            continue;
                        };
                        let tileset = tile.get_tileset();
                        let Some(sheet) = hooks.image(tileset, images) else {
                            continue;
                        };
                        let z = hooks.tile_z(layer, x, y);
                        let transform = Transform::from_xyz(
                            (x as f32 + 0.5) * TILE,
                            -(y as f32 + 0.5) * TILE,
                            z,
                        );
                        spawn(
                            commands,
                            sheet,
                            frames(tileset, tile.id()),
                            Vec2::splat(TILE),
                            transform,
                            tile.flip_h,
                            tile.flip_v,
                            None,
                        );
                    }
                }
                layer += 1;
            }
            tiled::LayerType::Objects(objects) => {
                for object in objects.objects() {
                    let Some(object_tile) = object.get_tile() else {
                        continue;
                    };
                    let Some(data) = object.tile_data() else {
                        continue;
                    };
                    let tileset = object_tile.get_tileset();
                    let Some(sheet) = hooks.image(tileset, images) else {
                        continue;
                    };
                    let size = Vec2::new(tileset.tile_width as f32, tileset.tile_height as f32);
                    let z = hooks.object_z(layer, object.x, object.y, map_height);
                    let transform = Transform::from_xyz(object.x, -object.y, z);
                    spawn(
                        commands,
                        sheet,
                        frames(tileset, data.id()),
                        size,
                        transform,
                        data.flip_h,
                        data.flip_v,
                        Some(Anchor::BOTTOM_LEFT),
                    );
                }
            }
            _ => {}
        }
    }
}

#[derive(Component)]
struct Animated {
    frames: Vec<(Rect, f32)>,
    total: f32,
}

/// Advances animated tiles through their frames. Add it wherever a map is shown.
pub struct TileAnimationPlugin;

impl Plugin for TileAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate);
    }
}

fn animate(time: Res<Time>, mut tiles: Query<(&Animated, &mut Sprite)>) {
    let now = time.elapsed_secs();
    for (anim, mut sprite) in &mut tiles {
        if anim.total <= 0.0 {
            continue;
        }
        let mut remaining = now % anim.total;
        for &(rect, duration) in &anim.frames {
            if remaining < duration {
                sprite.rect = Some(rect);
                break;
            }
            remaining -= duration;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    image: Handle<Image>,
    frames: Vec<(Rect, f32)>,
    size: Vec2,
    transform: Transform,
    flip_x: bool,
    flip_y: bool,
    anchor: Option<Anchor>,
) {
    let total: f32 = frames.iter().map(|&(_, duration)| duration).sum();
    let sprite = Sprite {
        image,
        rect: Some(frames[0].0),
        custom_size: Some(size),
        flip_x,
        flip_y,
        ..default()
    };
    let mut tile = commands.spawn((MapTile, sprite, transform));
    if let Some(anchor) = anchor {
        tile.insert(anchor);
    }
    if total > 0.0 {
        tile.insert(Animated { frames, total });
    }
}

fn region(tileset: &tiled::Tileset, id: u32) -> Rect {
    let columns = tileset.columns.max(1);
    let x = (tileset.margin + (id % columns) * (tileset.tile_width + tileset.spacing)) as f32;
    let y = (tileset.margin + (id / columns) * (tileset.tile_height + tileset.spacing)) as f32;
    Rect::new(
        x,
        y,
        x + tileset.tile_width as f32,
        y + tileset.tile_height as f32,
    )
}

fn frames(tileset: &tiled::Tileset, id: u32) -> Vec<(Rect, f32)> {
    match tileset.get_tile(id).and_then(|tile| tile.animation.clone()) {
        Some(frames) if !frames.is_empty() => frames
            .iter()
            .map(|frame| {
                (
                    region(tileset, frame.tile_id),
                    frame.duration as f32 / 1000.0,
                )
            })
            .collect(),
        _ => vec![(region(tileset, id), 0.0)],
    }
}

/// A [`MapHooks`] for off-disk tools: decodes each tileset image from its file with the `image` crate
/// (so absolute or relative paths both work) and uses the default layer/y-sort depth. Caches by
/// tileset so each sheet decodes once.
#[derive(Default)]
pub struct Files {
    sheets: HashMap<usize, Handle<Image>>,
}

impl MapHooks for Files {
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = tileset as *const tiled::Tileset as usize;
        if let Some(handle) = self.sheets.get(&key) {
            return Some(handle.clone());
        }
        let source = &tileset.image.as_ref()?.source;
        let decoded = image::open(source)
            .map_err(|error| eprintln!("tileset image {}: {error}", source.display()))
            .ok()?;
        let mut image = Image::from_dynamic(decoded, true, RenderAssetUsages::default());
        image.sampler = ImageSampler::nearest();
        let handle = images.add(image);
        self.sheets.insert(key, handle.clone());
        Some(handle)
    }
}
