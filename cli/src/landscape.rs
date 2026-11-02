//! An area laid out from noise, as a landscape starts: ground rising and falling over its relief
//! in features of its wavelength, painted by height from the lowest terrain to the highest in
//! bands of equal ground, strewn with props where a terrain lies and roamed by npcs, so a game
//! has a wide land to fill in rather than a flat one.

use std::path::Path;

use engine::core::content::fresh::Nearby;
use engine::core::content::{ContentFile, FolderReader};
use engine::core::ground::{GroundSize, HeightRange, Heightfield, PaintMap};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::Meters;
use engine::systems::area::prop::{PropId, ScaleRange, Scatter};
use engine::systems::area::scene::{Holes, ModuleNode, SceneFolder, SceneNode};
use engine::systems::area::terrain::TerrainId;
use engine::systems::area::{AreaDef, AreaModule, Population};
use engine::systems::npc::NpcId;
use noise::{Fbm, NoiseFn, Perlin};
use serde::Deserialize;

use crate::folder;

/// How an area is laid out from noise, read from a JSON file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Land {
    /// The area's name.
    pub name: String,
    /// The area's width, in metres.
    pub width: u32,
    /// The area's height, in metres.
    pub height: u32,
    /// What the noise is drawn from: the same seed lays out the same land.
    pub seed: u64,
    /// How far the ground rises from its lowest point to its highest, in metres.
    pub relief: f32,
    /// How wide the ground's features are, in metres: hills and hollows about this far apart.
    pub wavelength: f32,
    /// The terrains that paint the ground, from the lowest ground to the highest, each a terrain
    /// file's path in the game folder; the first is the ground where nothing is painted.
    pub terrains: Vec<String>,
    /// Props strewn where a terrain lies.
    #[serde(default)]
    pub scatters: Vec<LandScatter>,
    /// Npcs roaming the land.
    #[serde(default)]
    pub populations: Vec<LandPopulation>,
}

/// A prop strewn where one of the land's terrains lies, `per_square_meter` over its ground, its
/// copies scaled over `scale`.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandScatter {
    pub terrain: String,
    pub prop: String,
    pub per_square_meter: f32,
    #[serde(default)]
    pub scale: ScaleRange,
}

/// An npc roaming the land, `per_square_kilometer` over all of it.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandPopulation {
    pub npc: String,
    pub per_square_kilometer: f32,
}

/// The land the JSON file at `path` lays out.
pub fn read_land(path: &Path) -> Result<Land, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{} doesn't read: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn run(game: &Path, area_path: &str, land: &Land) -> Result<(), String> {
    let reader = folder::reader_of(game);
    let mut area = AreaDef::fresh(&Nearby::new(&reader, area_path))?;
    area.name = land.name.clone();
    area.size = GroundSize {
        width: land.width,
        height: land.height,
    };
    area.terrains = land
        .terrains
        .iter()
        .map(|path| reader.id_at::<TerrainId>(path))
        .collect::<Result<_, _>>()?;
    let scatters: Vec<ScatterSpec> = land
        .scatters
        .iter()
        .map(|scatter| ScatterSpec::resolve(scatter, &reader, &land.terrains))
        .collect::<Result<_, _>>()?;
    let populations: Vec<PopulationSpec> = land
        .populations
        .iter()
        .map(|population| PopulationSpec::resolve(population, &reader))
        .collect::<Result<_, _>>()?;

    if land.relief <= 0.0 {
        return Err(format!("a relief of {} m lays out no land", land.relief));
    }
    area.height_range = HeightRange {
        lowest: Meters(0.0),
        highest: Meters(land.relief),
    };
    area.heights = heights(area.size, area.height_range, land);
    area.paint = paint(area.size, &area.heights, land.terrains.len());
    area.scene = scatters
        .iter()
        .map(|scatter| scatter.folder(area.size, &area.paint, land))
        .chain(
            populations
                .iter()
                .map(|population| population.folder(area.size, land)),
        )
        .collect();
    folder::write(game, reader.write(area_path, &area))
}

/// The ground from the noise, lifted and stretched to rise over the relief exactly.
fn heights(size: GroundSize, range: HeightRange, land: &Land) -> Heightfield {
    let noise = Fbm::<Perlin>::new(land.seed as u32);
    let (columns, rows) = size.samples();
    let raw: Vec<f32> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| {
            let at = Heightfield::sample_pos(column, row);
            noise.get([
                f64::from(at.x / land.wavelength),
                f64::from(at.y / land.wavelength),
            ]) as f32
        })
        .collect();
    let low = raw.iter().copied().fold(f32::INFINITY, f32::min);
    let high = raw.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (high - low).max(f32::EPSILON);
    Heightfield::from_fn(size, range, |column, row| {
        Meters((raw[(row * columns + column) as usize] - low) / span * land.relief)
    })
}

