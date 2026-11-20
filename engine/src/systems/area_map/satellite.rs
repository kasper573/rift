//! The ground the maps show as a satellite sees it: each square of an area drawn once by the
//! world's own renderer from straight above, in a daylight of its own, and kept under the profile
//! until what it was drawn from changes. The squares come in levels as a slippy map's do: each
//! level up covers twice as far each way in the same pixels. Until a square's picture is there,
//! the map shows its painted picture, and the satellite picture fades in over it once it is.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget, ScalingMode};
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::light::AmbientLight;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use image::{RgbaImage, imageops};
use ui::egui;

use crate::core::content::{Content, FileHashes, version};
use crate::core::gui;
use crate::core::math::{Pos, Rect};
use crate::core::os::Profile;
use crate::core::render::camera::{self as world_camera, Lens};
use crate::core::render::world::{DRAWN_FRAMES_AFTER, RenderProgress, lights};
use crate::core::render::{Look, video::VideoPreferences};
use crate::core::units::{Degrees, MeterSize, Meters};
use crate::systems::area::scenery::{self, Eyes, Scenery, Sight};
use crate::systems::area::{self, Area, AreaId};

/// How many pixels a satellite picture is across, and down.
pub const SATELLITE_PIXELS: u32 = 512;

/// How many pixels of a level-0 satellite picture a metre spans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SatelliteDensity(pub f32);

impl SatelliteDensity {
    /// The density for maps whose closest reach is `closest`, with a minimap's ground `ground`
    /// points across: the power of two nearest to the pixels that ground spans at twice the
    /// interface's scale, over the metres the reach spans across.
    pub fn of(closest: Meters, ground: f32) -> SatelliteDensity {
        let wanted = (2.0 * 2.0 * ground) / (2.0 * closest.0).max(f32::EPSILON);
        SatelliteDensity(2f32.powf(wanted.log2().round()))
    }
}

/// A square of an area's ground as the map shows it from above: its level, and which square of
/// that level, counted from the ground's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SatelliteTile {
    pub level: u8,
    pub column: u32,
    pub row: u32,
}

impl SatelliteTile {
    /// How far a square of `level` reaches each way.
    pub fn side(level: u8, density: SatelliteDensity) -> Meters {
        Meters(SATELLITE_PIXELS as f32 / density.0 * f32::from(1u16 << level.min(15)))
    }

    pub fn rect(self, density: SatelliteDensity) -> Rect<Meters> {
        let side = SatelliteTile::side(self.level, density).0;
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
        density: SatelliteDensity,
    ) -> Vec<SatelliteTile> {
        let Some(held) = rect.intersection(&bounds) else {
            return Vec::new();
        };
        let side = SatelliteTile::side(level, density).0;
        let first = |at: f32| (at / side).floor().max(0.0) as u32;
        let last = |at: f32| ((at / side).ceil().max(1.0) as u32) - 1;
        let (columns, rows) = (
            first(held.min_x())..=last(held.max_x()),
            first(held.min_y())..=last(held.max_y()),
        );
        rows.flat_map(|row| {
            columns
                .clone()
                .map(move |column| SatelliteTile { level, column, row })
        })
        .collect()
    }

    /// The coarsest level whose pixels are still no larger than the screen's at
    /// `pixels_per_meter`, so the picture never shows blurrier than the screen could; no higher
    /// than `most`.
    pub fn level_for(density: SatelliteDensity, pixels_per_meter: f32, most: u8) -> u8 {
        let level = (density.0 / pixels_per_meter.max(f32::EPSILON))
            .log2()
            .floor();
        level.clamp(0.0, f32::from(most)) as u8
    }
}

/// What a satellite picture was made from: the ground under it, the props reaching over it, the
/// look it is drawn in, its square and density and the engine build. A picture kept with
/// another key is drawn again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SatelliteKey(pub [u8; 32]);

