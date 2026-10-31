//! An area laid out from noise, as a landscape starts: ground rising and falling over its relief
//! in features of its wavelength, painted by height from the lowest terrain to the highest in
//! bands of equal ground, strewn with props where a terrain lies and roamed by npcs, so a game
//! has a wide land to fill in rather than a flat one.

use std::path::Path;

use clap::Args;
use engine::core::content::ContentFile;
use engine::core::content::fresh::Nearby;
use engine::core::ground::{GroundSize, Heightfield, Paint};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::Meters;
use engine::systems::area::prop::{PropId, ScaleRange, Scatter};
use engine::systems::area::scene::{Holes, ModuleNode, SceneFolder, SceneNode};
use engine::systems::area::terrain::TerrainId;
use engine::systems::area::{AreaDef, AreaModule, Population};
use engine::systems::npc::NpcId;
use noise::{Fbm, NoiseFn, Perlin};

use crate::folder;

#[derive(Args, Debug)]
pub struct Land {
    /// The area's name.
    #[arg(long)]
    pub name: String,
    /// The area's width, in metres.
    #[arg(long)]
    pub width: u32,
    /// The area's height, in metres.
    #[arg(long)]
    pub height: u32,
    /// What the noise is drawn from: the same seed lays out the same land.
    #[arg(long)]
    pub seed: u64,
    /// How far the ground rises from its lowest point to its highest, in metres.
    #[arg(long)]
    pub relief: f32,
    /// How wide the ground's features are, in metres: hills and hollows about this far apart.
    #[arg(long)]
    pub wavelength: f32,
    /// The terrains that paint the ground, from the lowest ground to the highest, each a terrain
    /// file's path in the game folder; the first is the ground where nothing is painted.
    #[arg(long = "terrain", required = true)]
    pub terrains: Vec<String>,
    /// Props strewn where a terrain lies: TERRAIN:PROP:PER_SQUARE_METER, with TERRAIN the index
    /// of one of the terrains given, PROP a prop file's path, and after another colon the scale
    /// the copies range over as MIN-MAX.
    #[arg(long = "scatter")]
    pub scatters: Vec<String>,
    /// Npcs roaming the land: NPC:PER_SQUARE_KILOMETER, with NPC an npc file's path.
    #[arg(long = "population")]
    pub populations: Vec<String>,
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
        .map(|path| reader.from_json::<TerrainId>(&serde_json::Value::String(path.clone())))
        .collect::<Result<_, _>>()?;
    let scatters: Vec<ScatterSpec> = land
        .scatters
        .iter()
        .map(|spec| ScatterSpec::parse(spec, &reader, land.terrains.len()))
        .collect::<Result<_, _>>()?;
    let populations: Vec<PopulationSpec> = land
        .populations
        .iter()
        .map(|spec| PopulationSpec::parse(spec, &reader))
        .collect::<Result<_, _>>()?;

    area.heights = heights(area.size, land);
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
fn heights(size: GroundSize, land: &Land) -> Heightfield {
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
    Heightfield::from_fn(size, |column, row| {
        Meters((raw[(row * columns + column) as usize] - low) / span * land.relief)
    })
}

/// Each terrain over as much ground as the next, from the lowest ground up.
fn paint(size: GroundSize, heights: &Heightfield, terrains: usize) -> Paint {
    let (columns, rows) = size.paint_pixels();
    let at_pixel = |column: u32, row: u32| heights.height_at(Paint::pixel_pos(column, row)).0;
    let mut sorted: Vec<f32> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| at_pixel(column, row)))
        .collect();
    sorted.sort_by(f32::total_cmp);
    let bands: Vec<f32> = (1..terrains)
        .map(|band| sorted[band * sorted.len() / terrains])
        .collect();
    Paint::from_fn(size, |column, row| {
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
    fn parse(
        spec: &str,
        reader: &engine::core::content::FolderReader,
        terrains: usize,
    ) -> Result<ScatterSpec, String> {
        let mut parts = spec.split(':');
        let (terrain, prop, density) = (parts.next(), parts.next(), parts.next());
        let (Some(terrain), Some(prop), Some(density)) = (terrain, prop, density) else {
            return Err(format!(
                "a scatter is TERRAIN:PROP:PER_SQUARE_METER, not {spec}"
            ));
        };
        let terrain: usize = terrain
            .parse()
            .map_err(|_| format!("{terrain} is not the index of a terrain"))?;
        if terrain >= terrains {
            return Err(format!(
                "terrain {terrain} is not among the {terrains} given"
            ));
        }
        let scale = match parts.next() {
            None => ScaleRange::default(),
            Some(range) => {
                let (min, max) = range
                    .split_once('-')
                    .ok_or_else(|| format!("a scale range is MIN-MAX, not {range}"))?;
                ScaleRange {
                    min: min.parse().map_err(|_| format!("{min} is not a scale"))?,
                    max: max.parse().map_err(|_| format!("{max} is not a scale"))?,
                }
            }
        };
        Ok(ScatterSpec {
            terrain: terrain as u8,
            prop: reader.from_json(&serde_json::Value::String(prop.to_owned()))?,
            per_square_meter: density
                .parse()
                .map_err(|_| format!("{density} is not a density"))?,
            scale,
            label: format!("{prop} on terrain {terrain}"),
        })
    }

    /// A scatter over every block, a quarter of a wavelength across, where the terrain covers
    /// most of the ground.
    fn folder(&self, size: GroundSize, paint: &Paint, land: &Land) -> SceneNode {
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
fn mostly(paint: &Paint, block: &Rect<Meters>, terrain: u8) -> bool {
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
    fn parse(
        spec: &str,
        reader: &engine::core::content::FolderReader,
    ) -> Result<PopulationSpec, String> {
        let (npc, density) = spec
            .split_once(':')
            .ok_or_else(|| format!("a population is NPC:PER_SQUARE_KILOMETER, not {spec}"))?;
        Ok(PopulationSpec {
            npc: reader.from_json(&serde_json::Value::String(npc.to_owned()))?,
            per_square_kilometer: density
                .parse()
                .map_err(|_| format!("{density} is not a density"))?,
            label: npc.to_owned(),
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
