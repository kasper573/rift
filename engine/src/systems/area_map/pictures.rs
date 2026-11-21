//! The pictures of the ground the maps show: each square of an area as its designer baked it
//! (see [`super::bake`]) into the area's bundle, which the maps only read. The squares come in
//! levels as a slippy map's do: each level up covers twice as far each way in the same pixels.
//! Until a square's picture is read, the map shows what it holds of another level, or else the
//! bare ground's colour, and the square's picture fades in over it once it is.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{IoTaskPool, Task};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use image::{RgbaImage, imageops};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui;

use super::baked::{self, MapPictureFile};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::math::{Pos, Rect};
use crate::core::units::{MeterSize, Meters};
use crate::systems::area::{self, Area, AreaDef, AreaId};

/// How many pixels a picture of a square is across, and down.
pub const MAP_PICTURE_PIXELS: u32 = 512;

/// How many pixels of a picture of the sharpest level a metre spans.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(transparent)]
pub struct MapDetail(pub f32);

/// A square of an area's ground as the map shows it from above: its level, and which square of
/// that level, counted from the ground's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapTile {
    pub level: u8,
    pub column: u32,
    pub row: u32,
}

impl MapTile {
    /// The square's name in its folder of pictures, `<level>_<column>_<row>`.
    pub fn key(self) -> String {
        format!("{}_{}_{}", self.level, self.column, self.row)
    }

    /// The square named `key`, written as [`MapTile::key`] writes it.
    pub fn from_key(key: &str) -> Option<MapTile> {
        let mut numbers = key.split('_');
        let tile = MapTile {
            level: numbers.next()?.parse().ok()?,
            column: numbers.next()?.parse().ok()?,
            row: numbers.next()?.parse().ok()?,
        };
        (tile.key() == key).then_some(tile)
    }

    /// How far a square of `level` reaches each way.
    pub fn side(level: u8, detail: MapDetail) -> Meters {
        Meters(MAP_PICTURE_PIXELS as f32 / detail.0 * f32::from(1u16 << level.min(15)))
    }

    pub fn rect(self, detail: MapDetail) -> Rect<Meters> {
        let side = MapTile::side(self.level, detail).0;
        Rect::new(
            Pos::new(self.column as f32 * side, self.row as f32 * side),
            euclid::Size2D::new(side, side),
        )
    }

    /// The squares of `level` reaching into `rect`, as far as `bounds` holds ground.
    pub fn within(
        level: u8,
        rect: Rect<Meters>,
        bounds: Rect<Meters>,
        detail: MapDetail,
    ) -> Vec<MapTile> {
        let Some(held) = rect.intersection(&bounds) else {
            return Vec::new();
        };
        let side = MapTile::side(level, detail).0;
        let first = |at: f32| (at / side).floor().max(0.0) as u32;
        let last = |at: f32| ((at / side).ceil().max(1.0) as u32) - 1;
        let (columns, rows) = (
            first(held.min_x())..=last(held.max_x()),
            first(held.min_y())..=last(held.max_y()),
        );
        rows.flat_map(|row| {
            columns
                .clone()
                .map(move |column| MapTile { level, column, row })
        })
        .collect()
    }

    /// The coarsest level whose pixels are still no larger than the screen's at
    /// `pixels_per_meter`, so the picture never shows blurrier than the screen could; no higher
    /// than `most`.
    pub fn level_for(detail: MapDetail, pixels_per_meter: f32, most: u8) -> u8 {
        let level = (detail.0 / pixels_per_meter.max(f32::EPSILON))
            .log2()
            .floor();
        level.clamp(0.0, f32::from(most)) as u8
    }
}

/// Which map wants a square, most pressing first: what the minimap shows, then what the Map tab
/// shows, then the squares of the highest level over the whole area, which show wherever a finer
/// square is still on its way, then the ring just past each map, which a walk or a drag shows
/// next, then what the Map tab shows at each of its zoom steps when it opens. Last come the
/// pictures of another level kept under squares whose own picture hasn't faded in yet, so a
/// change of zoom never shows the bare ground through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Wanted {
    Minimap,
    Map,
    Floor,
    NextToMinimap,
    NextToMap,
    Preload,
    Underneath,
}

