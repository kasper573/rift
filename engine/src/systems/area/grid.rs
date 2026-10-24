use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{AsRefStr, EnumString};

use super::Tilemap;
use super::scene::PALETTE_FILE;
use super::tileset::TilesetDef;
use crate::core::content::id::{Leaf, written_path};
use crate::core::content::media::ImageSizes;
use crate::core::content::{
    BesideFile, BesideName, Content, ContentFile, ContentProblem, ContentScope, ErrorPlace,
    LoadError, with_content,
};
use crate::core::math::{Pos, Rect};
use crate::core::units::{GridSize, Meters};
use crate::systems::area::tileset::TilesetId;

#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Flip {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

impl Flip {
    pub fn is_none(&self) -> bool {
        *self == Flip::None
    }

    pub fn of(horizontal: bool, vertical: bool) -> Flip {
        match (horizontal, vertical) {
            (false, false) => Flip::None,
            (true, false) => Flip::Horizontal,
            (false, true) => Flip::Vertical,
            (true, true) => Flip::Both,
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Flip::Horizontal | Flip::Both)
    }

    pub fn vertical(self) -> bool {
        matches!(self, Flip::Vertical | Flip::Both)
    }

    pub fn compose(self, other: Flip) -> Flip {
        Flip::of(
            self.horizontal() ^ other.horizontal(),
            self.vertical() ^ other.vertical(),
        )
    }

    pub fn place(self, r: Rect<Meters>, at: Pos<Meters>) -> Rect<Meters> {
        let x = if self.horizontal() {
            at.x - (r.origin.x + r.size.width)
        } else {
            at.x + r.origin.x
        };
        let y = if self.vertical() {
            at.y - (r.origin.y + r.size.height)
        } else {
            at.y + r.origin.y
        };
        Rect::new(Pos::new(x, y), r.size)
    }

    pub fn apply(self, p: Pos<Meters>) -> Pos<Meters> {
        Pos::new(
            if self.horizontal() { -p.x } else { p.x },
            if self.vertical() { -p.y } else { p.y },
        )
    }
}

/// The palette as a part of each bundle of `table` that names tilesets.
pub fn palette_beside(table: &'static str) -> BesideFile {
    BesideFile {
        table,
        name: PALETTE_FILE,
        names: palette_names,
    }
}

fn palette_names(text: &str) -> Vec<BesideName> {
    let file = GridFile { path: "", text };
    read_palette(file, &mut Vec::new())
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, row)| BesideName {
            table: TilesetDef::TABLE,
            path: row.tileset,
            line: index + 2,
            column: 1,
        })
        .collect()
}

pub fn write_palette(rows: &[PaletteRow]) -> String {
    let header = PALETTE_HEADER.map(str::to_owned).to_vec();
    write_records(std::iter::once(header).chain(rows.iter().map(|row| {
        let flip = if row.flip.is_none() {
            ""
        } else {
            row.flip.as_ref()
        };
        vec![
            written_path(&row.tileset),
            row.tile.to_string(),
            flip.to_owned(),
        ]
    })))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteEntry {
    pub tileset: TilesetId,
    pub tile: u32,
    pub flip: Flip,
}

/// The cells of one ground layer, each a palette index or empty.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerGrid {
    pub name: String,
    pub cells: Vec<Option<u32>>,
}

/// The ground of an area or a blueprint as it draws: its palette, its layers in drawing order and
/// its forced walk cells.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaGrids {
    pub size: GridSize,
    pub palette: Vec<PaletteEntry>,
    pub palette_keys: Vec<PaletteRow>,
    pub ground: Vec<LayerGrid>,
    pub walk: Vec<Option<bool>>,
}

impl AreaGrids {
    pub fn layer_names(&self) -> Vec<String> {
        self.ground.iter().map(|layer| layer.name.clone()).collect()
    }

    pub fn tilesets(&self) -> BTreeSet<String> {
        self.palette_keys
            .iter()
            .map(|row| row.tileset.clone())
            .collect()
    }

    pub fn entry(&self, cell: Option<u32>) -> Option<PaletteEntry> {
        self.palette.get(cell? as usize).copied()
    }

    /// The index of `entry` in the palette, added at its end when the palette has none like it.
    pub fn palette_index(&mut self, entry: PaletteEntry, content: &Content) -> u32 {
        if let Some(index) = self.palette.iter().position(|known| *known == entry) {
            return index as u32;
        }
        self.palette.push(entry);
        self.palette_keys.push(PaletteRow {
            tileset: entry.tileset.path(content).to_owned(),
            tile: entry.tile,
            flip: entry.flip,
        });
        (self.palette.len() - 1) as u32
    }
}