impl SatelliteKey {
    /// The key of `tile` of `area` as `content` holds it.
    pub fn of(
        content: &Content,
        area: &Area,
        tile: SatelliteTile,
        density: SatelliteDensity,
    ) -> SatelliteKey {
        let rect = tile.rect(density);
        let hashes = &content.built::<FileHashes>().0;
        let hash_of = |path: Option<&str>| {
            path.and_then(|path| hashes.get(path))
                .map_or("", |hash| hash.hash.as_str())
                .to_owned()
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(version::build().as_bytes());
        hasher.update(&[tile.level]);
        hasher.update(&tile.column.to_le_bytes());
        hasher.update(&tile.row.to_le_bytes());
        hasher.update(&density.0.to_le_bytes());
        hasher.update(&SATELLITE_PIXELS.to_le_bytes());
        hasher.update(&SUN_ELEVATION.0.to_le_bytes());
        hasher.update(&SUN_AZIMUTH.0.to_le_bytes());
        for terrain in &area.terrains {
            hasher.update(hash_of(Some(terrain.path(content))).as_bytes());
        }
        for at in area.ground_size().tiles_within(rect) {
            for (stored, encoded) in [
                (area.heights.stored_at(at), area.heights.encode_tile(at)),
                (area.paint.stored_at(at), area.paint.encode_tile(at)),
                (area.water.stored_at(at), area.water.encode_tile(at)),
            ] {
                match stored.and_then(|path| hashes.get(path)) {
                    Some(hash) => hasher.update(hash.hash.as_bytes()),
                    None => hasher
                        .update(blake3::hash(encoded.as_deref().unwrap_or_default()).as_bytes()),
                };
            }
        }
        let reached = rect.inflate(OVERHANG.0, OVERHANG.0);
        for placed in area
            .props
            .iter()
            .filter(|placed| reached.contains(placed.at))
        {
            let def = placed.prop.get(content);
            hasher.update(hash_of(Some(placed.prop.path(content))).as_bytes());
            hasher.update(hash_of(Some(def.model.as_str())).as_bytes());
            for material in def.materials.values() {
                hasher.update(hash_of(Some(material.path(content))).as_bytes());
            }
            for value in [
                placed.at.x,
                placed.at.y,
                placed.yaw.0,
                placed.scale,
                placed.sunk.0,
            ] {
                hasher.update(&value.to_le_bytes());
            }
        }
        SatelliteKey(*hasher.finalize().as_bytes())
    }

    fn hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

/// Which map wants a square, most pressing first: what the minimap shows, then what the Map tab
/// shows, then the ring just past each, which a walk or a drag shows next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Wanted {
    Minimap,
    Map,
    NextToMinimap,
    NextToMap,
}

impl Wanted {
    fn shown(self) -> bool {
        matches!(self, Wanted::Minimap | Wanted::Map)
    }
}

/// A satellite picture ready to draw: the ground it covers and when it came, to fade in from.
pub(super) struct SatellitePicture {
    pub texture: egui::TextureId,
    pub rect: Rect<Meters>,
    came: f64,
    image: Handle<Image>,
}

impl SatellitePicture {
    /// How far the picture has faded in over the painted one by `now`, 0 to 1.
    pub fn shown(&self, now: f64) -> f32 {
        ((now - self.came) / FADE_SECS).clamp(0.0, 1.0) as f32
    }
}

/// How many of the squares the maps show have their satellite picture, for a driver of the client.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SatelliteTiles {
    pub wanted: u32,
    pub satellite: u32,
}

/// The satellite pictures of what the maps show, those on their way from the disk or the
/// renderer, and the one being drawn.
#[derive(Resource, Default)]
pub struct SatelliteMap {
    content: Option<Content>,
    asked: HashMap<Place, Wanted>,
    pictures: HashMap<Place, SatellitePicture>,
    keys: HashMap<Place, Keyed>,
    reading: HashMap<Place, Task<Option<Pixels>>>,
    uploading: HashMap<Place, Task<Mipmapped>>,
    unbaked: HashSet<Place>,
    baking: Option<Baking>,
    stand: Option<Stand>,
    told: SatelliteTiles,
}

impl SatelliteMap {
    pub(super) fn pictures(&self, area: AreaId) -> impl Iterator<Item = &SatellitePicture> {
        self.pictures
            .iter()
            .filter(move |((of, _), _)| *of == area)
            .map(|(_, picture)| picture)
    }

    pub fn tiles(&self) -> SatelliteTiles {
        self.told
    }
}