/// Each terrain over as much ground as the next, from the lowest ground up.
fn paint(size: GroundSize, heights: &Heightfield, terrains: usize) -> PaintMap {
    let (columns, rows) = size.paint_pixels();
    let at_pixel = |column: u32, row: u32| heights.height_at(PaintMap::pixel_pos(column, row)).0;
    let mut sorted: Vec<f32> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| at_pixel(column, row)))
        .collect();
    sorted.sort_by(f32::total_cmp);
    let bands: Vec<f32> = (1..terrains)
        .map(|band| sorted[band * sorted.len() / terrains])
        .collect();
    PaintMap::from_fn(size, |column, row| {
        let height = at_pixel(column, row);
        bands.iter().take_while(|&&from| height >= from).count() as u8
    })
}

/// The blocks the land is dealt in, `side` metres square, the last row and column cut to fit.
fn blocks(size: GroundSize, side: f32) -> Vec<Rect<Meters>> {
    let bounds = size.bounds();
    let across = (bounds.size.width / side).ceil() as u32;
    let down = (bounds.size.height / side).ceil() as u32;
    (0..down)
        .flat_map(|row| (0..across).map(move |column| (column, row)))
        .map(|(column, row)| {
            let origin = Pos::new(
                bounds.origin.x + column as f32 * side,
                bounds.origin.y + row as f32 * side,
            );
            Rect::new(
                origin,
                Size::new(
                    side.min(bounds.max_x() - origin.x),
                    side.min(bounds.max_y() - origin.y),
                ),
            )
        })
        .collect()
}

/// A seed of its own for every block, mixed from the land's seed and the block's place.
fn block_seed(seed: u64, block: &Rect<Meters>) -> u64 {
    let mut mixed = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in [block.origin.x.to_bits(), block.origin.y.to_bits()] {
        mixed ^= u64::from(part).wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed ^= mixed >> 31;
    }
    mixed
}

fn module(module: AreaModule) -> SceneNode {
    SceneNode::Module(ModuleNode {
        label: None,
        hidden: false,
        locked: false,
        module,
        holes: Holes::default(),
    })
}

struct ScatterSpec {
    terrain: u8,
    prop: PropId,
    per_square_meter: f32,
    scale: ScaleRange,
    label: String,
}

impl ScatterSpec {
    fn resolve(
        scatter: &LandScatter,
        reader: &FolderReader,
        terrains: &[String],
    ) -> Result<ScatterSpec, String> {
        let terrain = terrains
            .iter()
            .position(|terrain| *terrain == scatter.terrain)
            .ok_or_else(|| format!("{} is not among the land's terrains", scatter.terrain))?;
        Ok(ScatterSpec {
            terrain: terrain as u8,
            prop: reader.id_at(&scatter.prop)?,
            per_square_meter: scatter.per_square_meter,
            scale: scatter.scale,
            label: format!("{} on {}", scatter.prop, scatter.terrain),
        })
    }

    /// A scatter over every block, a quarter of a wavelength across, where the terrain covers
    /// most of the ground.
    fn folder(&self, size: GroundSize, paint: &PaintMap, land: &Land) -> SceneNode {
        let children = blocks(size, land.wavelength / 4.0)
            .into_iter()
            .filter(|block| mostly(paint, block, self.terrain))
            .map(|block| {
                module(AreaModule::Scatter(Scatter {
                    props: vec![self.prop],
                    region: block,
                    per_square_meter: self.per_square_meter,
                    seed: block_seed(land.seed, &block),
                    scale: self.scale,
                }))
            })
            .collect();
        SceneNode::Folder(SceneFolder {
            label: self.label.clone(),
            hidden: false,
            locked: false,
            children,
        })
    }
}

/// Whether `terrain` is painted over more than half of `block`.
fn mostly(paint: &PaintMap, block: &Rect<Meters>, terrain: u8) -> bool {
    let (first_column, first_row) = paint.pixel_at(block.origin);
    let (last_column, last_row) = paint.pixel_at(Pos::new(block.max_x(), block.max_y()));
    let (columns, rows) = paint.pixels();
    let mut painted = 0;
    let mut all = 0;
    for row in first_row..last_row.min(rows) {
        for column in first_column..last_column.min(columns) {
            all += 1;
            if paint.index(column, row) == terrain {
                painted += 1;
            }
        }
    }
    painted * 2 > all
}

struct PopulationSpec {
    npc: NpcId,
    per_square_kilometer: f32,
    label: String,
}

impl PopulationSpec {
    fn resolve(
        population: &LandPopulation,
        reader: &FolderReader,
    ) -> Result<PopulationSpec, String> {
        Ok(PopulationSpec {
            npc: reader.id_at(&population.npc)?,
            per_square_kilometer: population.per_square_kilometer,
            label: population.npc.clone(),
        })
    }

    /// A population roaming every block a wavelength across, as many as its ground holds at the
    /// density, where that is at least one.
    fn folder(&self, size: GroundSize, land: &Land) -> SceneNode {
        let children = blocks(size, land.wavelength)
            .into_iter()
            .filter_map(|block| {
                let count = (self.per_square_kilometer * block.area() / 1_000_000.0).round() as u32;
                (count > 0).then(|| {
                    module(AreaModule::Population(Population {
                        npc: self.npc,
                        count,
                        roams: Some(block),
                    }))
                })
            })
            .collect();
        SceneNode::Folder(SceneFolder {
            label: self.label.clone(),
            hidden: false,
            locked: false,
            children,
        })
    }
}
