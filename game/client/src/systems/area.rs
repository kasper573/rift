//! Renders the local player's area: when their area changes it clears the old map sprites and spawns
//! the new one through the shared [`bevy_tiled`] renderer, which also animates the flagged tiles.

use bevy::prelude::*;
use bevy_tiled::{AreaTile, TileAnimationPlugin, spawn_area};
use world::core::table::Id;
use world::systems::area::{self, AreaDef, AreaTag};
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>()
            .add_plugins(TileAnimationPlugin)
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
    spawn_area(&mut commands, &assets, &area::areas()[area_id.index()]);
}