/// Asks for the satellite pictures of `shown` of `area`, which a map draws at `pixels_per_meter`,
/// and of the ring of squares around it.
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
    let density = density(world);
    let horizon = world.resource::<VideoPreferences>().view_distance.horizon();
    let most = most_level(loaded, density, horizon);
    let level = SatelliteTile::level_for(density, pixels_per_meter, most);
    let side = SatelliteTile::side(level, density).0;
    let bounds = loaded.size.bounds();
    let ring = match wanted {
        Wanted::Minimap => Wanted::NextToMinimap,
        _ => Wanted::NextToMap,
    };
    let mut map = world.resource_mut::<SatelliteMap>();
    for (rect, asked) in [(shown.inflate(side, side), ring), (shown, wanted)] {
        for tile in SatelliteTile::within(level, rect, bounds, density) {
            let rank = map.asked.entry((area, tile)).or_insert(asked);
            *rank = (*rank).min(asked);
        }
    }
}

/// Keeps what the maps asked for this frame: reads what the disk holds, draws one square at a
/// time that it doesn't, and lets go of what no map shows any more.
pub(super) fn keep(world: &mut World) {
    let content = world.resource::<Content>().clone();
    if world
        .resource::<SatelliteMap>()
        .content
        .as_ref()
        .is_none_or(|kept| !kept.is(&content))
    {
        forget(world);
        world.resource_mut::<SatelliteMap>().content = Some(content.clone());
    }
    let asked = std::mem::take(&mut world.resource_mut::<SatelliteMap>().asked);
    let Some(now) = gui::context(world).map(|gui| gui.input(|input| input.time)) else {
        return;
    };
    let mut released = Vec::new();
    {
        let mut map = world.resource_mut::<SatelliteMap>();
        let told = SatelliteTiles {
            wanted: asked.values().filter(|wanted| wanted.shown()).count() as u32,
            satellite: asked
                .iter()
                .filter(|(place, wanted)| wanted.shown() && map.pictures.contains_key(place))
                .count() as u32,
        };
        map.told = told;
        map.pictures.retain(|place, picture| {
            let kept = asked.contains_key(place);
            if !kept {
                released.push(picture.image.clone());
            }
            kept
        });
        map.keys.retain(|place, _| asked.contains_key(place));
        map.reading.retain(|place, _| asked.contains_key(place));
        map.uploading.retain(|place, _| asked.contains_key(place));
        map.unbaked.retain(|place| asked.contains_key(place));
    }
    for image in released {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&image);
    }
    let mut order: Vec<(Place, Wanted)> = asked.into_iter().collect();
    order.sort_by_key(|&(place, wanted)| (wanted, place.1));
    let density = density(world);
    let folder = folder_under(world.resource::<Profile>());
    start_reading(world, &content, &order, density, &folder);
    finish_reading(world, now);
    bake(world, &content, &order, density, &folder);
}

/// Lets go of every picture and what is on its way, and of the square being drawn.
pub(super) fn forget(world: &mut World) {
    let mut map = std::mem::take(&mut *world.resource_mut::<SatelliteMap>());
    for (_, picture) in map.pictures.drain() {
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&picture.image);
    }
    if let Some(stand) = map.stand.take() {
        stand.despawn(world);
    }
}

type Place = (AreaId, SatelliteTile);

/// A square's key, while it is worked out off the frame and once it is.
enum Keyed {
    Making(Task<SatelliteKey>),
    Made(SatelliteKey),
}

/// The pixels of a picture, row by row from the top, four bytes each, not premultiplied.
type Pixels = Vec<u8>;

/// A picture with its smaller copies, premultiplied, ready to hand the renderer.
struct Mipmapped {
    data: Vec<u8>,
    levels: u32,
}

/// The scenery of the area being drawn from above, its camera and its sun, kept from square to
/// square while they are of the same area.
struct Stand {
    area: AreaId,
    scenery: Option<Scenery>,
    camera: Entity,
    target: Handle<Image>,
    layer: RenderLayers,
}

impl Stand {
    fn despawn(self, world: &mut World) {
        if let Some(scenery) = self.scenery {
            scenery::despawn(world, scenery);
        }
        world.despawn(self.camera);
    }
}

/// The square being drawn: since how many frames, and the read of its pixels once asked.
struct Baking {
    place: Place,
    key: SatelliteKey,
    frames: u32,
    readback: Option<Entity>,
}

