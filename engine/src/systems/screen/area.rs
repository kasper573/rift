use std::collections::HashSet;
use std::time::Duration;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::{TextRole, color};

use super::ClientScreen;
use crate::core::audio::soundscape::Soundscape;
use crate::core::content::Content;
use crate::core::content::media::ImageSizes;
use crate::core::gui::{self, Band};
use crate::core::interpolate::RenderValueSystems;
use crate::core::math::{Direction, Pos};
use crate::core::render::dynamic_z;
use crate::core::render::screen::ToScreen;
use crate::core::render::transition::{
    self, ActiveTransition, ScreenTransitionPhase, WorldViewSystems,
};
use crate::core::render::{GroundDepth, RowDepths, TILE};
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::tiling::TileSize;
use crate::core::tiling::Tiles;
use crate::core::time::Seconds;
use crate::systems::actor::Actor;
use crate::systems::area::tileset::TilesetId;
use crate::systems::area::{self, AreaId, AreaTag, tileset};
use crate::systems::input::map::{self, InputAction};
use crate::systems::movement::RenderPosition;
use crate::systems::player::session::{self, MyCharacter};
use crate::systems::player::{ARRIVAL_PATIENCE, AreaDrawn};

pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedArea>()
            .init_resource::<Crossing>()
            .init_resource::<RetainedImages>()
            .add_plugins(bevy_tilemap::TilemapPlugin)
            .add_systems(
                Update,
                (
                    carry_view_across
                        .after(RenderValueSystems)
                        .before(WorldViewSystems),
                    (spawn_area_tiles, play_area_soundscape).in_set(WorldViewSystems),
                    retain_loaded_images,
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                death_notice
                    .in_set(Band::Notices)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnEnter(ClientScreen::Area), restart_patience)
            .add_systems(OnExit(ClientScreen::Area), (silence_soundscape, leave_area));
    }
}

#[derive(Resource, Default)]
struct SpawnedArea {
    area: Option<AreaId>,
    images: Vec<Handle<Image>>,
}

#[derive(Resource, Default, Clone, Copy)]
struct Crossing {
    last_seen: Option<Pose>,
    held: Option<Held>,
}

#[derive(Clone, Copy)]
struct Pose {
    character: Entity,
    at: Pos<Tiles>,
    heading: Direction,
    area: AreaId,
}

#[derive(Clone, Copy)]
struct Held {
    since: Seconds,
    pose: Pose,
    /// The first frame that draws a newly drawable area uploads its images to the GPU, a long
    /// frame. The view is revealed only after it, so that frame passes under the full cover.
    rendered: bool,
}

/// The view is revealed when no crossing is held under the veil and no transition plays over it.
pub fn settled(world: &World) -> bool {
    world.resource::<Crossing>().held.is_none()
        && world.resource::<ActiveTransition>().phase() == ScreenTransitionPhase::Idle
}

