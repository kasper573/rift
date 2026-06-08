use std::collections::HashMap;
use std::io::Read;

use serde::{Deserialize, Deserializer};

mod b64;
mod tilesets;

pub use math::{Pixels, Pos, Size, Tiles};
pub use tilesets::Tilesets;

pub const FLIP_H: u32 = 0x8000_0000;
pub const FLIP_V: u32 = 0x4000_0000;
pub const FLIP_FLAGS: u32 = 0xE000_0000;

pub fn gid_id(raw: u32) -> u32 {
    raw & !FLIP_FLAGS
}

pub fn load_map(input: &str) -> Result<Map, String> {
    serde_json::from_str(input).map_err(|error| error.to_string())
}

pub fn load_tileset(input: &str) -> Result<Tileset, String> {
    serde_json::from_str(input).map_err(|error| error.to_string())
}

/// Custom properties by name.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(from = "Vec<PropRow>")]
pub struct Props(HashMap<String, Prop>);

impl Props {
    pub fn get(&self, name: &str) -> Option<&Prop> {
        self.0.get(name)
    }
}

/// A Tiled property's value; the JSON value's own type selects the variant, so the
/// rows' declared `"type"` field is redundant and ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Prop {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl Prop {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Map {
    pub width: Tiles,
    pub height: Tiles,
    #[serde(rename = "tilewidth")]
    pub tile_width: Pixels,
    #[serde(rename = "tileheight")]
    pub tile_height: Pixels,
    #[serde(deserialize_with = "layers")]
    pub layers: Vec<Layer>,
    pub tilesets: Vec<TilesetRef>,
    #[serde(default)]
    pub properties: Props,
}

impl Map {
    pub fn tile_layers(&self) -> impl Iterator<Item = &TileLayer> {
        self.layers.iter().filter_map(|layer| match layer {
            Layer::Tiles(tiles) => Some(tiles),
            Layer::Objects(_) => None,
        })
    }

    pub fn tile_layer(&self, name: &str) -> Option<&TileLayer> {
        self.layers.iter().find_map(|layer| match layer {
            Layer::Tiles(tiles) if tiles.name.eq_ignore_ascii_case(name) => Some(tiles),
            _ => None,
        })
    }

    pub fn objects(&self) -> Vec<&Object> {
        self.layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Objects(group) => Some(group.objects.iter()),
                Layer::Tiles(_) => None,
            })
            .flatten()
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TilesetRef {
    #[serde(rename = "firstgid")]
    pub first_gid: u32,
    pub source: String,
}

#[derive(Clone, Debug)]
pub enum Layer {
    Tiles(TileLayer),
    Objects(ObjectLayer),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "RawTileLayer")]
pub struct TileLayer {
    pub name: String,
    pub width: u32,
    pub height: u32,

    data: Vec<u32>,
}