/// The pixels the renderer read back of the square being drawn.
#[derive(Resource, Clone, Default)]
struct ReadPixels(Arc<Mutex<Option<ReadBack>>>);

/// The pixels the read `readback` brought back.
struct ReadBack {
    readback: Entity,
    pixels: Pixels,
}

/// The render layer the scenery drawn from above stands on, apart from every other.
const LAYER: usize = 399;
/// How long a picture takes to fade in over the painted one.
const FADE_SECS: f64 = 0.25;
/// How far past a square a prop may stand and still reach over it: a prop farther off changes
/// no picture of that square, so its key leaves it out.
const OVERHANG: Meters = Meters(16.0);
/// How far over the highest ground the camera stands, and past the lowest it still sees.
const CLEARANCE: Meters = Meters(60.0);
/// Noon, from the north-west, as the painted map lights its slopes.
const SUN_ELEVATION: Degrees = Degrees(55.0);
const SUN_AZIMUTH: Degrees = Degrees(315.0);
/// How many frames a square that never stands is waited on.
const GIVEN_UP_AFTER: u32 = 900;
/// The folder under the profile the pictures are kept in.
const SATELLITE: &str = "satellite";
const KEY_CHUNK: &str = "satellite-key";

fn density(world: &World) -> SatelliteDensity {
    let closest = world
        .resource::<crate::systems::GameSettings>()
        .map_reaches
        .closest();
    SatelliteDensity::of(closest, super::minimap::GROUND)
}

/// The highest level a square of `area` is drawn at: one that covers the whole area, or the
/// largest whose far corner still lies within the props' `horizon`, which the renderer draws no
/// prop past.
fn most_level(area: &Area, density: SatelliteDensity, horizon: Option<Meters>) -> u8 {
    let longest = area.size.width.max(area.size.height);
    let mut level = 0;
    while level < 15 {
        let side = SatelliteTile::side(level, density).0;
        let past_horizon = horizon.is_some_and(|horizon| {
            let corner = side * std::f32::consts::FRAC_1_SQRT_2;
            corner.hypot(CLEARANCE.0) > horizon.0
        });
        if side >= longest || past_horizon {
            return if past_horizon {
                level.saturating_sub(1)
            } else {
                level
            };
        }
        level += 1;
    }
    level
}

fn folder_under(profile: &Profile) -> PathBuf {
    profile.0.join(SATELLITE)
}

fn file_of(folder: &Path, area: &str, tile: SatelliteTile) -> PathBuf {
    folder
        .join(area)
        .join(tile.level.to_string())
        .join(format!("{}_{}.png", tile.column, tile.row))
}

/// Works out the keys of the squares asked for, and reads the pictures the disk holds of them,
/// as many at a time as there are threads to spare.
fn start_reading(
    world: &mut World,
    content: &Content,
    order: &[(Place, Wanted)],
    density: SatelliteDensity,
    folder: &Path,
) {
    let threads = AsyncComputeTaskPool::get().thread_num().max(1);
    let mut map = world.resource_mut::<SatelliteMap>();
    for (place, _) in order {
        if map.pictures.contains_key(place)
            || map.reading.contains_key(place)
            || map.uploading.contains_key(place)
            || map.unbaked.contains(place)
        {
            continue;
        }
        let making = map
            .keys
            .values()
            .filter(|keyed| matches!(keyed, Keyed::Making(_)))
            .count();
        let keyed = match map.keys.remove(place) {
            Some(Keyed::Making(mut task)) => match check_ready(&mut task) {
                Some(key) => Keyed::Made(key),
                None => Keyed::Making(task),
            },
            Some(made) => made,
            None if making < threads => {
                let Some(area) = area::loaded(content, place.0).cloned() else {
                    continue;
                };
                let (content, tile) = (content.clone(), place.1);
                Keyed::Making(
                    AsyncComputeTaskPool::get()
                        .spawn(async move { SatelliteKey::of(&content, &area, tile, density) }),
                )
            }
            None => continue,
        };
        if let Keyed::Made(key) = keyed {
            let file = file_of(folder, place.0.path(content), place.1);
            map.reading.insert(
                *place,
                IoTaskPool::get().spawn(async move { read(&file, key) }),
            );
        }
        map.keys.insert(*place, keyed);
    }
}

