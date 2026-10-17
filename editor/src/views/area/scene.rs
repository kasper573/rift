use std::collections::HashMap;
use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget, Viewport};
use bevy::ecs::system::RunSystemOnce;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::content::Content;
use engine::core::math::{Pos, Rect};
use engine::core::render::{self, TILE, ToScreen};
use engine::core::tiling::{TileSize, Tiles};
use engine::systems::area::{self, AreaId, AreaModule, Flip, Scenery};
use engine::systems::screen::area::TileSheets;

use super::stamp;
use crate::project::{ProjectFiles, hash};

pub struct AreaSceneRequest {
    pub key: String,
    pub spec: AreaSceneSpec,
    pub width: u32,
    pub height: u32,
    pub center: Pos<Tiles>,
    pub zoom: f32,
    pub generation: u64,
    // A still scene keeps its last frame until something about it changes, so the world map's
    // many cards cost no rendering while nothing moves.
    pub live: bool,
}

#[derive(Clone, PartialEq)]
pub struct AreaSceneSpec {
    pub ground: String,
    pub scenery: SceneryOf,
    pub hidden_ground: Vec<String>,
    pub show_blueprints: bool,
    pub show_scenery: bool,
}

#[derive(Clone, PartialEq)]
pub enum SceneryOf {
    Area,
    Parts {
        blueprint: String,
        at: Pos<Tiles>,
        values: serde_json::Value,
    },
}

pub struct AreaScene {
    pub texture: egui::TextureId,
    pub image: Handle<Image>,
    pub alloc: [u32; 2],
    pub shown: [u32; 2],
    pub layer: usize,
    camera: Entity,
    spec: AreaSceneSpec,
    generation: u64,
    seen_at: Duration,
    zoom: f32,
    center: Pos<Tiles>,
    changed_at: Duration,
    active: bool,
}

impl AreaScene {
    pub fn uv(&self) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::Pos2::ZERO,
            egui::pos2(
                self.shown[0] as f32 / self.alloc[0].max(1) as f32,
                self.shown[1] as f32 / self.alloc[1].max(1) as f32,
            ),
        )
    }
}

#[derive(Component, Clone)]
pub struct AreaSceneOf(pub usize);

#[derive(Default)]
pub struct AreaScenes {
    pub by_key: HashMap<String, AreaScene>,
    images: HashMap<String, (String, Handle<Image>)>,
    next_layer: usize,
    free_layers: Vec<usize>,
}

const IDLE_FOR: Duration = Duration::from_secs(2);

// Spawning a scene builds its tile meshes, so a view asking for many at once (the world map's
// cards) gets them over a few frames instead of one long one.
const SPAWNS_PER_FRAME: usize = 6;

// A still scene keeps rendering for this long after a change, so its spawned tiles and uploaded
// images are all on the target before the camera rests.
const SETTLE_FOR: Duration = Duration::from_millis(50);

pub fn sync(
    scenes: &mut AreaScenes,
    requests: Vec<AreaSceneRequest>,
    content: Option<&Content>,
    files: &ProjectFiles,
    world: &mut World,
) {
    let now = world.resource::<Time<Real>>().elapsed();
    let mut spawned = 0;
    for request in requests {
        if let Some(scene) = scenes.by_key.get_mut(&request.key) {
            scene.seen_at = now;
        }
        let Some(content) = content else {
            continue;
        };
        let respawn = match scenes.by_key.get(&request.key) {
            Some(scene) => scene.generation != request.generation || scene.spec != request.spec,
            None => true,
        };
        if respawn {
            if spawned == SPAWNS_PER_FRAME {
                continue;
            }
            spawned += 1;
            despawn(scenes, &request.key, world);
            spawn(scenes, &request, content, files, world, now);
        }
        let Some(scene) = scenes.by_key.get_mut(&request.key) else {
            continue;
        };
        let wanted = [request.width.max(1), request.height.max(1)];
        let changed = respawn
            || scene.shown != wanted
            || scene.zoom != request.zoom
            || scene.center != request.center;
        if changed {
            scene.changed_at = now;
        }
        let active = request.live || now.saturating_sub(scene.changed_at) < SETTLE_FOR;
        let rest = !changed && active == scene.active;
        scene.zoom = request.zoom;
        scene.center = request.center;
        scene.active = active;
        if rest {
            continue;
        }
        let fit = [round_up(wanted[0]), round_up(wanted[1])];
        let grows = wanted[0] > scene.alloc[0] || wanted[1] > scene.alloc[1];
        let shrinks = scene.alloc[0] >= fit[0] * 2 || scene.alloc[1] >= fit[1] * 2;
        if grows || shrinks {
            let (image, texture) = render_target(world, fit);
            world
                .resource_mut::<EguiUserTextures>()
                .remove_image(&scene.image);
            scene.texture = texture;
            scene.image = image.clone();
            scene.alloc = fit;
            if let Ok(mut camera) = world.get_entity_mut(scene.camera) {
                camera.insert(RenderTarget::Image(image.into()));
            }
        }
        scene.shown = wanted;
        if let Ok(mut camera) = world.get_entity_mut(scene.camera) {
            if let Some(mut cam) = camera.get_mut::<Camera>() {
                cam.is_active = active;
                cam.viewport = Some(Viewport {
                    physical_position: UVec2::ZERO,
                    physical_size: UVec2::new(wanted[0], wanted[1]),
                    ..default()
                });
            }
            if let Some(mut projection) = camera.get_mut::<Projection>()
                && let Projection::Orthographic(ortho) = &mut *projection
            {
                ortho.scale = 1.0 / request.zoom.max(0.01);
            }
            if let Some(mut transform) = camera.get_mut::<Transform>() {
                let at = request.center.to_screen();
                transform.translation.x = at.x;
                transform.translation.y = at.y;
            }
        }
    }
    let stale: Vec<String> = scenes
        .by_key
        .iter()
        .filter(|(_, scene)| now.saturating_sub(scene.seen_at) > IDLE_FOR)
        .map(|(key, _)| key.clone())
        .collect();
    for key in stale {
        despawn(scenes, &key, world);
    }
}