impl TileLayer {
    /// The raw cell value at (x, y); empty (0) outside the layer.
    pub fn at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.data[(y as u32 * self.width + x as u32) as usize]
    }

    /// Every cell as (x, y, raw value).
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32, u32)> + '_ {
        self.data.iter().enumerate().map(|(index, &raw)| {
            (
                (index as u32 % self.width) as i32,
                (index as u32 / self.width) as i32,
                raw,
            )
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ObjectLayer {
    pub name: String,
    pub objects: Vec<Object>,
}

/// A Tiled object. Tiled writes `x`/`y`/`width`/`height` as flat keys; [`RawObject`] mirrors that
/// schema and `From` folds them into a [`Pos`]/[`Size`].
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "RawObject")]
pub struct Object {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub gid: u32,
    pub pos: Pos<Pixels>,
    pub size: Size<Pixels>,
    pub point: bool,
    pub polyline: Vec<Pos<Pixels>>,
    pub properties: Props,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RawObject {
    id: u32,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    gid: u32,
    x: Pixels,
    y: Pixels,
    width: Pixels,
    height: Pixels,
    point: bool,
    polyline: Vec<Pos<Pixels>>,
    properties: Props,
}

impl From<RawObject> for Object {
    fn from(raw: RawObject) -> Object {
        Object {
            id: raw.id,
            name: raw.name,
            kind: raw.kind,
            gid: raw.gid,
            pos: Pos::new(raw.x, raw.y),
            size: Size::new(raw.width, raw.height),
            point: raw.point,
            polyline: raw.polyline,
            properties: raw.properties,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tileset {
    pub name: String,
    pub columns: u32,
    #[serde(rename = "tilecount")]
    pub tile_count: u32,
    #[serde(rename = "tilewidth")]
    pub tile_width: u32,
    #[serde(rename = "tileheight")]
    pub tile_height: u32,
    pub image: String,
    #[serde(rename = "imagewidth")]
    pub image_width: u32,
    #[serde(rename = "imageheight")]
    pub image_height: u32,
    pub tiles: TileDefs,
}

/// The tiles a tileset declares anything about (properties, animation), by local id.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(from = "Vec<TilesetTile>")]
pub struct TileDefs(HashMap<u32, TilesetTile>);

impl TileDefs {
    pub fn get(&self, id: u32) -> Option<&TilesetTile> {
        self.0.get(&id)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TilesetTile {
    pub id: u32,
    pub properties: Props,
    pub animation: Vec<AnimFrame>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct AnimFrame {
    #[serde(rename = "tileid")]
    pub tile_id: u32,
    #[serde(rename = "duration")]
    pub duration_ms: u32,
}

#[derive(Deserialize)]
struct PropRow {
    name: String,
    value: Prop,
}

impl From<Vec<PropRow>> for Props {
    fn from(rows: Vec<PropRow>) -> Props {
        Props(rows.into_iter().map(|row| (row.name, row.value)).collect())
    }
}

impl From<Vec<TilesetTile>> for TileDefs {
    fn from(rows: Vec<TilesetTile>) -> TileDefs {
        TileDefs(rows.into_iter().map(|tile| (tile.id, tile)).collect())
    }
}

#[derive(Deserialize)]
struct RawTileLayer {
    name: String,
    width: u32,
    height: u32,
    data: Data,
    #[serde(default)]
    compression: String,
}

/// Tiled writes tile data as a plain array (csv) or a base64 string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Data {
    Cells(Vec<u32>),
    Encoded(String),
}

impl TryFrom<RawTileLayer> for TileLayer {
    type Error = String;

    fn try_from(raw: RawTileLayer) -> Result<TileLayer, String> {
        let data = match raw.data {
            Data::Cells(cells) => cells,
            Data::Encoded(encoded) => decode_tile_data(&encoded, &raw.compression)?,
        };
        Ok(TileLayer {
            name: raw.name,
            width: raw.width,
            height: raw.height,
            data,
        })
    }
}

/// Keeps only the layer kinds we model, skipping Tiled's others (image, group, ...).
fn layers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Layer>, D::Error> {
    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "lowercase")]
    enum Raw {
        Tilelayer(TileLayer),
        Objectgroup(ObjectLayer),
        #[serde(other)]
        Other,
    }

    Ok(Vec::<Raw>::deserialize(deserializer)?
        .into_iter()
        .filter_map(|raw| match raw {
            Raw::Tilelayer(tiles) => Some(Layer::Tiles(tiles)),
            Raw::Objectgroup(objects) => Some(Layer::Objects(objects)),
            Raw::Other => None,
        })
        .collect())
}

fn decode_tile_data(encoded: &str, compression: &str) -> Result<Vec<u32>, String> {
    let raw = b64::decode(encoded)?;
    let bytes = match compression {
        "" | "none" => raw,
        "zlib" => inflate(flate2::read::ZlibDecoder::new(raw.as_slice()))?,
        "gzip" => inflate(flate2::read::GzDecoder::new(raw.as_slice()))?,
        "deflate" => inflate(flate2::read::DeflateDecoder::new(raw.as_slice()))?,
        other => return Err(format!("unsupported tile-layer compression '{other}'")),
    };
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn inflate(mut decoder: impl Read) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|error| format!("decompression failed: {error}"))?;
    Ok(out)
}