/// Takes the pictures read from the disk, and those made ready for the renderer; a square the
/// disk holds no picture of under its key is left to draw.
fn finish_reading(world: &mut World, now: f64) {
    let density = density(world);
    let mut map = world.resource_mut::<SatelliteMap>();
    let places: Vec<Place> = map.reading.keys().copied().collect();
    for place in places {
        let Some(task) = map.reading.get_mut(&place) else {
            continue;
        };
        let Some(read) = check_ready(task) else {
            continue;
        };
        map.reading.remove(&place);
        match read {
            Some(pixels) => {
                map.uploading.insert(
                    place,
                    AsyncComputeTaskPool::get().spawn(async move { mipmapped(pixels) }),
                );
            }
            None => {
                map.unbaked.insert(place);
            }
        }
    }
    let mut made = Vec::new();
    map.uploading.retain(|place, task| match check_ready(task) {
        Some(mipmapped) => {
            made.push((*place, mipmapped));
            false
        }
        None => true,
    });
    for (place, made) in made {
        let picture = upload(world, made, place.1.rect(density), now);
        let replaced = world
            .resource_mut::<SatelliteMap>()
            .pictures
            .insert(place, picture);
        if let Some(old) = replaced {
            world
                .resource_mut::<EguiUserTextures>()
                .remove_image(&old.image);
        }
    }
}

/// Draws the most pressing square no picture of is kept, one at a time, on the scenery of its
/// area standing on a layer of its own.
fn bake(
    world: &mut World,
    content: &Content,
    order: &[(Place, Wanted)],
    density: SatelliteDensity,
    folder: &Path,
) {
    let next = {
        let map = world.resource::<SatelliteMap>();
        order
            .iter()
            .map(|(place, _)| *place)
            .find(|place| map.unbaked.contains(place))
    };
    let baking = world
        .resource::<SatelliteMap>()
        .baking
        .as_ref()
        .map(|b| b.place);
    if baking.is_some_and(|place| !order.iter().any(|(asked, _)| *asked == place)) {
        world.resource_mut::<SatelliteMap>().baking = None;
    }
    if world.resource::<SatelliteMap>().baking.is_none() {
        let Some(place) = next else {
            if let Some(stand) = world.resource_mut::<SatelliteMap>().stand.take() {
                stand.despawn(world);
            }
            return;
        };
        let key = match world.resource::<SatelliteMap>().keys.get(&place) {
            Some(Keyed::Made(key)) => *key,
            _ => return,
        };
        world.resource_mut::<SatelliteMap>().baking = Some(Baking {
            place,
            key,
            frames: 0,
            readback: None,
        });
        if !stand_over(world, content, place, density) {
            world.resource_mut::<SatelliteMap>().baking = None;
            return;
        }
    }
    let waiting = {
        let map = world.resource::<SatelliteMap>();
        map.stand
            .as_ref()
            .is_some_and(|stand| stand.scenery.is_none())
            .then(|| map.baking.as_ref().map(|baking| baking.place))
            .flatten()
    };
    if let Some(place) = waiting
        && !stand_over(world, content, place, density)
    {
        world.resource_mut::<SatelliteMap>().baking = None;
        return;
    }
    advance(world, content, folder);
}