pub fn clear(scenes: &mut AreaScenes, world: &mut World) {
    let keys: Vec<String> = scenes.by_key.keys().cloned().collect();
    for key in keys {
        despawn(scenes, &key, world);
    }
    scenes.images.clear();
}

fn round_up(n: u32) -> u32 {
    n.div_ceil(256) * 256
}

pub fn render_target(world: &mut World, alloc: [u32; 2]) -> (Handle<Image>, egui::TextureId) {
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(target_image(alloc));
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    (image, texture)
}

fn target_image(alloc: [u32; 2]) -> Image {
    let mut image =
        Image::new_target_texture(alloc[0], alloc[1], TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::nearest();
    image
}

fn despawn(scenes: &mut AreaScenes, key: &str, world: &mut World) {
    let Some(scene) = scenes.by_key.remove(key) else {
        return;
    };
    let tiles: Vec<Entity> = world
        .query::<(Entity, &AreaSceneOf)>()
        .iter(world)
        .filter(|(_, of)| of.0 == scene.layer)
        .map(|(entity, _)| entity)
        .collect();
    for entity in tiles {
        world.despawn(entity);
    }
    world
        .resource_mut::<EguiUserTextures>()
        .remove_image(&scene.image);
    scenes.free_layers.push(scene.layer);
}

fn spawn(
    scenes: &mut AreaScenes,
    request: &AreaSceneRequest,
    content: &Content,
    files: &ProjectFiles,
    world: &mut World,
    now: Duration,
) {
    let Some(area_id) = content.by_key::<area::AreaDef>(&request.spec.ground) else {
        return;
    };
    let layer = scenes.free_layers.pop().unwrap_or_else(|| {
        scenes.next_layer += 1;
        scenes.next_layer
    });
    let alloc = [
        round_up(request.width.max(1)),
        round_up(request.height.max(1)),
    ];
    let (image, texture) = render_target(world, alloc);
    let at = request.center.to_screen();
    let camera = world
        .spawn((
            Camera2d,
            RenderTarget::Image(image.clone().into()),
            Camera {
                order: -(10 + layer as isize),
                clear_color: ClearColorConfig::Custom(Color::srgb(0.09, 0.09, 0.11)),
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scale: 1.0 / request.zoom.max(0.01),
                ..OrthographicProjection::default_2d()
            }),
            Transform::from_xyz(at.x, at.y, 0.0),
            RenderLayers::from_layers(&[layer]),
            AreaSceneOf(layer),
        ))
        .id();
    let area = area::load(content, area_id);
    let scenery: Vec<Scenery> = match &request.spec.scenery {
        SceneryOf::Area => area::placed(content, area_id)
            .iter()
            .filter(|placed| {
                if placed.chain.is_empty() {
                    request.spec.show_scenery
                } else {
                    request.spec.show_blueprints
                }
            })
            .filter_map(|placed| match &placed.module {
                AreaModule::Scenery(scenery) => Some(scenery.clone()),
                _ => None,
            })
            .collect(),
        SceneryOf::Parts {
            blueprint,
            at,
            values,
        } => parts_scenery(content, blueprint, *at, values),
    };
    let sheets = sheets_for(scenes, content, files, &area.grids, &scenery, world);
    let spec = SpawnSpec {
        content: content.clone(),
        area: area_id,
        scenery,
        sheets,
        layer,
        hidden_ground: request.spec.hidden_ground.clone(),
    };
    world
        .run_system_once_with(spawn_tiles, spec)
        .expect("spawn_tiles runs on the editor world");
    scenes.by_key.insert(
        request.key.clone(),
        AreaScene {
            texture,
            image,
            alloc,
            shown: [request.width.max(1), request.height.max(1)],
            layer,
            camera,
            spec: request.spec.clone(),
            generation: request.generation,
            seen_at: now,
            zoom: request.zoom,
            center: request.center,
            changed_at: now,
            active: true,
        },
    );
}