impl Wanted {
    fn shown(self) -> bool {
        matches!(self, Wanted::Minimap | Wanted::Map)
    }

    /// Whether a map waits for the square before it counts as ready.
    fn awaited(self) -> bool {
        self != Wanted::Underneath
    }
}

/// A picture of a square ready to draw: the ground it covers and when it came, to fade in from.
pub(super) struct MapPicture {
    pub texture: egui::TextureId,
    pub rect: Rect<Meters>,
    came: f64,
    image: Handle<Image>,
}

impl MapPicture {
    /// How far the picture has faded in over what was there before by `now`, 0 to 1.
    pub fn shown(&self, now: f64) -> f32 {
        ((now - self.came) / FADE_SECS).clamp(0.0, 1.0) as f32
    }
}

/// How many of the squares the maps show have their picture, and how many squares asked for
/// ahead of being shown are still on their way, for a driver of the client.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapTileCounts {
    pub wanted: u32,
    pub shown: u32,
    pub ahead: u32,
}

/// The pictures of what the maps show, those on their way from the area's baked map, and the
/// squares it holds no picture of.
#[derive(Resource, Default)]
pub struct MapPictures {
    content: Option<Content>,
    asked: HashMap<Place, Wanted>,
    pictures: HashMap<Place, MapPicture>,
    reading: HashMap<Place, Task<Option<Mipmapped>>>,
    absent: HashSet<Place>,
    told: MapTileCounts,
    /// The areas the maps asked squares of on the last frame, and whether each holds all of them.
    ready: HashMap<AreaId, bool>,
}

impl MapPictures {
    /// The pictures of `area`, coarsest level first, so the finer ones are drawn over them.
    pub(super) fn pictures(&self, area: AreaId) -> Vec<&MapPicture> {
        let mut pictures: Vec<(&MapTile, &MapPicture)> = self
            .pictures
            .iter()
            .filter(|((of, _), _)| *of == area)
            .map(|((_, tile), picture)| (tile, picture))
            .collect();
        pictures.sort_by_key(|(tile, _)| std::cmp::Reverse(tile.level));
        pictures.into_iter().map(|(_, picture)| picture).collect()
    }

    pub fn tiles(&self) -> MapTileCounts {
        self.told
    }

    /// Whether the maps asked for squares of `area` on the last frame, and hold every one of them
    /// its baked map has a picture of.
    pub fn ready_for(&self, area: AreaId) -> bool {
        self.ready.get(&area).copied().unwrap_or(false)
    }
}

/// Asks for the squares of the highest level over the whole of `area`: they stand under whatever
/// a map shows of it, and are what the full map shows at its widest.
pub(super) fn want_whole(world: &mut World, area: AreaId) {
    let content = world.resource::<Content>().clone();
    let Some(loaded) = area::loaded(&content, area) else {
        return;
    };
    let detail = detail(world);
    let most = most_level(loaded, detail);
    let bounds = loaded.size.bounds();
    let mut map = world.resource_mut::<MapPictures>();
    for tile in MapTile::within(most, bounds, bounds, detail) {
        let rank = map.asked.entry((area, tile)).or_insert(Wanted::Floor);
        *rank = (*rank).min(Wanted::Floor);
    }
}