pub fn carry_view_across(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Crossing { last_seen, held } = *world.resource::<Crossing>();
    let (last_seen, held) = match session::me(world) {
        Some(character) => match (pose_of(world, character), held, last_seen) {
            (None, _, _) => (last_seen, held),
            (Some(pose), None, Some(seen)) if seen.area != pose.area => {
                transition::freeze(world, seen.at, seen.heading);
                (Some(pose), Some(Held::new(now, seen)))
            }
            (Some(pose), None, Some(seen)) if seen.character == pose.character => {
                (Some(pose), None)
            }
            (Some(pose), None, _) => (Some(pose), Some(Held::new(now, pose))),
            (Some(pose), Some(held), _) => {
                let drawable = area_drawable(world, pose.area);
                if now - held.since > ARRIVAL_PATIENCE || (drawable && held.rendered) {
                    reveal(world, pose);
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
        },
        None => match (held, last_seen) {
            (Some(held), _) if now - held.since > ARRIVAL_PATIENCE => {
                transition::reveal(world, held.pose.at);
                (None, None)
            }
            (None, Some(pose)) if session::my_id(world).is_some() => {
                transition::freeze(world, pose.at, pose.heading);
                (None, Some(Held::new(now, pose)))
            }
            _ => (None, held),
        },
    };
    *world.resource_mut::<Crossing>() = Crossing { last_seen, held };
}

fn reveal(world: &mut World, pose: Pose) {
    transition::reveal(world, pose.at);
    release_unshown_images(world);
    world.write_message(AreaDrawn);
}

fn restart_patience(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    if let Some(held) = world.resource_mut::<Crossing>().held.as_mut() {
        held.since = now;
    }
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
    let entity = world.get_entity(character).ok()?;
    Some(Pose {
        character,
        at: entity.get::<RenderPosition>()?.0,
        heading: entity.get::<Actor>()?.dir,
        area: entity.get::<AreaTag>()?.area,
    })
}

fn area_drawable(world: &mut World, area: AreaId) -> bool {
    if world.resource::<SpawnedArea>().area != Some(area) {
        return false;
    }
    let on_show = images_on_show(world);
    let images = world.resource::<Assets<Image>>();
    on_show.iter().all(|image| images.contains(image))
}

fn images_on_show(world: &mut World) -> Vec<Handle<Image>> {
    let spawned = world.resource::<SpawnedArea>().images.clone();
    spawned
        .into_iter()
        .chain(
            world
                .query::<&Sprite>()
                .iter(world)
                .map(|sprite| sprite.image.clone()),
        )
        .collect()
}

/// Leaving the area scene keeps the last view under the veil, so a lost connection or a content
/// deploy shows the loading and reconnect cards over a frozen world and reveals the next one from
/// where the character lands.
fn leave_area(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Crossing { last_seen, held } = *world.resource::<Crossing>();
    let held = match (last_seen, held) {
        (Some(pose), None) => {
            transition::freeze(world, pose.at, pose.heading);
            Some(Held::new(now, pose))
        }
        (Some(_), Some(held)) => Some(held),
        (None, _) => {
            transition::abort(world);
            None
        }
    };
    *world.resource_mut::<Crossing>() = Crossing {
        last_seen: None,
        held,
    };
    world.resource_mut::<SpawnedArea>().area = None;
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
    let shown: HashSet<AssetId<Image>> = images_on_show(world).iter().map(Handle::id).collect();
    world
        .resource_mut::<RetainedImages>()
        .0
        .retain(|image| shown.contains(&image.id()));
}

#[allow(clippy::too_many_arguments)]
fn spawn_area_tiles(
    viewpoint: Res<MyCharacter>,
    areas: Query<&AreaTag>,
    assets: Res<AssetServer>,
    content: Res<Content>,
    mut spawned: ResMut<SpawnedArea>,
    tiles: Query<Entity, With<bevy_tilemap::MapTile>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<bevy_tilemap::TilemapMaterial>>,
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
    let area = area::load(&content, area_id);
    commands.insert_resource(GroundDepth(area.sort_base()));
    commands.insert_resource(RowDepths {
        height: area.size.height,
        base: area.sort_base(),
    });
    let origin = area.size.bounds().min().to_screen();
    let mut loaded = Vec::new();
    let sheets = TileSheets::new(
        &content,
        &area.grids,
        area.scenery.iter().map(|scenery| scenery.tileset),
        |def| {
            let image = assets.load(def.image.path());
            loaded.push(image.clone());
            image
        },
    );
    let grids = &area.grids;
    let size = grids.size;
    for (index, layer) in grids.ground.iter().enumerate() {
        let cells: Vec<Option<bevy_tilemap::GridCell>> = layer
            .cells
            .iter()
            .map(|cell| {
                let entry = grids.entry(*cell)?;
                Some(bevy_tilemap::GridCell {
                    sheet: sheets.index(entry.tileset)?,
                    tile: entry.tile,
                    flip_x: entry.flip.horizontal(),
                    flip_y: entry.flip.vertical(),
                })
            })
            .collect();
        bevy_tilemap::spawn_grid_layer(
            &mut commands,
            &mut bevy_tilemap::TilemapAssets {
                images: &mut images,
                meshes: &mut meshes,
                materials: &mut materials,
            },
            &sheets.sheets,
            &bevy_tilemap::GridLayer {
                width: size.width,
                height: size.height,
                cells: &cells,
            },
            bevy_tilemap::LayerPlacement {
                origin,
                tile_px: TILE.0,
                z: index as f32,
            },
            (),
        );
    }
    let flat_count = area
        .scenery
        .iter()
        .filter(|scenery| scenery.sort_line().is_none())
        .count();
    let mut flat_index = 0;
    for scenery in &area.scenery {
        let Some(sheet) = sheets.index(scenery.tileset) else {
            continue;
        };
        let z = match scenery.sort_line() {
            Some(line) => dynamic_z(area.size.height, area.sort_base(), line),
            None => {
                flat_index += 1;
                area.sort_base() - 1.0 + flat_index as f32 / (flat_count as f32 + 1.0)
            }
        };
        for (row, tiles) in scenery.tiles.iter().enumerate() {
            for (column, tile) in tiles.iter().enumerate() {
                let Some(tile) = tile else {
                    continue;
                };
                let square = area::walk::tile_square(scenery, column, row);
                let flip = tile.flip.compose(scenery.flip);
                bevy_tilemap::spawn_tile_sprite(
                    &mut commands,
                    &sheets.sheets,
                    bevy_tilemap::TileSprite {
                        sheet,
                        tile: tile.tile,
                        flip_x: flip.horizontal(),
                        flip_y: flip.vertical(),
                        bottom_left: square.max().to_screen().with_x(square.min().to_screen().x),
                        z,
                    },
                    TILE.0,
                    (),
                );
            }
        }
    }
    *spawned = SpawnedArea {
        area: Some(area_id),
        images: loaded,
    };
}

fn play_area_soundscape(
    viewpoint: Res<MyCharacter>,
    areas: Query<&AreaTag>,
    content: Res<Content>,
    mut soundscape: ResMut<Soundscape>,
) {
    let Some(area_id) = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .map(|tag| tag.area)
    else {
        return;
    };
    let zones = &area::load(&content, area_id).soundscape;
    if soundscape.0 != *zones {
        soundscape.0.clone_from(zones);
    }
}

fn silence_soundscape(mut soundscape: ResMut<Soundscape>) {
    soundscape.0.clear();
}

pub struct TileSheets {
    pub tilesets: Vec<TilesetId>,
    pub sheets: Vec<bevy_tilemap::TileSheet>,
}

impl TileSheets {
    pub fn new(
        content: &Content,
        grids: &area::AreaGrids,
        scenery: impl Iterator<Item = TilesetId>,
        mut image_of: impl FnMut(&tileset::TilesetDef) -> Handle<Image>,
    ) -> TileSheets {
        let sizes = content.built::<ImageSizes>();
        let mut tilesets: Vec<TilesetId> = grids
            .palette
            .iter()
            .map(|entry| entry.tileset)
            .chain(scenery)
            .collect();
        tilesets.sort_by_key(|id| id.index());
        tilesets.dedup();
        let sheets = tilesets
            .iter()
            .map(|&id| {
                let def = id.get(content);
                let (width, height) = sizes.0.get(def.image.as_str()).copied().unwrap_or((0, 0));
                bevy_tilemap::TileSheet {
                    image: image_of(def),
                    size: Vec2::new(width as f32, height as f32),
                    tile: Vec2::splat(def.tile as f32),
                    animations: def
                        .animations(width, height)
                        .map(|(tile, frames)| bevy_tilemap::TileAnimation {
                            tile,
                            frames: frames
                                .iter()
                                .map(|frame| bevy_tilemap::TileAnimationFrame {
                                    tile: frame.tile,
                                    lasts: Duration::from_millis(u64::from(frame.ms)),
                                })
                                .collect(),
                        })
                        .collect(),
                }
            })
            .collect();
        TileSheets { tilesets, sheets }
    }

    pub fn index(&self, tileset: TilesetId) -> Option<usize> {
        self.tilesets.iter().position(|&id| id == tileset)
    }
}

/// The death notice reads over any ground, so its line sits in a popup frame. It takes no clicks:
/// respawning is a key the line names.
fn death_notice(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    if !session::is_dead(world) {
        return;
    }
    let line = StyledLine::new(vec![
        TextPiece::text("You died! Press "),
        map::input(InputAction::Respawn),
        TextPiece::text(" to respawn"),
    ]);
    let area = egui::Area::new(egui::Id::new("death"))
        .order(Band::Notices.order())
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .interactable(false);
    gui::show_anchored(&gui, area, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let look = TextLook {
                size: TextRole::Body.size(),
                color: color::TEXT,
            };
            text::show(ui, world, &line, look, None);
        });
    });
}
