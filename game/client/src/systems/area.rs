//! Renders the area's static map: when the local player's area changes it respawns the tile sprites
//! layer by layer (depth-sorting the dynamic layer's groups against actors), and animates flagged tiles.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::table::Id;
use world::core::tiling::{Cell, GridDims, TileSize, Tiles};
use world::core::time::Seconds;
use world::systems::area::AreaTag;
use world::systems::area::{self, AreaDef, TileRef};
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

use crate::core::render::{TILE, atlas_rect, dynamic_z, sprite_transform};

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>().add_systems(
            Update,
            (spawn_area_tiles, animate_tiles).run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<Id<AreaDef>>);

#[derive(Component)]
pub(super) struct AreaTile;

#[derive(Component)]
pub(super) struct Animated(TileRef);

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

    let area = &area::areas()[area_id.index()];
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
            let mut tile = commands.spawn((
                AreaTile,
                tile_sprite(&assets, &sprite, Vec2::splat(TILE.0)),
                sprite_transform(c.center(), z),
            ));
            if area.animated(cell) {
                tile.insert(Animated(cell));
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
                    sprite_transform(c.center(), z),
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

fn animate_tiles(
    time: Res<Time>,
    spawned: Res<SpawnedArea>,
    mut tiles: Query<(&Animated, &mut Sprite)>,
) {
    let Some(area_id) = spawned.0 else {
        return;
    };
    let area = &area::areas()[area_id.index()];
    let now = Seconds(time.elapsed_secs());
    for (animated, mut sprite) in &mut tiles {
        if let Some(resolved) = area.resolve(animated.0, now) {
            sprite.rect = Some(atlas_rect(resolved.region));
        }
    }
}

fn tile_sprite(assets: &AssetServer, sprite: &area::TileSprite, size: Vec2) -> Sprite {
    Sprite {
        image: assets.load(sprite.sheet.to_owned()),
        rect: Some(atlas_rect(sprite.region)),
        custom_size: Some(size),
        flip_x: sprite.flip.x,
        flip_y: sprite.flip.y,
        ..default()
    }
}