/// Asks for the pictures of `shown` of `area`, which a map draws at `pixels_per_meter`,
/// and, for a map on screen, of the ring of squares around it.
pub(super) fn want(
    world: &mut World,
    area: AreaId,
    shown: Rect<Meters>,
    pixels_per_meter: f32,
    wanted: Wanted,
) {
    let content = world.resource::<Content>().clone();
    let Some(loaded) = area::loaded(&content, area) else {
        return;
    };
    let detail = detail(world);
    let most = most_level(loaded, detail);
    let level = MapTile::level_for(detail, pixels_per_meter, most);
    let side = MapTile::side(level, detail).0;
    let bounds = loaded.size.bounds();
    let ring = match wanted {
        Wanted::Minimap => Some(Wanted::NextToMinimap),
        Wanted::Map => Some(Wanted::NextToMap),
        _ => None,
    };
    let now = gui::context(world).map_or(0.0, |gui| gui.input(|input| input.time));
    let mut map = world.resource_mut::<MapPictures>();
    let asks = ring
        .map(|ring| (level, shown.inflate(side, side), ring))
        .into_iter()
        .chain([(level, shown, wanted), (most, shown, Wanted::Floor)]);
    for (level, rect, asked) in asks {
        for tile in MapTile::within(level, rect, bounds, detail) {
            let rank = map.asked.entry((area, tile)).or_insert(asked);
            *rank = (*rank).min(asked);
        }
    }
    if !wanted.shown() {
        return;
    }
    let unshown: Vec<Rect<Meters>> = MapTile::within(level, shown, bounds, detail)
        .into_iter()
        .filter(|tile| {
            map.pictures
                .get(&(area, *tile))
                .is_none_or(|picture| picture.shown(now) < 1.0)
        })
        .map(|tile| tile.rect(detail))
        .collect();
    let underneath: Vec<Place> = map
        .pictures
        .iter()
        .filter(|((of, tile), picture)| {
            *of == area
                && tile.level != level
                && unshown.iter().any(|rect| rect.intersects(&picture.rect))
        })
        .map(|(place, _)| *place)
        .collect();
    for place in underneath {
        map.asked.entry(place).or_insert(Wanted::Underneath);
    }
}

/// Keeps what the maps asked for this frame: reads what the baked maps hold of it, as many squares
/// at a time as there are threads to spare, and lets go of what no map shows any more.
pub(super) fn keep(world: &mut World) {
    let content = world.resource::<Content>().clone();
    if world
        .resource::<MapPictures>()
        .content
        .as_ref()
        .is_none_or(|kept| !kept.is(&content))
    {
        forget(world);
        world.resource_mut::<MapPictures>().content = Some(content.clone());
    }
    let asked = std::mem::take(&mut world.resource_mut::<MapPictures>().asked);
    let Some(now) = gui::context(world).map(|gui| gui.input(|input| input.time)) else {
        return;
    };
    let mut released = Vec::new();
    {
        let mut map = world.resource_mut::<MapPictures>();
        let had = |map: &MapPictures, place: &Place| {
            map.pictures.contains_key(place) || map.absent.contains(place)
        };
        map.told = MapTileCounts {
            wanted: asked.values().filter(|wanted| wanted.shown()).count() as u32,
            shown: asked
                .iter()
                .filter(|(place, wanted)| wanted.shown() && map.pictures.contains_key(place))
                .count() as u32,
            ahead: asked
                .iter()
                .filter(|(place, wanted)| {
                    matches!(wanted, Wanted::Floor | Wanted::Preload) && !had(&map, place)
                })
                .count() as u32,
        };
        let mut ready = HashMap::new();
        for (place, wanted) in &asked {
            let held = !wanted.awaited() || had(&map, place);
            *ready.entry(place.0).or_insert(true) &= held;
        }
        map.ready = ready;
        map.pictures.retain(|place, picture| {
            let kept = asked.contains_key(place);
            if !kept {
                released.push(picture.image.clone());
            }
            kept
        });
        map.reading.retain(|place, _| asked.contains_key(place));
    }
    for image in released {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&image);
    }
    let mut order: Vec<(Place, Wanted)> = asked.into_iter().collect();
    order.sort_by_key(|&(place, wanted)| (wanted, place.1));
    start_reading(world, &content, &order);
    finish_reading(world, now);
}

/// Lets go of every picture and what is on its way.
pub(super) fn forget(world: &mut World) {
    let mut map = std::mem::take(&mut *world.resource_mut::<MapPictures>());
    for (_, picture) in map.pictures.drain() {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&picture.image);
    }
}

/// The detail the maps of the game in `world` draw their pictures in.
pub(super) fn detail(world: &World) -> MapDetail {
    detail_of(world.resource::<Content>())
}

/// The detail the maps of `content` draw their pictures in.
pub(super) fn detail_of(content: &Content) -> MapDetail {
    content.built::<crate::systems::GameSettings>().map_detail
}

/// The highest level a square of `area` is drawn at: the first whose one square covers the
/// whole area.
pub(super) fn most_level(area: &Area, detail: MapDetail) -> u8 {
    let longest = area.size.width.max(area.size.height);
    (0..15)
        .find(|&level| MapTile::side(level, detail).0 >= longest)
        .unwrap_or(15)
}

