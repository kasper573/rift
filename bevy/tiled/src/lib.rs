//! A small, general-purpose renderer that draws a [`tiled::Map`] for Bevy — finite tile layers
//! (stacked by file order), tile-objects, flips, and per-tile animation. It is deliberately
//! game-agnostic: it knows nothing about any particular world model. Callers plug in the two things
//! that *are* theirs — how to load a tileset's image, and what depth (`z`) each tile and object gets —
//! through [`MapHooks`], so a game can layer its own depth model (e.g. grouped occluders) and asset
//! source (e.g. an embed) on top without this crate ever depending on the game. [`Files`] is a ready
//! hook for tools that read maps straight off disk.
//!
//! Tile-layer cells are merged into meshes so a whole layer is a handful of entities and draw calls
//! rather than one sprite per cell — the cost of bringing a large map on screen scales with distinct
//! depths and animations, not tile count. Static cells merge by (tileset, depth); animated cells merge
//! by (tileset, depth, animation) and a frame advance just rewrites the shared mesh's UVs. Tile-objects
//! can't be merged, so they stay individual sprites. A merged mesh still sorts against sprites (and the
//! game's own sprites) by its depth, so a caller's grouped-occluder model keeps interleaving with actors.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::sprite::Anchor;

/// Logical pixels per tile, fixed at the art's native size so nearest-neighbour magnification keeps
/// each art-pixel a whole number of screen pixels.
pub const TILE: f32 = 16.0;

/// Tags every sprite a map spawns, so a caller can despawn the lot to swap maps.
#[derive(Component)]
pub struct MapTile;

/// The caller-supplied integration points: this crate draws the map and asks the hook for the bits
/// that are the caller's business. The `tile_z`/`object_z` defaults stack layers by index and y-sort
/// objects just above them — enough for previews; a game overrides them with its own depth model.
pub trait MapHooks {
    /// Resolve a tileset's image to a loaded handle. The caller owns the asset source (filesystem,
    /// an embed, a network reader…); `images` is offered for callers that decode into it directly.
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>>;

    /// Depth of a tile-layer cell. `layer` is the tile layer's index in file order.
    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> f32 {
        let _ = (x, y);
        layer as f32
    }

    /// Depth of a tile-object at its foot. `above` is the number of tile layers drawn before it; `y`
    /// is the foot in pixels (downward); `map_height` is the map height in tiles.
    fn object_z(&mut self, above: usize, x: f32, y: f32, map_height: f32) -> f32 {
        let _ = x;
        above as f32 + (y / TILE + 1.0) / (map_height + 2.0)
    }
}

/// Draws every tile-layer cell and tile-object of `map`, taking image handles and depths from `hooks`.
/// Tile-layer cells are merged into [`MapTile`] meshes — static cells one mesh per (tileset, depth),
/// animated cells one mesh per (tileset, depth, animation) carrying an [`Animated`] component that
/// [`TileAnimationPlugin`] drives by rewriting the shared UVs. Tile-objects, which can't be merged,
/// stay individual [`MapTile`] sprites. `origin` translates everything: the map is laid out in Tiled's
/// corner-origin pixel space, so a caller passes the screen position of that origin ([`Vec2::ZERO`]
/// draws raw).
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
                        let Some(sheet) = hooks.image(tileset, images) else {
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
                    let Some(sheet) = hooks.image(tileset, images) else {
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
                ..default()
            })),
            Transform::from_xyz(0.0, 0.0, z),
            animation,
        ));
    }
}

/// Accumulates the quads of every static tile that shares one tileset and depth into a single mesh, so
/// a layer becomes one draw call. Tile centers are baked into the vertices; the mesh entity carries the
/// shared depth as its `z`, so it sorts against sprites exactly as the tiles would have individually.
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

/// Like [`Batch`], but for tiles sharing one *animation* (so one frame timeline drives them all). The
/// geometry is fixed; advancing a frame just rewrites the shared mesh's UVs — a handful of meshes a few
/// times a second, rather than one sprite per cell touched every frame.
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
        let uvs = frame_uvs(self.frames[0].0, self.atlas, &self.flips);
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(self.indices));
        let animated = Animated {
            frames: self.frames,
            total,
            atlas: self.atlas,
            flips: self.flips,
            current: 0,
        };
        (mesh, animated)
    }
}

