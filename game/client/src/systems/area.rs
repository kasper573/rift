//! Renders the local player's area: when their area changes it clears the old map sprites and spawns
//! the new one via bevy_tiled, which also handles animation.

use bevy::prelude::*;
use world::core::table::Id;
use world::core::tiling::{CellPos, TileSize};
use world::systems::area::{self, AreaDef, AreaTag};
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

use crate::core::render::dynamic_z;
use crate::core::render::screen::ToScreen;

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>()
            .add_plugins(bevy_tiled::TileAnimationPlugin)
            .add_systems(
                Update,
                spawn_area_tiles.run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<Id<AreaDef>>);

fn spawn_area_tiles(
    me: Res<MyClient>,
    players: Query<(&Owner, &AreaTag)>,
    assets: Res<AssetServer>,
    mut spawned: ResMut<SpawnedArea>,
    tiles: Query<Entity, With<bevy_tiled::MapTile>>,
    mut images: ResMut<bevy::asset::Assets<Image>>,
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
    let mut hooks = AreaHooks::new(area, assets.clone());
    let origin = area.size.bounds().min().to_screen();
    bevy_tiled::spawn_map(&mut commands, &mut images, &area.map, &mut hooks, origin);
}

struct AreaHooks {
    assets: AssetServer,
    dynamic_layer: usize,
    height: f32,
    group_z: std::collections::HashMap<CellPos, f32>,
}

impl AreaHooks {
    fn new(area: &area::Area, assets: AssetServer) -> Self {
        let mut group_z = std::collections::HashMap::new();
        let dynamic_layer = area.dynamic_layer();
        for group in &area.groups {
            let z = dynamic_z(area.size.height, dynamic_layer as f32, group.bottom);
            for &(cell, _) in &group.tiles {
                group_z.insert(cell, z);
            }
        }
        AreaHooks {
            assets,
            dynamic_layer,
            height: area.size.height,
            group_z,
        }
    }
}

impl bevy_tiled::MapHooks for AreaHooks {
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        _images: &mut bevy::asset::Assets<Image>,
    ) -> Option<Handle<Image>> {
        let name = tileset.image.as_ref()?.source.file_name()?.to_str()?;
        let path = world::core::assets::find(world::core::assets::TILESETS, name)?;
        Some(self.assets.load(path))
    }

    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> f32 {
        if layer == self.dynamic_layer {
            let cell = CellPos::new(x, y);
            if let Some(&z) = self.group_z.get(&cell) {
                return z;
            }
        }
        layer as f32
    }

    fn object_z(&mut self, _above: usize, _x: f32, y: f32, _map_height: f32) -> f32 {
        dynamic_z(
            self.height,
            self.dynamic_layer as f32,
            world::core::tiling::Tiles(y / bevy_tiled::TILE - 0.5),
        )
    }
}