/// The pixels of a picture, row by row from the top, four bytes each, not premultiplied.
pub type Pixels = Vec<u8>;

type Place = (AreaId, MapTile);

/// A picture with its smaller copies, premultiplied, ready to hand the renderer.
struct Mipmapped {
    data: Vec<u8>,
    levels: u32,
}

/// How long a picture takes to fade in over what was there before.
const FADE_SECS: f64 = 0.25;

/// Reads the pictures the baked maps hold of the squares asked for, as many at a time as there
/// are threads to spare.
fn start_reading(world: &mut World, content: &Content, order: &[(Place, Wanted)]) {
    let threads = IoTaskPool::get().thread_num().max(1);
    let mut map = world.resource_mut::<MapPictures>();
    for (place, _) in order {
        if map.reading.len() >= threads {
            return;
        }
        if map.pictures.contains_key(place)
            || map.reading.contains_key(place)
            || map.absent.contains(place)
        {
            continue;
        }
        let picture = content
            .table::<AreaDef>()
            .get(place.0)
            .and_then(|area| area.map.squares.get(&place.1))
            .map(|square| square.picture.clone());
        map.reading.insert(
            *place,
            IoTaskPool::get().spawn(async move { read(picture?).map(mipmapped) }),
        );
    }
}

/// Takes the pictures read, made ready for the renderer; a square with no picture that reads is
/// noted so it isn't looked for again.
fn finish_reading(world: &mut World, now: f64) {
    let detail = detail(world);
    let mut made = Vec::new();
    {
        let mut map = world.resource_mut::<MapPictures>();
        let mut absent = Vec::new();
        map.reading.retain(|place, task| match check_ready(task) {
            Some(Some(mipmapped)) => {
                made.push((*place, mipmapped));
                false
            }
            Some(None) => {
                absent.push(*place);
                false
            }
            None => true,
        });
        map.absent.extend(absent);
    }
    for (place, made) in made {
        let picture = upload(world, made, place.1.rect(detail), now);
        let replaced = world
            .resource_mut::<MapPictures>()
            .pictures
            .insert(place, picture);
        if let Some(old) = replaced {
            world
                .resource_mut::<EguiUserTextures>()
                .remove_image(&old.image);
        }
    }
}

/// The pixels of `picture`, none where it doesn't read as a picture of a square.
fn read(picture: MapPictureFile) -> Option<Pixels> {
    let pixels = picture.bytes().ok().and_then(|bytes| baked::decode(&bytes));
    if pixels.is_none() {
        warn!("Couldn't read the map picture {picture:?}");
    }
    pixels
}

/// `pixels` with every smaller copy down to a pixel, premultiplied as egui blends them.
fn mipmapped(mut pixels: Pixels) -> Mipmapped {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let [red, green, blue, alpha] = *pixel;
        *pixel = egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha).to_array();
    }
    let levels = MAP_PICTURE_PIXELS.ilog2() + 1;
    let Some(picture) = RgbaImage::from_raw(MAP_PICTURE_PIXELS, MAP_PICTURE_PIXELS, pixels) else {
        return Mipmapped {
            data: Vec::new(),
            levels: 0,
        };
    };
    let data = (0..levels)
        .scan(picture, |level, _| {
            let bytes = level.as_raw().clone();
            *level = imageops::thumbnail(
                level,
                (level.width() / 2).max(1),
                (level.height() / 2).max(1),
            );
            Some(bytes)
        })
        .flatten()
        .collect();
    Mipmapped { data, levels }
}

fn upload(world: &mut World, made: Mipmapped, rect: Rect<Meters>, now: f64) -> MapPicture {
    let mut image = Image::new_uninit(
        Extent3d {
            width: MAP_PICTURE_PIXELS,
            height: MAP_PICTURE_PIXELS,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = made.levels.max(1);
    image.data = Some(made.data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(image.clone()));
    MapPicture {
        texture,
        rect,
        came: now,
        image,
    }
}

pub(super) fn init(app: &mut App) {
    app.init_resource::<MapPictures>();
}
