use std::collections::HashSet;

use crate::core::assets::AssetService;
use crate::core::audio::soundscape::Soundscape;
use crate::core::math::{Direction, Pos};
use crate::core::render::transition::{self, WorldViewSystems};
use crate::core::tiling::{CellPos, TileSize, Tiles};
use crate::core::time::Seconds;
use crate::systems::actor::Actor;
use crate::systems::area::{self, AreaTag};
use crate::systems::input::map::{self, InputAction};
use crate::systems::movement::RenderPosition;
use crate::systems::player::session::{self, Viewpoint};
use bevy::asset::AssetPath;
use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{RichPiece, RichText};

use crate::core::render::dynamic_z;
use crate::core::render::screen::ToScreen;

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>()
            .init_resource::<Crossing>()
            .init_resource::<RetainedImages>()
            .add_plugins(bevy_tiled::TileAnimationPlugin)
            .add_systems(
                Update,
                (
                    carry_view_across.before(WorldViewSystems),
                    (spawn_area_tiles, play_area_soundscape).in_set(WorldViewSystems),
                    retain_loaded_images,
                    sync_death_banner,
                )
                    .run_if(in_state(super::Scene::Area)),
            )
            .add_systems(
                OnExit(super::Scene::Area),
                (
                    crate::systems::scene::despawn_all::<DeathBanner>,
                    silence_soundscape,
                    forget_crossing,
                ),
            );
    }
}

/// A view held on screen this long without its new area becoming drawable is revealed regardless:
/// a missing image or a lost arrival must not hide the world for good.
const ARRIVAL_PATIENCE: Seconds = Seconds(5.0);

#[derive(Resource, Default)]
struct SpawnedArea {
    area: Option<crate::systems::area::Id>,
    images: Vec<Handle<Image>>,
}

#[derive(Resource, Default)]
struct Crossing {
    last_seen: Option<Pose>,
    held: Option<Held>,
}

#[derive(Clone, Copy)]
struct Pose {
    at: Pos<Tiles>,
    heading: Direction,
    area: area::Id,
}

#[derive(Clone, Copy)]
struct Held {
    since: Seconds,
    pose: Pose,
    /// The first frame that draws a newly drawable area uploads its images to the GPU, a long
    /// frame. The view is revealed only after it, so that frame passes under the full cover.
    rendered: bool,
}

fn carry_view_across(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Crossing { last_seen, held } = *world.resource::<Crossing>();
    let (last_seen, held) = match world.resource::<Viewpoint>().0 {
        Some(character) => match (pose_of(world, character), held, last_seen) {
            (None, _, _) => (last_seen, held),
            (Some(pose), None, Some(seen)) if seen.area != pose.area => {
                transition::freeze(world, seen.at, seen.heading);
                (Some(pose), Some(Held::new(now, seen)))
            }
            (Some(pose), Some(held), _) => {
                let drawable = area_drawable(world, pose.area);
                if now - held.since > ARRIVAL_PATIENCE || (drawable && held.rendered) {
                    transition::reveal(world, pose.at);
                    release_unshown_images(world);
                    (Some(pose), None)
                } else {
                    (
                        Some(pose),
                        Some(Held {
                            rendered: drawable,
                            ..held
                        }),
                    )
                }
            }
            (Some(pose), None, _) => (Some(pose), None),
        },
        None => match (held, last_seen) {
            (Some(_), _) if session::followed(world).is_none() => {
                transition::abort(world);
                (None, None)
            }
            (Some(held), _) if now - held.since > ARRIVAL_PATIENCE => {
                transition::reveal(world, held.pose.at);
                (None, None)
            }
            (None, Some(pose)) if session::followed(world).is_some() => {
                transition::freeze(world, pose.at, pose.heading);
                (None, Some(Held::new(now, pose)))
            }
            _ => (None, held),
        },
    };
    *world.resource_mut::<Crossing>() = Crossing { last_seen, held };
}

impl Held {
    fn new(since: Seconds, pose: Pose) -> Held {
        Held {
            since,
            pose,
            rendered: false,
        }
    }
}

fn pose_of(world: &World, character: Entity) -> Option<Pose> {
    let character = world.get_entity(character).ok()?;
    Some(Pose {
        at: character.get::<RenderPosition>()?.0,
        heading: character.get::<Actor>()?.dir,
        area: character.get::<AreaTag>()?.area,
    })
}