/// Stands the scenery of the area of `place` from above over its square, its camera framing
/// the square: whether it could.
fn stand_over(
    world: &mut World,
    content: &Content,
    place: Place,
    density: SatelliteDensity,
) -> bool {
    let Some(area) = area::loaded(content, place.0).cloned() else {
        return false;
    };
    let same = world
        .resource::<SatelliteMap>()
        .stand
        .as_ref()
        .is_some_and(|stand| stand.area == place.0);
    if !same {
        if let Some(stand) = world.resource_mut::<SatelliteMap>().stand.take() {
            stand.despawn(world);
        }
        let stand = spawn_stand(world, place.0);
        world.resource_mut::<SatelliteMap>().stand = Some(stand);
    }
    let rect = place.1.rect(density);
    let center = rect.center();
    let range = area.heights.range();
    let over = range.highest.0 + CLEARANCE.0;
    let depth = over - range.lowest.0 + CLEARANCE.0;
    let eye = Vec3::new(center.x, over, center.y);
    let side = rect.width();
    let Some(camera) = world
        .resource::<SatelliteMap>()
        .stand
        .as_ref()
        .map(|stand| stand.camera)
    else {
        return false;
    };
    world.entity_mut(camera).insert((
        Transform::from_translation(eye).looking_at(Vec3::new(center.x, 0.0, center.y), -Vec3::Z),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: side,
                height: side,
            },
            near: 0.0,
            far: depth,
            ..OrthographicProjection::default_3d()
        }),
        world_camera::CameraFocus(Vec3::new(
            center.x,
            area.heights.height_at(center).0,
            center.y,
        )),
    ));
    let ground = area.heights.height_at(center);
    let fov = Degrees((2.0 * (side / 2.0).atan2(over - ground.0)).to_degrees());
    let sight = Sight {
        at: center,
        reach: Meters(side * std::f32::consts::FRAC_1_SQRT_2 + OVERHANG.0),
        horizon: None,
        lens: Lens::new(SATELLITE_PIXELS as f32, fov),
        eyes: Eyes::standing(eye, center, ground),
    };
    let ready = scenery::shaders_ready(world, content, &area);
    let Some(stand) = world
        .resource_mut::<SatelliteMap>()
        .stand
        .as_mut()
        .map(|stand| (stand.scenery, stand.layer.clone()))
    else {
        return false;
    };
    let scenery = match stand.0 {
        Some(scenery) => scenery,
        None if ready => {
            let stood = scenery::spawn(world, content, &area, stand.1);
            for entity in [stood.sky, stood.air, stood.rain].into_iter().flatten() {
                world.entity_mut(entity).insert(Visibility::Hidden);
            }
            if let Some(stand) = world.resource_mut::<SatelliteMap>().stand.as_mut() {
                stand.scenery = Some(stood);
            }
            stood
        }
        None => return true,
    };
    scenery::view_from(world, scenery, sight);
    // Nobody waits on a picture drawn off the screen: arriving in an area, or anything else that
    // waits for the scenery in view to stand, never waits for this one.
    scenery::unseen(world, scenery);
    true
}

