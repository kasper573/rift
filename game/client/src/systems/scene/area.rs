use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::text_colored;
use world::core::assets::AssetService;
use world::core::tiling::{CellPos, TileSize};
use world::systems::area::{self, AreaTag};
use world::systems::player::Owner;
use world::systems::player::session::{self, MyClient};

use crate::core::render::dynamic_z;
use crate::core::render::screen::ToScreen;

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>()
            .add_plugins(bevy_tiled::TileAnimationPlugin)
            .add_systems(
                Update,
                (spawn_area_tiles, sync_death_banner).run_if(in_state(super::Scene::Area)),
            )
            .add_systems(
                OnExit(super::Scene::Area),
                crate::systems::despawn_all::<DeathBanner>,
            );
    }
}

#[derive(Resource, Default)]
struct SpawnedArea(Option<world::systems::area::Id>);

#[allow(clippy::too_many_arguments)]
fn spawn_area_tiles(
    me: Res<MyClient>,
    players: Query<(&Owner, &AreaTag)>,
    assets: Res<AssetServer>,
    service: Res<AssetService>,
    mut spawned: ResMut<SpawnedArea>,
    tiles: Query<Entity, With<bevy_tiled::MapTile>>,
    mut images: ResMut<bevy::asset::Assets<Image>>,
    mut meshes: ResMut<bevy::asset::Assets<Mesh>>,
    mut tilemaps: ResMut<bevy::asset::Assets<bevy_tiled::TilemapMaterial>>,
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
    let area = service.resolve(area_id, |a| area::build_area(a, area_id));
    let mut hooks = AreaHooks::new(area, assets.clone());
    let origin = area.size.bounds().min().to_screen();
    bevy_tiled::spawn_map(
        &mut commands,
        &mut images,
        &mut meshes,
        &mut tilemaps,
        &area.map,
        &mut hooks,
        origin,
    );
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
            for &cell in &group.tiles {
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
        let source = &tileset.image.as_ref()?.source;
        let key = crate::core::assets::key(source)?;
        Some(self.assets.load(key))
    }

    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> Option<f32> {
        if layer == self.dynamic_layer {
            return self.group_z.get(&CellPos::new(x, y)).copied();
        }
        None
    }

    fn object_z(&mut self, _above: usize, _x: f32, y: f32, _map_height: f32) -> f32 {
        dynamic_z(
            self.height,
            self.dynamic_layer as f32,
            world::core::tiling::Tiles(y / bevy_tiled::TILE - 0.5),
        )
    }
}

#[derive(Component, Default, Clone)]
struct DeathBanner;

fn sync_death_banner(world: &mut World) {
    let dead = session::is_dead(world);
    let banner = world
        .query_filtered::<Entity, With<DeathBanner>>()
        .iter(world)
        .next();
    match (dead, banner) {
        (true, None) => {
            let _ = world.spawn_scene(death_banner());
        }
        (false, Some(banner)) => world.entity_mut(banner).despawn(),
        _ => {}
    }
}

fn death_banner() -> impl Scene {
    bsn! {
        DeathBanner
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        GlobalZIndex({50})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored("You died! Press any key to respawn", Color::WHITE))} ]
    }
}
