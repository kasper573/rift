//! The maps' pictures of an area as its designer last baked them (see [`super::bake`]): a
//! picture of each square of every level, kept in the area's bundle beside the key it was drawn
//! from. The game only ever reads them, and a check of the game tells the squares whose ground,
//! props or style have changed since by their keys.

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::pictures::{MAP_PICTURE_PIXELS, MapTile, Pixels};
use crate::core::content::{ContentProblem, ErrorPlace, FileWriter, PartSpec, Parts, StoredPart};

/// The pictures of an area's map, one file to a square, named by [`MapTile::key`].
pub const MAP_PICTURES: PartSpec = PartSpec::Keyed {
    folder: "map",
    exts: &[PICTURE_EXT],
};
/// The key each picture of [`MAP_PICTURES`] was drawn from.
pub const MAP_KEYS: PartSpec = PartSpec::File {
    name: "map",
    exts: &["json"],
};

/// An area's map as its designer last baked it: none of its squares where it never was.
#[derive(Clone, Debug, Default)]
pub struct BakedMap {
    pub squares: BTreeMap<MapTile, BakedMapSquare>,
}

/// A square of a baked map: its picture, and the key it was drawn from, none where the bundle
/// keeps no key for it.
#[derive(Clone, Debug)]
pub struct BakedMapSquare {
    pub picture: MapPictureFile,
    pub key: Option<MapTileKey>,
}

/// What a picture of a square was drawn from: the files of everything it shows, its square, the
/// map detail and the style the maps are drawn in. A picture kept with another key is out of
/// date.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct MapTileKey(pub String);

/// A picture's file: as the bundle keeps it, or as a bake drew it.
#[derive(Clone, Debug)]
pub enum MapPictureFile {
    Stored(StoredPart),
    Drawn(Arc<[u8]>),
}

impl MapPictureFile {
    pub fn bytes(&self) -> io::Result<Arc<[u8]>> {
        match self {
            MapPictureFile::Stored(stored) => stored.read().map(Arc::from),
            MapPictureFile::Drawn(bytes) => Ok(bytes.clone()),
        }
    }
}

impl BakedMap {
    pub fn is_empty(&self) -> bool {
        self.squares.is_empty()
    }
}

/// The path from the game folder of the picture of `tile` in the bundle at `bundle`.
pub fn picture_file(bundle: &str, tile: MapTile) -> String {
    format!("{bundle}/{}", MAP_PICTURES.named(&tile.key(), PICTURE_EXT))
}

/// The path from the game folder of the keys of the map of the bundle at `bundle`.
pub fn keys_file(bundle: &str) -> String {
    format!("{bundle}/{}", MAP_KEYS.named("", "json"))
}

/// The map the bundle keeps: its pictures left stored, each with its key, refusing a picture
/// named for no square.
pub(crate) fn read(parts: &mut Parts) -> BakedMap {
    let mut squares = BTreeMap::new();
    for (key, file) in parts.keyed(MAP_PICTURES) {
        let Some(tile) = MapTile::from_key(key) else {
            parts.error(
                file,
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "not a square of the map",
                    key,
                    "<level>_<column>_<row>: bake the maps again",
                ),
            );
            continue;
        };
        squares.insert(
            tile,
            BakedMapSquare {
                picture: MapPictureFile::Stored(parts.stored(file)),
                key: None,
            },
        );
    }
    if let Some(file) = parts.file(MAP_KEYS)
        && let Some(MapKeys(keys)) = parts.json::<MapKeys>(file, |_| {})
    {
        for (name, key) in keys {
            if let Some(square) = MapTile::from_key(&name).and_then(|tile| squares.get_mut(&tile)) {
                square.key = Some(key);
            }
        }
    }
    BakedMap { squares }
}

/// Writes `map` into the bundle at `bundle`: each picture, and the keys of all of them.
pub(crate) fn write(map: &BakedMap, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
    if map.is_empty() {
        return Ok(());
    }
    for (tile, square) in &map.squares {
        let file = picture_file(bundle, *tile);
        if writer.wants(&file) {
            writer.file(&file, &square.picture.bytes()?)?;
        }
    }
    let keys = MapKeys(
        map.squares
            .iter()
            .filter_map(|(tile, square)| Some((tile.key(), square.key.clone()?)))
            .collect(),
    );
    writer.json(&keys_file(bundle), &keys)
}

/// `pixels`, a picture of a square, as its file keeps it.
pub(super) fn encode(pixels: &[u8]) -> Arc<[u8]> {
    let rgb: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[red, green, blue, _]| [*red, *green, *blue])
        .collect();
    let side = MAP_PICTURE_PIXELS;
    Arc::from(&*webp::Encoder::from_rgb(&rgb, side, side).encode(PICTURE_QUALITY))
}

/// The pixels of the picture of a square its file `bytes` keeps, every one opaque; none where
/// they aren't one.
pub(super) fn decode(bytes: &[u8]) -> Option<Pixels> {
    let picture = webp::Decoder::new(bytes).decode()?;
    let side = MAP_PICTURE_PIXELS;
    if picture.width() != side || picture.height() != side {
        return None;
    }
    let channels = if picture.is_alpha() { 4 } else { 3 };
    Some(
        picture
            .chunks_exact(channels)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], u8::MAX])
            .collect(),
    )
}

const PICTURE_EXT: &str = "webp";
/// How finely a picture is kept, on WebP's scale to 100: past this, a picture takes more room
/// without looking any different.
const PICTURE_QUALITY: f32 = 80.0;

/// The keys of a map's pictures, by the name of each one's square.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct MapKeys(BTreeMap<String, MapTileKey>);