pub fn write_walk(size: GridSize, walk: &[Option<bool>]) -> String {
    write_grid(size, walk, |cell| match cell {
        Some(true) => "1".to_owned(),
        Some(false) => "0".to_owned(),
        None => String::new(),
    })
}

pub fn write_grid_cells(size: GridSize, cells: &[Option<u32>]) -> String {
    write_grid(size, cells, |cell| match cell {
        Some(index) => index.to_string(),
        None => String::new(),
    })
}

fn write_grid<T>(size: GridSize, cells: &[T], show: impl Fn(&T) -> String) -> String {
    write_records(
        cells
            .chunks(size.width.max(1) as usize)
            .map(|row| row.iter().map(&show).collect()),
    )
}

pub struct GridFile<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

pub struct BadCell {
    pub rule: &'static str,
    pub expected: String,
}

pub fn read_grid<T>(
    file: GridFile,
    size: GridSize,
    mut cell: impl FnMut(&str) -> Result<T, BadCell>,
    errors: &mut Vec<LoadError>,
) -> Option<Vec<T>>
where
    T: Default + Clone,
{
    let height = size.height as usize;
    let width = size.width as usize;
    let mut cells = vec![T::default(); width * height];
    let mut bad = Vec::new();
    let mut rows = 0;
    let mut misfit = None;
    each_record(&file, errors, |row, record| {
        rows += 1;
        if record.len() != width {
            misfit = misfit.or(Some(record.len()));
            return;
        }
        if row >= height {
            return;
        }
        for (column, field) in record.iter().map(str::trim).enumerate() {
            match cell(field) {
                Ok(value) => cells[row * width + column] = value,
                Err(refused) => bad.push(LoadError {
                    file: file.path.to_owned(),
                    at: ErrorPlace::Csv {
                        row: row + 1,
                        column: column + 1,
                    },
                    problem: ContentProblem::new(
                        refused.rule,
                        format!("{field:?}"),
                        refused.expected,
                    ),
                }),
            }
        }
    })?;
    if rows != height || misfit.is_some() {
        let found = match misfit {
            Some(columns) if rows == height => format!("a row of {columns} columns"),
            _ => format!("{rows} rows"),
        };
        errors.push(LoadError {
            file: file.path.to_owned(),
            at: ErrorPlace::WholeFile,
            problem: ContentProblem::new(
                "grid size",
                found,
                format!("{height} rows of {width} columns, the map size"),
            ),
        });
        return None;
    }
    let ok = bad.is_empty();
    errors.extend(bad);
    ok.then_some(cells)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub tileset: String,
    pub tile: u32,
    pub flip: Flip,
}

pub fn read_palette(file: GridFile, errors: &mut Vec<LoadError>) -> Option<Vec<PaletteRow>> {
    let mut header = None;
    let mut rows = Vec::new();
    let mut bad = Vec::new();
    each_record(&file, errors, |index, record| {
        if index == 0 {
            header =
                Some((!record.iter().map(str::trim).eq(PALETTE_HEADER)).then(|| joined(record)));
            return;
        }
        match palette_row(record) {
            Ok(row) => rows.push(row),
            Err((column, found, expected)) => bad.push(LoadError {
                file: file.path.to_owned(),
                at: ErrorPlace::Csv {
                    row: index + 1,
                    column,
                },
                problem: ContentProblem::new("bad cell", format!("{found:?}"), expected),
            }),
        }
    })?;
    if let Some(found) = header.unwrap_or(Some(String::new())) {
        errors.push(LoadError {
            file: file.path.to_owned(),
            at: ErrorPlace::Csv { row: 1, column: 1 },
            problem: ContentProblem::new(
                "bad cell",
                format!("{found:?}"),
                format!("the header {}", PALETTE_HEADER.join(",")),
            ),
        });
        return None;
    }
    let ok = bad.is_empty();
    errors.extend(bad);
    ok.then_some(rows)
}

const PALETTE_HEADER: [&str; 3] = ["tileset", "tile", "flip"];

fn palette_row(record: &csv::StringRecord) -> Result<PaletteRow, (usize, String, &'static str)> {
    if record.len() != PALETTE_HEADER.len() {
        return Err((1, joined(record), "three fields: tileset,tile,flip"));
    }
    let field = |index: usize| record[index].trim();
    let tile = field(1)
        .parse::<u32>()
        .map_err(|_| (2, field(1).to_owned(), "a tile number"))?;
    let flip = match field(2) {
        "" => Ok(Flip::None),
        text => text.parse::<Flip>(),
    }
    .map_err(|_| {
        (
            3,
            field(2).to_owned(),
            "empty, horizontal, vertical or both",
        )
    })?;
    Ok(PaletteRow {
        tileset: field(0).to_owned(),
        tile,
        flip,
    })
}

