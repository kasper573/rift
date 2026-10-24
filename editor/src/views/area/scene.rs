use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use ui::egui;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget, Viewport};
use bevy::ecs::system::RunSystemOnce;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use engine::core::content::Content;
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::{TILE, ToScreen};
use engine::core::units::{Meters, TileSize};
use engine::systems::area::blueprint::Placement;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaGrids, AreaModule, Picture, Scenery};
use engine::systems::screen::area::TileSheets;

use crate::project::ProjectFiles;

pub struct AreaSceneRequest {
    pub key: String,
    pub spec: AreaSceneSpec,
    pub width: u32,
    pub height: u32,
    pub center: Pos<Meters>,
    pub zoom: f32,
    pub generation: u64,
    // A still scene keeps its last frame until something about it changes, so the world map's
    // many cards cost no rendering while nothing moves.
    pub live: bool,
}

#[derive(Clone, PartialEq)]
pub struct AreaSceneSpec {
    pub drawn: Drawn,
    pub hidden_ground: Vec<String>,
    /// Whether the scene clears to nothing, so what is drawn under it shows where it paints no
    /// cell.
    pub see_through: bool,
}

impl AreaSceneSpec {
    pub fn area(path: &str) -> AreaSceneSpec {
        AreaSceneSpec {
            drawn: Drawn::Area(path.to_owned()),
            hidden_ground: Vec::new(),
            see_through: false,
        }
    }

    /// The ground of the area at `path` alone, as a backdrop under a bundle's own picture.
    pub fn ground(path: &str) -> AreaSceneSpec {
        AreaSceneSpec {
            drawn: Drawn::Ground(path.to_owned()),
            ..AreaSceneSpec::area(path)
        }
    }
}

/// What a scene draws: an area as the game builds it, only its ground, or a picture the editor
/// made of a bundle.
#[derive(Clone)]
pub enum Drawn {
    Area(String),
    Ground(String),
    Picture(Arc<Picture>),
}

impl PartialEq for Drawn {
    fn eq(&self, other: &Drawn) -> bool {
        match (self, other) {
            (Drawn::Area(a), Drawn::Area(b)) | (Drawn::Ground(a), Drawn::Ground(b)) => a == b,
            (Drawn::Picture(a), Drawn::Picture(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
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
    center: Pos<Meters>,
    changed_at: Duration,
    settling: u32,
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
    pub by_path: HashMap<String, AreaScene>,
    images: HashMap<String, (String, Handle<Image>)>,
    next_layer: usize,
    free_layers: Vec<usize>,
}

const IDLE_FOR: Duration = Duration::from_secs(2);

// Spawning a scene builds its tile meshes, so a view asking for many at once (the world map's
// cards) gets them over a few frames instead of one long one.
const SPAWNS_PER_FRAME: usize = 6;

// A still scene keeps rendering for this long and this many frames after a change, so its
// spawned tiles and uploaded images are all on the target before the camera rests. Time alone
// isn't enough: a frame that spawns a large scene can outlast it, leaving one render made while
// its tiles' materials were still being prepared.
const SETTLE_FOR: Duration = Duration::from_millis(50);
const SETTLE_FRAMES: u32 = 4;

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
        if let Some(scene) = scenes.by_path.get_mut(&request.key) {
            scene.seen_at = now;
        }
        let Some(content) = content else {
            continue;
        };
        let respawn = match scenes.by_path.get(&request.key) {
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
        let Some(scene) = scenes.by_path.get_mut(&request.key) else {
            continue;
        };
        let wanted = [request.width.max(1), request.height.max(1)];
        let changed = respawn
            || scene.shown != wanted
            || scene.zoom != request.zoom
            || scene.center != request.center;
        if changed {
            scene.changed_at = now;
            scene.settling = SETTLE_FRAMES;
        }
        let settled = now.saturating_sub(scene.changed_at) >= SETTLE_FOR && scene.settling == 0;
        scene.settling = scene.settling.saturating_sub(1);
        let active = request.live || !settled;
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
        .by_path
        .iter()
        .filter(|(_, scene)| now.saturating_sub(scene.seen_at) > IDLE_FOR)
        .map(|(key, _)| key.clone())
        .collect();
    for key in stale {
        despawn(scenes, &key, world);
    }
}

pub fn clear(scenes: &mut AreaScenes, world: &mut World) {
    let keys: Vec<String> = scenes.by_path.keys().cloned().collect();
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
    let Some(scene) = scenes.by_path.remove(key) else {
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
    let picture = match &request.spec.drawn {
        Drawn::Area(path) | Drawn::Ground(path) => {
            let Some(id) = content.by_path::<area::AreaDef>(path) else {
                return;
            };
            let area = area::load(content, id);
            let scenery = match request.spec.drawn {
                Drawn::Ground(_) => Vec::new(),
                _ => area.scenery.clone(),
            };
            Pictured {
                size: area.size,
                sort_base: area.sort_base(),
                ground: area.grids.clone(),
                scenery,
            }
        }
        Drawn::Picture(picture) => Pictured {
            size: picture.area.size,
            sort_base: picture.area.sort_base(),
            ground: picture.ground.clone(),
            scenery: picture.scenery.clone(),
        },
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
                clear_color: ClearColorConfig::Custom(if request.spec.see_through {
                    Color::NONE
                } else {
                    Color::srgb(0.09, 0.09, 0.11)
                }),
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
    let sheets = sheets_for(
        scenes,
        content,
        files,
        &picture.ground,
        &picture.scenery,
        world,
    );
    let spec = SpawnSpec {
        picture,
        sheets,
        layer,
        hidden_ground: request.spec.hidden_ground.clone(),
    };
    world
        .run_system_once_with(spawn_tiles, spec)
        .expect("spawn_tiles runs on the editor world");
    scenes.by_path.insert(
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
            settling: SETTLE_FRAMES,
            active: true,
        },
    );
}

pub fn scenery_extent(sceneries: &[Scenery]) -> Option<Rect<Meters>> {
    sceneries
        .iter()
        .map(Scenery::bounds)
        .reduce(|a, b| a.union(&b))
}

/// The scenery a placement brings, its ids those of `content`.
pub fn parts_scenery(content: &Content, placement: &Placement) -> Vec<Scenery> {
    let placement = SceneNode::Module(ModuleNode::new(AreaModule::Blueprint(placement.clone())));
    let mut errors = Vec::new();
    let flattening = area::blueprint::flatten(content, &[placement], &mut errors);
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
    let (Some(bytes), Some(hash)) = (files.get(path), files.hash_of(path)) else {
        return Handle::default();
    };
    if let Some((known, handle)) = scenes.images.get(path)
        && known == hash
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
    scenes
        .images
        .insert(path.to_owned(), (hash.to_owned(), handle.clone()));
    handle
}

/// What a scene spawns tiles for.
struct Pictured {
    size: Size<Meters>,
    sort_base: f32,
    ground: Arc<AreaGrids>,
    scenery: Vec<Scenery>,
}

struct SpawnSpec {
    picture: Pictured,
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
    let picture = &spec.picture;
    let grids = picture.ground.clone();
    let size = grids.size;
    let origin = picture.size.bounds().min().to_screen();
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
    let depths = area::scenery_depths(&picture.scenery, picture.size.height, picture.sort_base);
    for (scenery, z) in picture.scenery.iter().zip(depths) {
        let Some(sheet) = spec.sheets.index(scenery.tileset) else {
            continue;
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