/// Drives one merged animated tile mesh: the shared frame timeline plus what's needed to rewrite its
/// UVs when the frame advances.
#[derive(Component)]
struct Animated {
    frames: Vec<(Rect, f32)>,
    total: f32,
    atlas: Vec2,
    flips: Vec<(bool, bool)>,
    current: usize,
}

/// An individually-spawned animated tile — a tile-object, which (unlike a tile-layer cell) can't be
/// merged, so it animates by swapping its own sprite's atlas rect.
#[derive(Component)]
struct AnimatedSprite {
    frames: Vec<(Rect, f32)>,
    total: f32,
}

/// Advances animated tiles through their frames. Add it wherever a map is shown.
pub struct TileAnimationPlugin;

impl Plugin for TileAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (animate, animate_sprites));
    }
}

/// The tile-object counterpart to [`animate`]: points each animated object sprite at its current frame.
fn animate_sprites(time: Res<Time>, mut sprites: Query<(&AnimatedSprite, &mut Sprite)>) {
    let now = time.elapsed_secs();
    for (anim, mut sprite) in &mut sprites {
        if anim.total > 0.0 {
            sprite.rect = Some(anim.frames[frame_at(&anim.frames, anim.total, now)].0);
        }
    }
}

/// Repaints each merged animated mesh's UVs onto the current frame, but only when the frame index
/// actually changes — so a still-running animation costs one modulo and a comparison per mesh.
fn animate(
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tiles: Query<(&mut Animated, &Mesh2d)>,
) {
    let now = time.elapsed_secs();
    for (mut anim, mesh) in &mut tiles {
        if anim.total <= 0.0 {
            continue;
        }
        let frame = frame_at(&anim.frames, anim.total, now);
        if frame == anim.current {
            continue;
        }
        anim.current = frame;
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            let uvs = frame_uvs(anim.frames[frame].0, anim.atlas, &anim.flips);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        }
    }
}

/// The frame showing at `now` for a looping timeline of `(region, duration)`s totalling `total`.
fn frame_at(frames: &[(Rect, f32)], total: f32, now: f32) -> usize {
    let mut remaining = now % total;
    for (index, &(_, duration)) in frames.iter().enumerate() {
        if remaining < duration {
            return index;
        }
        remaining -= duration;
    }
    frames.len() - 1
}

/// Every cell's UVs for one frame's atlas region, each with its own flip — the layout [`AnimBatch`]
/// builds and [`animate`] rewrites.
fn frame_uvs(region: Rect, atlas: Vec2, flips: &[(bool, bool)]) -> Vec<[f32; 2]> {
    flips
        .iter()
        .flat_map(|&(flip_x, flip_y)| quad_uvs(region, atlas, flip_x, flip_y))
        .collect()
}

/// A tile quad's four corners (top-left, top-right, bottom-right, bottom-left) centred on `center`.
fn quad_positions(center: Vec2, size: Vec2) -> [[f32; 3]; 4] {
    let half = size / 2.0;
    [
        [center.x - half.x, center.y + half.y, 0.0],
        [center.x + half.x, center.y + half.y, 0.0],
        [center.x + half.x, center.y - half.y, 0.0],
        [center.x - half.x, center.y - half.y, 0.0],
    ]
}

/// A tile quad's four UVs into the atlas region, in the same corner order as [`quad_positions`].
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

/// The tileset sheet's pixel dimensions, for normalizing tile regions to UVs. Falls back to the grid
/// extent when a tileset declares no image (a collection of images, which this renderer doesn't merge).
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

/// A [`MapHooks`] for off-disk tools: decodes each tileset image from its file with the `image` crate
/// (so absolute or relative paths both work) and uses the default layer/y-sort depth. Caches by
/// tileset so each sheet decodes once.
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
