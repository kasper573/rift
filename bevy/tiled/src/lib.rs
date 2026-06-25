//! A small, game-agnostic renderer for a [`tiled::Map`]: tile layers, tile-objects, flips, and per-tile
//! animation. Cells are merged into meshes (one per tileset+depth, animated ones per animation) so cost
//! scales with distinct depths, not tile count. A caller supplies image loading and per-tile depth
//! through [`MapHooks`]; [`Files`] is a ready hook for off-disk tools.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::sprite::Anchor;

/// Logical pixels per tile — the art's native size.
pub const TILE: f32 = 16.0;

/// Tags every entity a map spawns, so a caller can despawn them all to swap maps.
#[derive(Component)]
pub struct MapTile;

/// The caller-supplied integration points: image loading and per-tile/object depth.
pub trait MapHooks {
    /// Resolves a tileset's image to a loaded handle.
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>>;

    /// Depth of a tile-layer cell; `layer` is the layer's file-order index.
    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> f32 {
        let _ = (x, y);
        layer as f32
    }

    /// Depth of a tile-object at its foot (`y` in downward pixels).
    fn object_z(&mut self, above: usize, x: f32, y: f32, map_height: f32) -> f32 {
        let _ = x;
        above as f32 + (y / TILE + 1.0) / (map_height + 2.0)
    }
}

/// Draws `map`'s tile-layer cells (merged) and tile-objects via `hooks`. `origin` is the screen
/// position of the map's top-left corner ([`Vec2::ZERO`] draws raw).
pub fn spawn_map(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    map: &tiled::Map,
    hooks: &mut impl MapHooks,
    origin: Vec2,
) {
    let map_height = map.height as f32;
    let mut statics: HashMap<(AssetId<Image>, u32), Batch> = HashMap::new();
    let mut animated: HashMap<(AssetId<Image>, u32, u32), AnimBatch> = HashMap::new();
    let mut sheets: HashMap<usize, Option<Handle<Image>>> = HashMap::new();
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
                        let Some(sheet) = resolve_sheet(&mut sheets, hooks, images, tileset) else {
                            continue;
                        };
                        let z = hooks.tile_z(layer, x, y);
                        let center = Vec2::new(
                            origin.x + (x as f32 + 0.5) * TILE,
                            origin.y - (y as f32 + 0.5) * TILE,
                        );
                        let atlas = atlas_size(tileset);
                        let cells = frames(tileset, tile.id());
                        if cells.len() > 1 {
                            animated
                                .entry((sheet.id(), z.to_bits(), tile.id()))
                                .or_insert_with(|| AnimBatch::new(sheet, z, atlas, cells))
                                .push(center, Vec2::splat(TILE), tile.flip_h, tile.flip_v);
                        } else {
                            statics
                                .entry((sheet.id(), z.to_bits()))
                                .or_insert_with(|| Batch::new(sheet, z, atlas))
                                .push(
                                    cells[0].0,
                                    center,
                                    Vec2::splat(TILE),
                                    tile.flip_h,
                                    tile.flip_v,
                                );
                        }
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
                    let Some(sheet) = resolve_sheet(&mut sheets, hooks, images, tileset) else {
                        continue;
                    };
                    let size = Vec2::new(tileset.tile_width as f32, tileset.tile_height as f32);
                    let z = hooks.object_z(layer, object.x, object.y, map_height);
                    let transform =
                        Transform::from_xyz(origin.x + object.x, origin.y - object.y, z);
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
    for batch in statics.into_values() {
        let (sheet, z) = (batch.sheet.clone(), batch.z);
        commands.spawn((
            MapTile,
            Mesh2d(meshes.add(batch.mesh())),
            MeshMaterial2d(materials.add(ColorMaterial {
                texture: Some(sheet),
                ..default()
            })),
            Transform::from_xyz(0.0, 0.0, z),
        ));
    }
    for batch in animated.into_values() {
        let (sheet, z) = (batch.sheet.clone(), batch.z);
        let (mesh, animation) = batch.build();
        commands.spawn((
            MapTile,
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(ColorMaterial {
                texture: Some(sheet),
                uv_transform: animation.frames[0].0,
                ..default()
            })),
            Transform::from_xyz(0.0, 0.0, z),
            animation,
        ));
    }
}

