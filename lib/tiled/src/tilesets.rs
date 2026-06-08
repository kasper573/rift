use std::sync::OnceLock;

use image::{Image, Region, decode_png};
use math::{Pixels, Pos, Size};

use crate::{AnimFrame, FLIP_H, FLIP_V, Prop, Tileset, gid_id, load_tileset};

/// A map's tilesets, resolved and queryable by raw cell value — flip flags, gid bases,
/// atlas layout, and tile animation stay internal. Images decode lazily on first
/// [`Tilesets::resolve`], so non-rendering consumers never pay for pixels.
#[derive(Clone)]
pub struct Tilesets {
    sets: Vec<Entry>,
}

impl Tilesets {
    /// Loads every tileset the map references; `fetch` resolves a tileset source or image
    /// path to its bytes.
    pub fn load(
        map: &crate::Map,
        fetch: impl Fn(&str) -> Option<&'static [u8]>,
    ) -> Result<Tilesets, String> {
        let mut sets = Vec::new();
        for reference in &map.tilesets {
            let bytes = fetch(&reference.source)
                .ok_or_else(|| format!("unknown tileset source {}", reference.source))?;
            let json = std::str::from_utf8(bytes)
                .map_err(|_| format!("tileset {} is not utf-8", reference.source))?;
            let tileset = load_tileset(json)?;
            let png = fetch(&tileset.image)
                .ok_or_else(|| format!("unknown tileset image {}", tileset.image))?;
            sets.push(Entry {
                first_gid: reference.first_gid,
                tileset,
                png,
                image: OnceLock::new(),
            });
        }
        sets.sort_by_key(|entry| entry.first_gid);
        Ok(Tilesets { sets })
    }

    /// The named custom property of the cell's tile, if any.
    pub fn property(&self, raw: u32, name: &str) -> Option<&Prop> {
        self.tile(raw)?.properties.get(name)
    }

    /// The cell's tile entry, if the tileset declares one (properties or animation).
    pub fn tile(&self, raw: u32) -> Option<&crate::TilesetTile> {
        let (entry, local) = self.entry(raw)?;
        entry.tileset.tiles.get(local)
    }

    /// The image and pixel region of the cell's tile at time `t` (animated tiles advance),
    /// plus the cell's flip flags. Decodes the tileset image on first use.
    pub fn resolve(&self, raw: u32, time: f32) -> Option<(&Image, Region, (bool, bool))> {
        let flip = (raw & FLIP_H != 0, raw & FLIP_V != 0);
        let (entry, mut local) = self.entry(raw)?;
        let tileset = &entry.tileset;
        if let Some(tile) = tileset.tiles.get(local) {
            local = tile_frame(local, &tile.animation, time);
        }
        let columns = tileset.columns.max(1);
        let region = Region::new(
            Pos::new(
                Pixels(((local % columns) * tileset.tile_width) as f32),
                Pixels(((local / columns) * tileset.tile_height) as f32),
            ),
            Size::new(
                Pixels(tileset.tile_width as f32),
                Pixels(tileset.tile_height as f32),
            ),
        );
        let image = entry.image.get_or_init(|| decode_png(entry.png));
        Some((image, region, flip))
    }

    fn entry(&self, raw: u32) -> Option<(&Entry, u32)> {
        let gid = gid_id(raw);
        if gid == 0 {
            return None;
        }
        let entry = self
            .sets
            .iter()
            .rev()
            .find(|entry| entry.first_gid <= gid)?;
        Some((entry, gid - entry.first_gid))
    }
}

#[derive(Clone)]
struct Entry {
    first_gid: u32,
    tileset: Tileset,
    png: &'static [u8],
    image: OnceLock<Image>,
}

fn tile_frame(base: u32, animation: &[AnimFrame], t: f32) -> u32 {
    let total: u32 = animation.iter().map(|frame| frame.duration_ms).sum();
    if total == 0 {
        return base;
    }
    let mut remaining = (t * 1000.0).max(0.0) as u32 % total;
    for frame in animation {
        if remaining < frame.duration_ms {
            return frame.tile_id;
        }
        remaining -= frame.duration_ms;
    }
    base
}
