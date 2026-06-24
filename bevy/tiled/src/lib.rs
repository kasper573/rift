//! Renders our Tiled-built [`Area`]s as Bevy sprites: the tile↔screen projection, the depth model
//! (static layers stacked by index, the dynamic layer's groups and objects depth-sorted by their
//! near edge), and the per-area sprite spawn. The game client and the `render` map-preview tool both
//! draw maps through here, so a map looks the same in the running game and in a preview.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::math::{Pos, Size, WorldPx};
use world::core::table::Id;
use world::core::tiling::{Cell, GridDims, TileSize, Tiles};
use world::core::time::Seconds;
use world::systems::area::{self, Area, AreaDef, TileRef, TileSprite};

/// Logical pixels per tile. A multiple keeps each art-pixel an exact whole number of screen pixels
/// under nearest-neighbour magnification.
pub const TILE: WorldPx = WorldPx(16.0);

/// Marks a sprite belonging to the current area's static map, so a caller can despawn the lot when
/// the area changes.
#[derive(Component)]
pub struct AreaTile;

/// Advances flagged tiles through their animation frames; add it wherever maps are shown.
pub struct TileAnimationPlugin;

impl Plugin for TileAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate_tiles);
    }
}

/// Spawns every static tile, depth group, and tile object of `area` as an [`AreaTile`] sprite. Frames
/// are resolved at time zero; [`TileAnimationPlugin`] keeps animated tiles moving thereafter.
pub fn spawn_area(commands: &mut Commands, assets: &AssetServer, area: &Area) {
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
            let sprite = tile_sprite(assets, &sprite, Vec2::splat(TILE.0));
            place(commands, area, sprite, sprite_transform(c.center(), z), cell, None);
        }
        if !layer.dynamic {
            continue;
        }
        for group in &area.groups {
            let z = dynamic_z(area.size.height, z, group.bottom);
            for &(c, cell) in &group.tiles {
                let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                    continue;
                };
                let sprite = tile_sprite(assets, &sprite, Vec2::splat(TILE.0));
                place(commands, area, sprite, sprite_transform(c.center(), z), cell, None);
            }
        }
        for &(pos, cell) in &area.objects {
            let Some(sprite) = area.resolve(cell, Seconds(0.0)) else {
                continue;
            };
            let size = Vec2::new(sprite.region.size.width, sprite.region.size.height);
            let sprite = tile_sprite(assets, &sprite, size);
            let transform = sprite_transform(pos, dynamic_z(area.size.height, z, Tiles(pos.y)));
            place(
                commands,
                area,
                sprite,
                transform,
                cell,
                Some(Anchor::BOTTOM_LEFT),
            );
        }
    }
}

pub trait ToScreen {
    fn to_screen(self) -> Vec2;
}

impl ToScreen for Pos<Tiles> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.x * TILE.0, -self.y * TILE.0)
    }
}

impl ToScreen for Size<Tiles> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.width * TILE.0, self.height * TILE.0)
    }
}

pub trait ToTile {
    fn to_tile(self) -> Pos<Tiles>;
}

impl ToTile for Vec2 {
    fn to_tile(self) -> Pos<Tiles> {
        Pos::new(self.x / TILE.0, -self.y / TILE.0)
    }
}

pub fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_translation(pos.to_screen().extend(z))
}

pub fn dynamic_z(area_height: f32, base: f32, y: Tiles) -> f32 {
    base + (y + Tiles(1.0)).ratio(Tiles(area_height + 2.0))
}

pub fn atlas_rect(region: world::core::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.min().x,
        region.min().y,
        region.max().x,
        region.max().y,
    )
}

#[derive(Component)]
struct Animated {
    area: Id<AreaDef>,
    cell: TileRef,
}

fn place(
    commands: &mut Commands,
    area: &Area,
    sprite: Sprite,
    transform: Transform,
    cell: TileRef,
    anchor: Option<Anchor>,
) {
    let mut tile = commands.spawn((AreaTile, sprite, transform));
    if let Some(anchor) = anchor {
        tile.insert(anchor);
    }
    if area.animated(cell) {
        tile.insert(Animated {
            area: area.id,
            cell,
        });
    }
}

fn animate_tiles(time: Res<Time>, mut tiles: Query<(&Animated, &mut Sprite)>) {
    let now = Seconds(time.elapsed_secs());
    for (animated, mut sprite) in &mut tiles {
        if let Some(resolved) = area::areas()[animated.area.index()].resolve(animated.cell, now) {
            sprite.rect = Some(atlas_rect(resolved.region));
        }
    }
}

fn tile_sprite(assets: &AssetServer, sprite: &TileSprite, size: Vec2) -> Sprite {
    Sprite {
        image: assets.load(sprite.sheet.to_owned()),
        rect: Some(atlas_rect(sprite.region)),
        custom_size: Some(size),
        flip_x: sprite.flip.x,
        flip_y: sprite.flip.y,
        ..default()
    }
}
