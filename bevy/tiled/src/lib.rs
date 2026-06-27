use std::collections::HashMap;

use bevy::asset::{RenderAssetUsages, load_internal_asset, uuid_handle};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite::Anchor;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

const TILEMAP_SHADER: Handle<Shader> = uuid_handle!("9d3c6e1a-4b2f-4a8c-9e7d-1f2a3b4c5d6e");

pub const TILE: f32 = 16.0;

#[derive(Component)]
pub struct MapTile;

pub trait MapHooks {
    fn image(
        &mut self,
        tileset: &tiled::Tileset,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>>;

    fn tile_z(&mut self, layer: usize, x: i32, y: i32) -> Option<f32> {
        let _ = (layer, x, y);
        None
    }

    fn object_z(&mut self, above: usize, x: f32, y: f32, map_height: f32) -> f32 {
        let _ = x;
        above as f32 + (y / TILE + 1.0) / (map_height + 2.0)
    }
}
pub fn spawn_map(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    tilemaps: &mut Assets<TilemapMaterial>,
    map: &tiled::Map,
    hooks: &mut impl MapHooks,
    origin: Vec2,
) {
    let map_w = map.width as i32;
    let map_h = map.height as i32;
    let map_height = map.height as f32;
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let mut sheets: HashMap<usize, Option<Handle<Image>>> = HashMap::new();
    let mut layer = 0;
    for tiled_layer in map.layers() {
        match tiled_layer.layer_type() {
            tiled::LayerType::Tiles(tiles) => {
                let mut flats: HashMap<usize, IndexBuilder> = HashMap::new();
                for y in 0..map_h {
                    for x in 0..map_w {
                        let Some(tile) = tiles.get_tile(x, y) else {
                            continue;
                        };
                        let tileset = tile.get_tileset();
                        let Some(sheet) = resolve_sheet(&mut sheets, hooks, images, tileset) else {
                            continue;
                        };
                        match hooks.tile_z(layer, x, y) {
                            None => flats
                                .entry(tileset_key(tileset))
                                .or_insert_with(|| IndexBuilder::new(map_w, map_h, sheet, tileset))
                                .set(x, y, tile.id(), tile.flip_h, tile.flip_v),
                            Some(z) => {
                                let center = Vec2::new(
                                    origin.x + (x as f32 + 0.5) * TILE,
                                    origin.y - (y as f32 + 0.5) * TILE,
                                );
                                spawn(
                                    commands,
                                    sheet,
                                    frames(tileset, tile.id()),
                                    Vec2::splat(TILE),
                                    Transform::from_xyz(center.x, center.y, z),
                                    tile.flip_h,
                                    tile.flip_v,
                                    None,
                                );
                            }
                        }
                    }
                }
                for builder in flats.into_values() {
                    spawn_tilemap(
                        commands,
                        images,
                        &quad,
                        tilemaps,
                        builder,
                        origin,
                        layer as f32,
                    );
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
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct TilemapMaterial {
    #[texture(0, sample_type = "u_int")]
    index: Handle<Image>,
    #[texture(1)]
    #[sampler(2)]
    atlas: Handle<Image>,
    #[texture(3, sample_type = "u_int")]
    frame_map: Handle<Image>,
    #[uniform(4)]
    data: Tilemap,
}

impl Material2d for TilemapMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(TILEMAP_SHADER)
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[derive(Clone, ShaderType)]
struct Tilemap {
    grid: Vec4,
    sheet: Vec4,
    params: Vec4,
}

#[derive(Component)]
struct TilemapAnim {
    frame_map: Handle<Image>,
    anims: Vec<Anim>,
}

struct Anim {
    tile_id: u32,
    frames: Vec<(u32, f32)>,
    total: f32,
    current: usize,
}

struct IndexBuilder {
    sheet: Handle<Image>,
    width: i32,
    height: i32,
    data: Vec<u8>,
    cols: u32,
    atlas: Vec2,
    tile: Vec2,
    margin: f32,
    spacing: f32,
    tilecount: u32,
    anims: Vec<Anim>,
}

impl IndexBuilder {
    fn new(
        width: i32,
        height: i32,
        sheet: Handle<Image>,
        tileset: &tiled::Tileset,
    ) -> IndexBuilder {
        IndexBuilder {
            sheet,
            width,
            height,
            data: vec![0u8; (width * height * 4) as usize],
            cols: tileset.columns.max(1),
            atlas: atlas_size(tileset),
            tile: Vec2::new(tileset.tile_width as f32, tileset.tile_height as f32),
            margin: tileset.margin as f32,
            spacing: tileset.spacing as f32,
            tilecount: tileset.tilecount,
            anims: tileset_anims(tileset),
        }
    }

    fn set(&mut self, x: i32, y: i32, id: u32, flip_x: bool, flip_y: bool) {
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i] = (id & 0xff) as u8;
        self.data[i + 1] = (id >> 8) as u8;
        self.data[i + 2] = u8::from(flip_x) | (u8::from(flip_y) << 1);
        self.data[i + 3] = 255;
    }

    fn uniform(&self) -> Tilemap {
        Tilemap {
            grid: Vec4::new(self.width as f32, self.height as f32, self.cols as f32, 0.0),
            sheet: Vec4::new(self.atlas.x, self.atlas.y, self.tile.x, self.tile.y),
            params: Vec4::new(self.margin, self.spacing, 0.0, 0.0),
        }
    }

    fn frame_map_data(&self) -> Vec<u8> {
        let mut data = vec![0u8; (self.tilecount.max(1) * 4) as usize];
        for id in 0..self.tilecount {
            let i = (id * 4) as usize;
            data[i] = (id & 0xff) as u8;
            data[i + 1] = (id >> 8) as u8;
        }
        data
    }
}

fn spawn_tilemap(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    quad: &Handle<Mesh>,
    tilemaps: &mut Assets<TilemapMaterial>,
    builder: IndexBuilder,
    origin: Vec2,
    z: f32,
) {
    let width_px = builder.width as f32 * TILE;
    let height_px = builder.height as f32 * TILE;
    let uniform = builder.uniform();
    let frame_map = images.add(uint_image(
        builder.tilecount.max(1),
        1,
        builder.frame_map_data(),
        RenderAssetUsages::default(),
    ));
    let index = images.add(uint_image(
        builder.width as u32,
        builder.height as u32,
        builder.data,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let material = tilemaps.add(TilemapMaterial {
        index,
        atlas: builder.sheet,
        frame_map: frame_map.clone(),
        data: uniform,
    });
    commands.spawn((
        MapTile,
        Mesh2d(quad.clone()),
        MeshMaterial2d(material),
        Transform {
            translation: Vec3::new(origin.x + width_px / 2.0, origin.y - height_px / 2.0, z),
            scale: Vec3::new(width_px, height_px, 1.0),
            ..default()
        },
        TilemapAnim {
            frame_map,
            anims: builder.anims,
        },
    ));
}

#[derive(Component)]
struct AnimatedSprite {
    frames: Vec<(Rect, f32)>,
    total: f32,
}

pub struct TileAnimationPlugin;

impl Plugin for TileAnimationPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, TILEMAP_SHADER, "tilemap.wgsl", Shader::from_wgsl);
        app.add_plugins(Material2dPlugin::<TilemapMaterial>::default())
            .add_systems(Update, (animate_sprites, animate_tilemaps));
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

fn animate_tilemaps(
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TilemapMaterial>>,
    mut maps: Query<(&mut TilemapAnim, &MeshMaterial2d<TilemapMaterial>)>,
) {
    let now = time.elapsed_secs();
    for (mut map, material) in &mut maps {
        let mut dirty = false;
        for anim in &mut map.anims {
            if anim.total <= 0.0 {
                continue;
            }
            let frame = frame_at(&anim.frames, anim.total, now);
            if frame != anim.current {
                anim.current = frame;
                dirty = true;
            }
        }
        if !dirty {
            continue;
        }
        if let Some(mut image) = images.get_mut(&map.frame_map)
            && let Some(data) = image.data.as_mut()
        {
            for anim in &map.anims {
                let atlas_id = anim.frames[anim.current].0;
                let i = anim.tile_id as usize * 4;
                data[i] = (atlas_id & 0xff) as u8;
                data[i + 1] = (atlas_id >> 8) as u8;
            }
        }
        materials.get_mut(&material.0);
    }
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

fn tileset_key(tileset: &tiled::Tileset) -> usize {
    tileset as *const tiled::Tileset as usize
}

fn resolve_sheet(
    sheets: &mut HashMap<usize, Option<Handle<Image>>>,
    hooks: &mut impl MapHooks,
    images: &mut Assets<Image>,
    tileset: &tiled::Tileset,
) -> Option<Handle<Image>> {
    let key = tileset_key(tileset);
    if let Some(cached) = sheets.get(&key) {
        return cached.clone();
    }
    let resolved = hooks.image(tileset, images);
    sheets.insert(key, resolved.clone());
    resolved
}

fn tileset_anims(tileset: &tiled::Tileset) -> Vec<Anim> {
    (0..tileset.tilecount)
        .filter_map(|id| {
            let tile = tileset.get_tile(id)?;
            let animation = tile.animation.as_ref()?;
            if animation.len() <= 1 {
                return None;
            }
            let frames: Vec<(u32, f32)> = animation
                .iter()
                .map(|frame| (frame.tile_id, frame.duration as f32 / 1000.0))
                .collect();
            let total = frames.iter().map(|&(_, duration)| duration).sum();
            Some(Anim {
                tile_id: id,
                frames,
                total,
                current: 0,
            })
        })
        .collect()
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

fn uint_image(width: u32, height: u32, data: Vec<u8>, usage: RenderAssetUsages) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Uint,
        usage,
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image.sampler = ImageSampler::nearest();
    image
}

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
        let key = tileset_key(tileset);
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