pub fn scenery_extent(sceneries: &[Scenery]) -> Option<Rect<Tiles>> {
    sceneries
        .iter()
        .map(Scenery::bounds)
        .reduce(|a, b| a.union(&b))
}

pub fn parts_scenery(
    content: &Content,
    blueprint: &str,
    at: Pos<Tiles>,
    values: &serde_json::Value,
) -> Vec<Scenery> {
    let Some(module) = stamp::placement_module(content, blueprint, at, Flip::None, values) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    let flattening = area::blueprint::flatten(content, &[module], &mut errors);
    flattening
        .modules
        .into_iter()
        .filter_map(|placed| match placed.module {
            AreaModule::Scenery(scenery) => Some(scenery),
            _ => None,
        })
        .collect()
}

fn sheets_for(
    scenes: &mut AreaScenes,
    content: &Content,
    files: &ProjectFiles,
    grids: &area::AreaGrids,
    scenery: &[Scenery],
    world: &mut World,
) -> TileSheets {
    TileSheets::new(
        content,
        grids,
        scenery.iter().map(|scenery| scenery.tileset),
        |def| image_handle(scenes, files, def.image.as_str(), world),
    )
}

pub fn image_handle(
    scenes: &mut AreaScenes,
    files: &ProjectFiles,
    path: &str,
    world: &mut World,
) -> Handle<Image> {
    let file = format!("assets/{path}");
    let Some(bytes) = files.get(&file) else {
        return Handle::default();
    };
    let hash = hash(bytes);
    if let Some((known, handle)) = scenes.images.get(&file)
        && *known == hash
    {
        return handle.clone();
    }
    let handle = match image::load_from_memory(bytes) {
        Ok(decoded) => {
            let rgba = decoded.into_rgba8();
            let mut image = Image::new(
                Extent3d {
                    width: rgba.width(),
                    height: rgba.height(),
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba.into_raw(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = ImageSampler::nearest();
            world.resource_mut::<Assets<Image>>().add(image)
        }
        Err(_) => Handle::default(),
    };
    scenes.images.insert(file, (hash, handle.clone()));
    handle
}

struct SpawnSpec {
    content: Content,
    area: AreaId,
    scenery: Vec<Scenery>,
    sheets: TileSheets,
    layer: usize,
    hidden_ground: Vec<String>,
}

fn spawn_tiles(
    In(spec): In<SpawnSpec>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<bevy_tilemap::TilemapMaterial>>,
) {
    let area = area::load(&spec.content, spec.area);
    let grids = area.grids.clone();
    let size = grids.size;
    let origin = area.size.bounds().min().to_screen();
    let scene_tag = (
        AreaSceneOf(spec.layer),
        RenderLayers::from_layers(&[spec.layer]),
    );
    for (index, layer) in grids.ground.iter().enumerate() {
        if spec.hidden_ground.contains(&layer.name) {
            continue;
        }
        let cells: Vec<Option<bevy_tilemap::GridCell>> = layer
            .cells
            .iter()
            .map(|cell| {
                let entry = grids.entry(*cell)?;
                Some(bevy_tilemap::GridCell {
                    sheet: spec.sheets.index(entry.tileset)?,
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
            &spec.sheets.sheets,
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
            scene_tag.clone(),
        );
    }
    let flat_count = spec
        .scenery
        .iter()
        .filter(|s| s.sort_line().is_none())
        .count();
    let mut flat_index = 0;
    for scenery in &spec.scenery {
        let Some(sheet) = spec.sheets.index(scenery.tileset) else {
            continue;
        };
        let z = match scenery.sort_line() {
            Some(line) => render::dynamic_z(area.size.height, area.sort_base(), line),
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
                    &spec.sheets.sheets,
                    bevy_tilemap::TileSprite {
                        sheet,
                        tile: tile.tile,
                        flip_x: flip.horizontal(),
                        flip_y: flip.vertical(),
                        bottom_left: square.max().to_screen().with_x(square.min().to_screen().x),
                        z,
                    },
                    TILE.0,
                    scene_tag.clone(),
                );
            }
        }
    }
}