/// Hands `each` every record of `file` in turn, by its index, through one record reused for all,
/// as a large grid holds many thousands; `None` once the file doesn't read as CSV.
fn each_record(
    file: &GridFile,
    errors: &mut Vec<LoadError>,
    mut each: impl FnMut(usize, &csv::StringRecord),
) -> Option<()> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(file.text.as_bytes());
    let mut record = csv::StringRecord::new();
    for index in 0.. {
        match reader.read_record(&mut record) {
            Ok(true) => each(index, &record),
            Ok(false) => break,
            Err(error) => {
                errors.push(LoadError {
                    file: file.path.to_owned(),
                    at: ErrorPlace::WholeFile,
                    problem: ContentProblem::new("unreadable file", error, "a UTF-8 CSV file"),
                });
                return None;
            }
        }
    }
    Some(())
}

fn joined(record: &csv::StringRecord) -> String {
    record.iter().map(str::trim).collect::<Vec<_>>().join(",")
}

fn write_records(records: impl Iterator<Item = Vec<String>>) -> String {
    let mut writer = csv::Writer::from_writer(Vec::new());
    for record in records {
        writer
            .write_record(record)
            .expect("a record writes to memory");
    }
    String::from_utf8(writer.into_inner().expect("records write to memory"))
        .expect("records write as UTF-8")
}

/// The palette's rows as the tiles they name, when every row names a tile its tileset has at the
/// map's tile size.
pub fn resolve_palette(
    content: &Content,
    sizes: &ImageSizes,
    map: &Tilemap,
    file: &str,
    rows: &[PaletteRow],
    errors: &mut Vec<LoadError>,
) -> Option<Vec<PaletteEntry>> {
    let tilesets = content.table::<TilesetDef>();
    let (ids, scope) = with_content(ContentScope::of(content), || {
        rows.iter()
            .map(|row| serde_json::from_value::<TilesetId>(Value::String(row.tileset.clone())).ok())
            .collect::<Vec<_>>()
    });
    let mut fault = |row: usize, column: usize, problem: ContentProblem| {
        errors.push(LoadError {
            file: file.to_owned(),
            at: ErrorPlace::Csv {
                row: row + 2,
                column,
            },
            problem,
        });
        None
    };
    let entries: Vec<Option<PaletteEntry>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            if let Some(problem) = scope.leaves().get(index).and_then(Leaf::reported) {
                return fault(index, 1, problem);
            }
            let tileset = ids[index]?;
            let def = tilesets.get(tileset)?;
            if def.tile != map.tile {
                return fault(
                    index,
                    1,
                    ContentProblem::new(
                        "tile size mismatch",
                        format!("{} against {}", map.tile, def.tile),
                        format!("tilesets of {} px tiles, the map's", map.tile),
                    ),
                );
            }
            if let Some(&(width, height)) = sizes.0.get(def.image.as_str()) {
                let count = def.grid(width, height).count();
                if row.tile >= count {
                    return fault(
                        index,
                        2,
                        ContentProblem::new(
                            "tile out of range",
                            format!("tile {} of {count}", row.tile),
                            format!("0 to {}", count.saturating_sub(1)),
                        ),
                    );
                }
            }
            Some(PaletteEntry {
                tileset,
                tile: row.tile,
                flip: row.flip,
            })
        })
        .collect();
    entries.into_iter().collect()
}

pub fn palette_cell(field: &str, entries: usize) -> Result<Option<u32>, BadCell> {
    if field.is_empty() {
        return Ok(None);
    }
    let index: u32 = field.parse().map_err(|_| BadCell {
        rule: "bad cell",
        expected: "empty or a palette index".to_owned(),
    })?;
    if index as usize >= entries {
        return Err(BadCell {
            rule: "palette out of range",
            expected: format!(
                "0 to {}, an index into the {entries} palette entries",
                entries.saturating_sub(1)
            ),
        });
    }
    Ok(Some(index))
}

pub fn walk_cell(field: &str) -> Result<Option<bool>, BadCell> {
    match field {
        "" => Ok(None),
        "1" => Ok(Some(true)),
        "0" => Ok(Some(false)),
        _ => Err(BadCell {
            rule: "bad cell",
            expected: "empty, 1 or 0".to_owned(),
        }),
    }
}