struct Batch {
    sheet: Handle<Image>,
    z: f32,
    atlas: Vec2,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Batch {
    fn new(sheet: Handle<Image>, z: f32, atlas: Vec2) -> Batch {
        Batch {
            sheet,
            z,
            atlas,
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn push(&mut self, region: Rect, center: Vec2, size: Vec2, flip_x: bool, flip_y: bool) {
        let base = self.positions.len() as u32;
        self.positions.extend(quad_positions(center, size));
        self.uvs
            .extend(quad_uvs(region, self.atlas, flip_x, flip_y));
        self.indices
            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }

    fn mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

struct AnimBatch {
    sheet: Handle<Image>,
    z: f32,
    atlas: Vec2,
    frames: Vec<(Rect, f32)>,
    positions: Vec<[f32; 3]>,
    flips: Vec<(bool, bool)>,
    indices: Vec<u32>,
}

impl AnimBatch {
    fn new(sheet: Handle<Image>, z: f32, atlas: Vec2, frames: Vec<(Rect, f32)>) -> AnimBatch {
        AnimBatch {
            sheet,
            z,
            atlas,
            frames,
            positions: Vec::new(),
            flips: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn push(&mut self, center: Vec2, size: Vec2, flip_x: bool, flip_y: bool) {
        let base = self.positions.len() as u32;
        self.positions.extend(quad_positions(center, size));
        self.flips.push((flip_x, flip_y));
        self.indices
            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }

    fn build(self) -> (Mesh, Animated) {
        let total = self.frames.iter().map(|&(_, duration)| duration).sum();
        let uvs: Vec<[f32; 2]> = self
            .flips
            .iter()
            .flat_map(|&(flip_x, flip_y)| unit_quad_uvs(flip_x, flip_y))
            .collect();
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(self.indices));
        let frames = self
            .frames
            .iter()
            .map(|&(region, duration)| (frame_transform(region, self.atlas), duration))
            .collect();
        (
            mesh,
            Animated {
                frames,
                total,
                current: 0,
            },
        )
    }
}

#[derive(Component)]
struct Animated {
    frames: Vec<(Affine2, f32)>,
    total: f32,
    current: usize,
}

#[derive(Component)]
struct AnimatedSprite {
    frames: Vec<(Rect, f32)>,
    total: f32,
}

/// Drives tile animation; add it wherever a map is shown.
pub struct TileAnimationPlugin;

impl Plugin for TileAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (animate, animate_sprites));
    }
}

fn animate(
    time: Res<Time>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut tiles: Query<(&mut Animated, &MeshMaterial2d<ColorMaterial>)>,
) {
    let now = time.elapsed_secs();
    for (mut anim, material) in &mut tiles {
        if anim.total <= 0.0 {
            continue;
        }
        let frame = frame_at(&anim.frames, anim.total, now);
        if frame == anim.current {
            continue;
        }
        anim.current = frame;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.uv_transform = anim.frames[frame].0;
        }
    }
}

fn animate_sprites(time: Res<Time>, mut sprites: Query<(&AnimatedSprite, &mut Sprite)>) {
    let now = time.elapsed_secs();
    for (anim, mut sprite) in &mut sprites {
        if anim.total > 0.0 {
            sprite.rect = Some(anim.frames[frame_at(&anim.frames, anim.total, now)].0);
        }
    }
}

fn frame_at<T>(frames: &[(T, f32)], total: f32, now: f32) -> usize {
    let mut remaining = now % total;
    for (index, (_, duration)) in frames.iter().enumerate() {
        if remaining < *duration {
            return index;
        }
        remaining -= *duration;
    }
    frames.len() - 1
}

fn frame_transform(region: Rect, atlas: Vec2) -> Affine2 {
    Affine2::from_scale_angle_translation(region.size() / atlas, 0.0, region.min / atlas)
}

fn unit_quad_uvs(flip_x: bool, flip_y: bool) -> [[f32; 2]; 4] {
    let (u0, u1) = if flip_x { (1.0, 0.0) } else { (0.0, 1.0) };
    let (v0, v1) = if flip_y { (1.0, 0.0) } else { (0.0, 1.0) };
    [[u0, v0], [u1, v0], [u1, v1], [u0, v1]]
}

fn quad_positions(center: Vec2, size: Vec2) -> [[f32; 3]; 4] {
    let half = size / 2.0;
    [
        [center.x - half.x, center.y + half.y, 0.0],
        [center.x + half.x, center.y + half.y, 0.0],
        [center.x + half.x, center.y - half.y, 0.0],
        [center.x - half.x, center.y - half.y, 0.0],
    ]
}

fn quad_uvs(region: Rect, atlas: Vec2, flip_x: bool, flip_y: bool) -> [[f32; 2]; 4] {
    let (mut u0, mut u1) = (region.min.x / atlas.x, region.max.x / atlas.x);
    let (mut v0, mut v1) = (region.min.y / atlas.y, region.max.y / atlas.y);
    if flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    if flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    [[u0, v0], [u1, v0], [u1, v1], [u0, v1]]
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
        tile.insert(AnimatedSprite { frames, total });
    }
}

/// Memoized per tileset: the hook's image lookup is too costly to repeat per cell.
fn resolve_sheet(
    sheets: &mut HashMap<usize, Option<Handle<Image>>>,
    hooks: &mut impl MapHooks,
    images: &mut Assets<Image>,
    tileset: &tiled::Tileset,
) -> Option<Handle<Image>> {
    let key = tileset as *const tiled::Tileset as usize;
    if let Some(cached) = sheets.get(&key) {
        return cached.clone();
    }
    let resolved = hooks.image(tileset, images);
    sheets.insert(key, resolved.clone());
    resolved
}

fn atlas_size(tileset: &tiled::Tileset) -> Vec2 {
    match &tileset.image {
        Some(image) => Vec2::new(image.width as f32, image.height as f32),
        None => Vec2::new(
            (tileset.columns * tileset.tile_width).max(1) as f32,
            (tileset.tile_height).max(1) as f32,
        ),
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

/// A [`MapHooks`] for off-disk tools: loads tileset images from the filesystem.
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