fn spawn_stand(world: &mut World, area: AreaId) -> Stand {
    let look = *world.resource::<Look>();
    let layer = RenderLayers::from_layers(&[LAYER]);
    let target = {
        let mut image = Image::new_target_texture(
            SATELLITE_PIXELS,
            SATELLITE_PIXELS,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        world.resource_mut::<Assets<Image>>().add(image)
    };
    let camera = world
        .spawn((
            Camera3d::default(),
            Camera {
                order: ORDER,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            world_camera::plain_picture(look.picture),
            AmbientLight {
                color: Color::WHITE,
                brightness: look.light.ambient_nits,
                ..default()
            },
            layer.clone(),
            Transform::default(),
        ))
        .id();
    let sun = lights::sun(Color::WHITE, look.light.sun_lux, SUN_ELEVATION, SUN_AZIMUTH);
    let sun = world.spawn((sun, layer.clone(), ChildOf(camera))).id();
    if let Some(mut light) = world.get_mut::<DirectionalLight>(sun) {
        light.shadow_maps_enabled = false;
    }
    Stand {
        area,
        scenery: None,
        camera,
        target,
        layer,
    }
}

/// A satellite camera draws before every camera that draws to the window.
const ORDER: isize = -102;

/// Moves the square being drawn on: reads its pixels back once its scenery has stood whole for
/// long enough, and keeps them.
fn advance(world: &mut World, content: &Content, folder: &Path) {
    let Some(stand) = world
        .resource::<SatelliteMap>()
        .stand
        .as_ref()
        .map(|stand| (stand.scenery, stand.target.clone()))
    else {
        return;
    };
    let (Some(scenery), target) = stand else {
        return;
    };
    let compiling = world
        .get_resource::<RenderProgress>()
        .is_some_and(|progress| progress.compiling() > 0);
    let stood = scenery::stood(world, scenery);
    let read = world.resource::<ReadPixels>().clone();
    let Some(baking) = world
        .resource_mut::<SatelliteMap>()
        .baking
        .as_mut()
        .map(|baking| {
            baking.frames += 1;
            (baking.place, baking.key, baking.frames, baking.readback)
        })
    else {
        return;
    };
    let (place, key, frames, readback) = baking;
    if frames > GIVEN_UP_AFTER {
        let mut map = world.resource_mut::<SatelliteMap>();
        map.baking = None;
        map.unbaked.remove(&place);
        return;
    }
    match readback {
        None if stood && !compiling && u64::from(frames) >= DRAWN_FRAMES_AFTER => {
            let reads = read.clone();
            let entity = world
                .spawn(ReadbackOnce::texture(target))
                .observe(move |done: On<ReadbackComplete>, mut commands: Commands| {
                    *reads.0.lock().expect("the read pixels") = Some(ReadBack {
                        readback: done.entity,
                        pixels: done.data.clone(),
                    });
                    commands.entity(done.entity).despawn();
                })
                .id();
            if let Some(baking) = world.resource_mut::<SatelliteMap>().baking.as_mut() {
                baking.readback = Some(entity);
            }
        }
        None => {}
        Some(entity) => {
            let pixels = {
                let mut read = read.0.lock().expect("the read pixels");
                match read.take() {
                    Some(back) if back.readback == entity => Some(back.pixels),
                    other => {
                        *read = other;
                        None
                    }
                }
            };
            let Some(pixels) = pixels else {
                return;
            };
            if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
                if let Some(baking) = world.resource_mut::<SatelliteMap>().baking.as_mut() {
                    baking.readback = None;
                }
                return;
            }
            let mut map = world.resource_mut::<SatelliteMap>();
            map.baking = None;
            map.unbaked.remove(&place);
            let file = file_of(folder, place.0.path(content), place.1);
            let kept = pixels.clone();
            IoTaskPool::get()
                .spawn(async move { write(&file, key, &kept) })
                .detach();
            map.uploading.insert(
                place,
                AsyncComputeTaskPool::get().spawn(async move { mipmapped(pixels) }),
            );
        }
    }
}

/// The picture kept at `file`, if it was made under `key`.
fn read(file: &Path, key: SatelliteKey) -> Option<Pixels> {
    let bytes = std::fs::read(file).ok()?;
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().ok()?;
    let made_under = reader
        .info()
        .uncompressed_latin1_text
        .iter()
        .any(|chunk| chunk.keyword == KEY_CHUNK && chunk.text == key.hex());
    let info = reader.info();
    let fits = info.width == SATELLITE_PIXELS
        && info.height == SATELLITE_PIXELS
        && info.color_type == png::ColorType::Rgba
        && info.bit_depth == png::BitDepth::Eight;
    if !made_under || !fits {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());
    Some(pixels)
}

/// Keeps `pixels` at `file` under `key`, over any picture kept there before.
fn write(file: &Path, key: SatelliteKey, pixels: &[u8]) {
    let written = (|| -> Result<(), Box<dyn std::error::Error>> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, SATELLITE_PIXELS, SATELLITE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_text_chunk(KEY_CHUNK.to_owned(), key.hex())?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixels)?;
        writer.finish()?;
        std::fs::write(file, out)?;
        Ok(())
    })();
    if let Err(error) = written {
        warn!(
            "Couldn't keep the satellite picture {}: {error}",
            file.display()
        );
    }
}

/// `pixels` with every smaller copy down to a pixel, premultiplied as egui blends them.
fn mipmapped(mut pixels: Pixels) -> Mipmapped {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let [red, green, blue, alpha] = *pixel;
        *pixel = egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha).to_array();
    }
    let levels = SATELLITE_PIXELS.ilog2() + 1;
    let Some(picture) = RgbaImage::from_raw(SATELLITE_PIXELS, SATELLITE_PIXELS, pixels) else {
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

fn upload(world: &mut World, made: Mipmapped, rect: Rect<Meters>, now: f64) -> SatellitePicture {
    let mut image = Image::new_uninit(
        Extent3d {
            width: SATELLITE_PIXELS,
            height: SATELLITE_PIXELS,
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
    SatellitePicture {
        texture,
        rect,
        came: now,
        image,
    }
}

pub(super) fn init(app: &mut App) {
    app.init_resource::<SatelliteMap>()
        .init_resource::<ReadPixels>();
}
