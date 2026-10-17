use std::collections::{BTreeMap, BTreeSet, HashMap};

use engine::systems::area::grid::{self, PaletteRow};
use image::{GenericImage, RgbaImage};
use serde_json::{Value, json};

use super::formats::{self, AnimationExport};
use crate::ui::widgets;

pub const MAX_TILE_PX: u32 = 512;
pub const MAX_COLORS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGridSettings {
    pub tile: u32,
    pub margin: u32,
    pub spacing: u32,
    pub extruded: u32,
}

pub struct TileGridCut {
    pub columns: u32,
    pub rows: u32,
    pub drawn: usize,
    pub empty: usize,
    pub image: RgbaImage,
}

pub fn cut_grid(image: &RgbaImage, settings: TileGridSettings) -> Result<TileGridCut, String> {
    let TileGridSettings {
        tile,
        margin,
        spacing,
        extruded,
    } = settings;
    if tile == 0 || tile > MAX_TILE_PX {
        return Err(format!("a tile is 1 to {MAX_TILE_PX} px"));
    }
    let cell = tile + 2 * extruded;
    let count = |length: u32| -> Option<u32> {
        let inner = length.checked_sub(2 * margin)?;
        if inner < cell {
            return None;
        }
        let rest = inner - cell;
        (rest % (cell + spacing) == 0).then(|| 1 + rest / (cell + spacing))
    };
    let (Some(columns), Some(rows)) = (count(image.width()), count(image.height())) else {
        return Err(format!(
            "{} × {} is not whole {tile} × {tile} tiles with a {margin} px margin and {spacing} px spacing",
            image.width(),
            image.height()
        ));
    };
    let mut packed = RgbaImage::new(columns * tile, rows * tile);
    let mut drawn = 0;
    let mut empty = 0;
    for row in 0..rows {
        for column in 0..columns {
            let x = margin + column * (cell + spacing) + extruded;
            let y = margin + row * (cell + spacing) + extruded;
            let cut = image::imageops::crop_imm(image, x, y, tile, tile).to_image();
            if formats::is_blank(&cut) {
                empty += 1;
            } else {
                drawn += 1;
            }
            packed
                .copy_from(&cut, column * tile, row * tile)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(TileGridCut {
        columns,
        rows,
        drawn,
        empty,
        image: packed,
    })
}

pub fn tile_image(image: &RgbaImage, tile: u32, index: u32) -> Option<RgbaImage> {
    let columns = image.width() / tile.max(1);
    if columns == 0 {
        return None;
    }
    let x = (index % columns) * tile;
    let y = (index / columns) * tile;
    if x + tile > image.width() || y + tile > image.height() {
        return None;
    }
    Some(image::imageops::crop_imm(image, x, y, tile, tile).to_image())
}

pub fn tile_hashes(image: &RgbaImage, tile: u32) -> Vec<(String, bool)> {
    let tile = tile.max(1);
    let columns = image.width() / tile;
    let rows = image.height() / tile;
    (0..columns * rows)
        .filter_map(|index| tile_image(image, tile, index))
        .map(|cut| (formats::pixel_hash(&cut), formats::is_blank(&cut)))
        .collect()
}

#[derive(Clone, Debug)]
pub struct PaletteUse {
    pub area: String,
    pub entries: usize,
}

#[derive(Clone, Debug)]
pub struct PaletteRewrite {
    pub area: String,
    pub entries: usize,
    pub first: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub struct TilePlan {
    pub tile: u32,
    pub old_columns: u32,
    pub new_columns: u32,
    pub old_rows: u32,
    pub new_rows: u32,
    pub matches: BTreeMap<u32, u32>,
    pub found: usize,
    pub renumbered: usize,
    pub new_tiles: usize,
    pub gone: Vec<u32>,
    pub gone_used: BTreeMap<u32, Vec<PaletteUse>>,
    pub replacements: BTreeMap<u32, Option<u32>>,
    pub rewrites: Vec<PaletteRewrite>,
    pub example: Option<(u32, u32)>,
    pub blank_data: usize,
    pub walk: usize,
    pub surfaces: usize,
    pub animations: usize,
}

impl TilePlan {
    pub fn ready(&self) -> Result<(), String> {
        let waiting: Vec<String> = self
            .gone_used
            .keys()
            .filter(|tile| !matches!(self.replacements.get(tile), Some(Some(_))))
            .map(|tile| tile.to_string())
            .collect();
        if waiting.is_empty() {
            return Ok(());
        }
        Err(format!(
            "tile{} {} {} gone but still painted; pick what replaces {}",
            widgets::plural(waiting.len()),
            waiting.join(", "),
            if waiting.len() == 1 { "is" } else { "are" },
            if waiting.len() == 1 { "it" } else { "them" }
        ))
    }

    pub fn target(&self, old: u32) -> Option<u32> {
        self.matches
            .get(&old)
            .copied()
            .or_else(|| self.replacements.get(&old).copied().flatten())
    }

    pub fn new_count(&self) -> u32 {
        self.new_columns * self.new_rows
    }
}

pub fn tile_plan(
    old: &RgbaImage,
    row: &Value,
    new: &RgbaImage,
    key: &str,
    palettes: &[(String, Vec<PaletteRow>)],
) -> TilePlan {
    let tile = row["tile"].as_u64().unwrap_or(16).max(1) as u32;
    let old_hashes = tile_hashes(old, tile);
    let new_hashes = tile_hashes(new, tile);
    let old_columns = old.width() / tile;
    let new_columns = new.width() / tile;
    let mut by_hash: HashMap<&str, Vec<u32>> = HashMap::new();
    for (index, (hash, blank)) in new_hashes.iter().enumerate() {
        if !blank {
            by_hash.entry(hash.as_str()).or_default().push(index as u32);
        }
    }
    let mut used: BTreeSet<u32> = BTreeSet::new();
    let mut matches = BTreeMap::new();
    let mut gone = Vec::new();
    let mut pending: Vec<(u32, &str)> = Vec::new();
    for (index, (hash, blank)) in old_hashes.iter().enumerate() {
        if *blank {
            continue;
        }
        let index = index as u32;
        let (column, row_index) = (index % old_columns.max(1), index / old_columns.max(1));
        let same_place = (column < new_columns)
            .then(|| row_index * new_columns + column)
            .filter(|candidate| {
                by_hash
                    .get(hash.as_str())
                    .is_some_and(|list| list.contains(candidate))
                    && !used.contains(candidate)
            });
        match same_place {
            Some(candidate) => {
                used.insert(candidate);
                matches.insert(index, candidate);
            }
            None => pending.push((index, hash.as_str())),
        }
    }
    for (index, hash) in pending {
        let lowest = by_hash
            .get(hash)
            .and_then(|list| list.iter().find(|candidate| !used.contains(candidate)))
            .copied();
        match lowest {
            Some(candidate) => {
                used.insert(candidate);
                matches.insert(index, candidate);
            }
            None => gone.push(index),
        }
    }
    let new_tiles = new_hashes
        .iter()
        .enumerate()
        .filter(|(index, (_, blank))| !blank && !used.contains(&(*index as u32)))
        .count();
    let renumbered = matches.iter().filter(|(old, new)| old != new).count();
    let data_tiles: Vec<u32> = row["tiles"]
        .as_object()
        .map(|tiles| tiles.keys().filter_map(|key| key.parse().ok()).collect())
        .unwrap_or_default();
    let mut walk = 0;
    let mut surfaces = 0;
    let mut animations = 0;
    let blank_data = data_tiles
        .iter()
        .filter(|index| {
            old_hashes
                .get(**index as usize)
                .is_some_and(|(_, blank)| *blank)
        })
        .count();
    for index in &data_tiles {
        if !matches.contains_key(index) {
            continue;
        }
        let data = &row["tiles"][index.to_string()];
        walk += usize::from(data.get("walk").is_some());
        surfaces += usize::from(data.get("surface").is_some());
        animations += usize::from(data["animation"].as_array().is_some_and(|a| !a.is_empty()));
    }
    let mut gone_used: BTreeMap<u32, Vec<PaletteUse>> = BTreeMap::new();
    let mut rewrites = Vec::new();
    for (area, rows) in palettes {
        let mut entries = 0;
        let mut first = None;
        let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
        for row in rows.iter().filter(|row| row.tileset == key) {
            match matches.get(&row.tile) {
                Some(new_tile) if *new_tile != row.tile => {
                    entries += 1;
                    first.get_or_insert((row.tile, *new_tile));
                }
                Some(_) => {}
                None => {
                    entries += 1;
                    *uses.entry(row.tile).or_default() += 1;
                }
            }
        }
        for (tile, count) in uses {
            gone_used.entry(tile).or_default().push(PaletteUse {
                area: area.clone(),
                entries: count,
            });
        }
        if entries > 0 {
            rewrites.push(PaletteRewrite {
                area: area.clone(),
                entries,
                first,
            });
        }
    }
    let example = rewrites
        .iter()
        .find_map(|rewrite| rewrite.first)
        .or_else(|| {
            matches
                .iter()
                .find(|(old, new)| old != new)
                .map(|(o, n)| (*o, *n))
        });
    TilePlan {
        tile,
        old_columns,
        new_columns,
        old_rows: old.height() / tile,
        new_rows: new.height() / tile,
        found: matches.len(),
        matches,
        renumbered,
        new_tiles,
        gone,
        gone_used,
        replacements: BTreeMap::new(),
        rewrites,
        example,
        blank_data,
        walk,
        surfaces,
        animations,
    }
}

pub fn apply_tile_plan(
    plan: &TilePlan,
    row: &Value,
    key: &str,
    palettes: &[(String, Vec<PaletteRow>)],
) -> Result<(Value, Vec<(String, String)>), String> {
    plan.ready()?;
    let mut row = row.clone();
    let mut tiles = serde_json::Map::new();
    if let Some(old_tiles) = row["tiles"].as_object() {
        let mut keyed: Vec<(u32, Value)> = old_tiles
            .iter()
            .filter_map(|(index, data)| Some((index.parse::<u32>().ok()?, data.clone())))
            .collect();
        keyed.sort_by_key(|(index, _)| *index);
        let mut moved: BTreeMap<u32, Value> = BTreeMap::new();
        for (index, mut data) in keyed {
            let Some(target) = plan.target(index) else {
                continue;
            };
            if let Some(frames) = data.get_mut("animation").and_then(Value::as_array_mut) {
                let mut kept = Vec::new();
                for frame in frames.iter() {
                    let old_tile = frame["tile"].as_u64().unwrap_or(0) as u32;
                    if let Some(new_tile) = plan.target(old_tile) {
                        let mut frame = frame.clone();
                        frame["tile"] = json!(new_tile);
                        kept.push(frame);
                    }
                }
                if kept.len() < 2 {
                    kept.clear();
                }
                *frames = kept;
                if frames.is_empty() {
                    data.as_object_mut().map(|data| data.remove("animation"));
                }
            }
            if data.as_object().is_some_and(|data| !data.is_empty())
                && moved.insert(target, data).is_some()
            {
                return Err(format!(
                    "two painted tiles would both become tile {target}; pick distinct replacements"
                ));
            }
        }
        for (index, data) in moved {
            tiles.insert(index.to_string(), data);
        }
    }
    row["tiles"] = Value::Object(tiles);
    let mut rewritten = Vec::new();
    for (area, rows) in palettes {
        let mut changed = false;
        let moved: Vec<PaletteRow> = rows
            .iter()
            .map(|row| {
                let target = (row.tileset == key)
                    .then(|| plan.target(row.tile))
                    .flatten()
                    .unwrap_or(row.tile);
                changed |= target != row.tile;
                PaletteRow {
                    tile: target,
                    ..row.clone()
                }
            })
            .collect();
        if changed {
            rewritten.push((area.clone(), grid::write_palette(&moved)));
        }
    }
    Ok((row, rewritten))
}

#[derive(Clone, Debug)]
pub enum AnimFrameRef {
    Block,
    Existing(u32),
    New(usize),
}

#[derive(Clone, Debug)]
pub struct AnimCell {
    pub column: u32,
    pub row: u32,
    pub tile: u32,
    pub redrawn: bool,
    pub frames: Vec<AnimFrameRef>,
}

#[derive(Clone, Debug)]
pub struct AnimPlan {
    pub block: (u32, u32),
    pub size: (u32, u32),
    pub cells: Vec<AnimCell>,
    pub still: usize,
    pub reused: Vec<u32>,
    pub new_images: Vec<RgbaImage>,
    pub lengths: Vec<u32>,
    pub warnings: Vec<String>,
    pub fits: Option<String>,
}

impl AnimPlan {
    pub fn animated(&self) -> usize {
        self.cells.len()
    }
}

pub fn anim_plan(
    tileset: &RgbaImage,
    row: &Value,
    export: &AnimationExport,
    kind_limits: bool,
    block: (u32, u32),
) -> Result<AnimPlan, String> {
    let tile = row["tile"].as_u64().unwrap_or(16).max(1) as u32;
    let strip = export.strips.first().ok_or("the export has no frames")?;
    if strip.frames.len() < 2 {
        return Err("an animation has at least two frames".to_owned());
    }
    let (fw, fh) = export.frame;
    if fw % tile != 0 || fh % tile != 0 {
        return Err(format!(
            "{fw} × {fh} frames are not whole {tile} × {tile} tiles"
        ));
    }
    let size = (fw / tile, fh / tile);
    let columns = tileset.width() / tile;
    let rows = tileset.height() / tile;
    if block.0 + size.0 > columns || block.1 + size.1 > rows {
        return Err(format!(
            "a {} × {} block at column {}, row {} runs off the {columns} × {rows} tileset",
            size.0, size.1, block.0, block.1
        ));
    }
    let hashes = tile_hashes(tileset, tile);
    let mut by_hash: HashMap<&str, u32> = HashMap::new();
    for (index, (hash, blank)) in hashes.iter().enumerate().rev() {
        if !blank {
            by_hash.insert(hash.as_str(), index as u32);
        }
    }
    let mut cells = Vec::new();
    let mut still = 0;
    let mut reused = BTreeSet::new();
    let mut new_images: Vec<RgbaImage> = Vec::new();
    let mut new_hashes: Vec<String> = Vec::new();
    for y in 0..size.1 {
        for x in 0..size.0 {
            let cuts: Vec<RgbaImage> = strip
                .frames
                .iter()
                .map(|frame| {
                    image::imageops::crop_imm(&frame.image, x * tile, y * tile, tile, tile)
                        .to_image()
                })
                .collect();
            let first = formats::pixel_hash(&cuts[0]);
            if cuts.iter().all(|cut| formats::pixel_hash(cut) == first) {
                still += 1;
                continue;
            }
            let column = block.0 + x;
            let row_index = block.1 + y;
            let index = row_index * columns + column;
            let redrawn = hashes
                .get(index as usize)
                .is_some_and(|(hash, _)| *hash != first);
            let mut frames = vec![AnimFrameRef::Block];
            for cut in cuts.iter().skip(1) {
                let hash = formats::pixel_hash(cut);
                if hash == first {
                    frames.push(AnimFrameRef::Block);
                } else if let Some(existing) = by_hash.get(hash.as_str()) {
                    reused.insert(*existing);
                    frames.push(AnimFrameRef::Existing(*existing));
                } else if let Some(position) = new_hashes.iter().position(|seen| *seen == hash) {
                    frames.push(AnimFrameRef::New(position));
                } else {
                    new_hashes.push(hash);
                    new_images.push(cut.clone());
                    frames.push(AnimFrameRef::New(new_images.len() - 1));
                }
            }
            cells.push(AnimCell {
                column,
                row: row_index,
                tile: index,
                redrawn,
                frames,
            });
        }
    }
    if cells.is_empty() {
        return Err(
            "the block never changes between frames; a still block animates nothing".to_owned(),
        );
    }
    let mut warnings = Vec::new();
    let mut fits = None;
    if kind_limits {
        let mut colors = BTreeSet::new();
        let mut soft = false;
        for cell in &cells {
            if let Some(image) = tile_image(tileset, tile, cell.tile) {
                for pixel in image.pixels() {
                    colors.insert(pixel.0);
                    soft |= pixel[3] != 0 && pixel[3] != 255;
                }
            }
        }
        let over = colors.len() > MAX_COLORS;
        if over || soft {
            warnings.push(format!(
                "the tiles have {}{}; a GIF keeps {MAX_COLORS} colors and hard transparency, so the frames lose detail",
                if over {
                    format!("{} colors", colors.len())
                } else {
                    String::new()
                },
                match (over, soft) {
                    (true, true) => " and soft edges",
                    (false, true) => "soft edges",
                    _ => "",
                }
            ));
        } else {
            fits = Some(format!(
                "Fits a GIF's limits: {} colors, and no soft edges to lose.",
                colors.len()
            ));
        }
    }
    Ok(AnimPlan {
        block,
        size,
        cells,
        still,
        reused: reused.into_iter().collect(),
        new_images,
        lengths: strip
            .frames
            .iter()
            .map(|frame| frame.ms.unwrap_or(formats::DEFAULT_MS))
            .collect(),
        warnings,
        fits,
    })
}

pub fn apply_anim_plan(
    plan: &AnimPlan,
    tileset: &RgbaImage,
    row: &Value,
    export: &AnimationExport,
) -> Result<(Option<Vec<u8>>, Value), String> {
    let tile = row["tile"].as_u64().unwrap_or(16).max(1) as u32;
    let columns = (tileset.width() / tile).max(1);
    let old_count = columns * (tileset.height() / tile);
    let total = old_count + plan.new_images.len() as u32;
    let rows = total.div_ceil(columns);
    let mut image = RgbaImage::new(columns * tile, rows * tile);
    image
        .copy_from(tileset, 0, 0)
        .map_err(|error| error.to_string())?;
    let strip = export.strips.first().ok_or("the export has no frames")?;
    let mut changed = !plan.new_images.is_empty();
    for cell in &plan.cells {
        if cell.redrawn {
            let x = (cell.column - plan.block.0) * tile;
            let y = (cell.row - plan.block.1) * tile;
            let cut =
                image::imageops::crop_imm(&strip.frames[0].image, x, y, tile, tile).to_image();
            image
                .copy_from(&cut, cell.column * tile, cell.row * tile)
                .map_err(|error| error.to_string())?;
            changed = true;
        }
    }
    for (offset, new) in plan.new_images.iter().enumerate() {
        let index = old_count + offset as u32;
        image
            .copy_from(new, (index % columns) * tile, (index / columns) * tile)
            .map_err(|error| error.to_string())?;
    }
    let mut row = row.clone();
    if !row["tiles"].is_object() {
        row["tiles"] = json!({});
    }
    for cell in &plan.cells {
        let frames: Vec<Value> = cell
            .frames
            .iter()
            .zip(&plan.lengths)
            .map(|(frame, ms)| {
                let tile = match frame {
                    AnimFrameRef::Block => cell.tile,
                    AnimFrameRef::Existing(index) => *index,
                    AnimFrameRef::New(offset) => old_count + *offset as u32,
                };
                json!({ "tile": tile, "ms": ms })
            })
            .collect();
        let entry = &mut row["tiles"][cell.tile.to_string()];
        if !entry.is_object() {
            *entry = json!({});
        }
        entry["animation"] = Value::Array(frames);
    }
    let png = if changed {
        Some(formats::encode_png(&image)?)
    } else {
        None
    };
    Ok((png, row))
}