fn area_drawable(world: &mut World, area: area::Id) -> bool {
    let spawned = world.resource::<SpawnedArea>();
    if spawned.area != Some(area) {
        return false;
    }
    let mut pending = spawned.images.clone();
    pending.extend(
        world
            .query::<&Sprite>()
            .iter(world)
            .map(|sprite| sprite.image.clone()),
    );
    let images = world.resource::<Assets<Image>>();
    pending.iter().all(|image| images.contains(image))
}

fn forget_crossing(world: &mut World) {
    transition::abort(world);
    *world.resource_mut::<Crossing>() = Crossing::default();
    world.resource_mut::<RetainedImages>().0.clear();
}

/// Images loaded since the last crossing finished, and those it ended up showing, stay loaded until
/// the next one finishes. What the area left behind shares with the next (the player's own sheet, a
/// common tileset) then survives its departure, instead of unloading and decoding again moments
/// later.
#[derive(Resource, Default)]
struct RetainedImages(Vec<Handle<Image>>);

fn retain_loaded_images(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut retained: ResMut<RetainedImages>,
) {
    for event in events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = *event
            && let Some(image) = images.get_strong_handle(id)
        {
            retained.0.push(image);
        }
    }
}

fn release_unshown_images(world: &mut World) {
    let mut shown: HashSet<AssetId<Image>> = world
        .resource::<SpawnedArea>()
        .images
        .iter()
        .map(Handle::id)
        .collect();
    shown.extend(
        world
            .query::<&Sprite>()
            .iter(world)
            .map(|sprite| sprite.image.id()),
    );
    world
        .resource_mut::<RetainedImages>()
        .0
        .retain(|image| shown.contains(&image.id()));
}

#[allow(clippy::too_many_arguments)]
fn spawn_area_tiles(
    viewpoint: Res<Viewpoint>,
    areas: Query<&AreaTag>,
    assets: Res<AssetServer>,
    service: Res<AssetService>,
    mut spawned: ResMut<SpawnedArea>,
    tiles: Query<Entity, With<bevy_tiled::MapTile>>,
    mut images: ResMut<bevy::asset::Assets<Image>>,
    mut meshes: ResMut<bevy::asset::Assets<Mesh>>,
    mut tilemaps: ResMut<bevy::asset::Assets<bevy_tiled::TilemapMaterial>>,
    mut commands: Commands,
) {
    let Some(area_id) = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .map(|tag| tag.area)
    else {
        return;
    };
    if spawned.area == Some(area_id) {
        return;
    }
    for tile in &tiles {
        commands.entity(tile).despawn();
    }
    let area = area::load(&service, area_id);
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
    *spawned = SpawnedArea {
        area: Some(area_id),
        images: hooks.images,
    };
}

fn play_area_soundscape(
    viewpoint: Res<Viewpoint>,
    areas: Query<&AreaTag>,
    service: Res<AssetService>,
    mut soundscape: ResMut<Soundscape>,
) {
    let Some(area_id) = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .map(|tag| tag.area)
    else {
        return;
    };
    let zones = area::load(&service, area_id).soundscape.as_slice();
    if !std::ptr::eq(soundscape.0, zones) {
        soundscape.0 = zones;
    }
}

fn silence_soundscape(mut soundscape: ResMut<Soundscape>) {
    soundscape.0 = &[];
}

struct AreaHooks {
    assets: AssetServer,
    images: Vec<Handle<Image>>,
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
            images: Vec::new(),
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
        let source = tileset.image.as_ref()?.source.to_str()?;
        let image = self
            .assets
            .load(AssetPath::from("").resolve(&AssetPath::parse(source)));
        self.images.push(image.clone());
        Some(image)
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
            crate::core::tiling::Tiles(y / bevy_tiled::TILE - 0.5),
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
        GlobalZIndex({ui::tokens::layer::ALERTS})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(ui::rich_text(death_text(), false))} ]
    }
}

fn death_text() -> RichText {
    RichText {
        color: Color::WHITE,
        ..RichText::new(vec![
            RichPiece::text("You died! Press "),
            map::input(InputAction::Respawn),
            RichPiece::text(" to respawn"),
        ])
    }
}
