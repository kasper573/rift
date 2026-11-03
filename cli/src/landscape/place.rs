//! What grows on the land and roams it: each biome's props strewn block by block, as thick as the
//! share of the block its ground covers where it can be walked, thickening and thinning in clumps;
//! and its monsters roaming tile by tile, as many as the share of the tile it covers holds.

use engine::core::chunk::CHUNK_SIDE;
use engine::core::ground::{GroundSize, TILE_SIDE};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::{Degrees, Meters};
use engine::systems::area::prop::Scatter;
use engine::systems::area::scene::{Holes, ModuleNode, SceneFolder, SceneNode};
use engine::systems::area::{AreaModule, Population};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rayon::prelude::*;

use super::biomes::{Biomes, Climate};
use super::relief::Relief;
use super::{LAID_OUT, Land, Stream, layer_of};

pub fn lay_out(
    size: GroundSize,
    relief: &Relief,
    climate: &Climate,
    biomes: &Biomes,
    max_slope: Degrees,
    land: &Land,
) -> SceneNode {
    let prop_blocks = covered(size, PROP_BLOCK, relief, climate, biomes, max_slope);
    let tiles = covered(size, TILE_SIDE.0, relief, climate, biomes, max_slope);
    let children = biomes
        .all()
        .iter()
        .enumerate()
        .map(|(index, biome)| {
            let props = biome.props.iter().enumerate().map(|(kind, prop)| {
                let clumps = Fbm::<Perlin>::new(land.seed_of(Stream::Clumps {
                    biome: index,
                    prop: kind,
                }))
                .set_octaves(2)
                .set_frequency(1.0 / f64::from(PROP_BLOCK * BLOCKS_PER_CLUMP));
                let modules = prop_blocks
                    .iter()
                    .filter_map(|(block, cover)| {
                        let middle = block.center();
                        let clumped = 1.0
                            + prop.clumping
                                * 2.0
                                * clumps.get([f64::from(middle.x), f64::from(middle.y)]) as f32;
                        let per_square_meter =
                            prop.per_square_meter * cover[index] * clumped.max(0.0);
                        (per_square_meter * block.area() >= HALF_A_COPY).then(|| {
                            module(AreaModule::Scatter(Scatter {
                                props: prop.props.clone(),
                                region: *block,
                                per_square_meter,
                                seed: block_seed(land.seed, block, layer_of(index, kind) as u64),
                                scale: prop.scale,
                            }))
                        })
                    })
                    .collect();
                folder(&prop.label, modules)
            });
            let monsters = biome.monsters.iter().map(|monster| {
                let modules = tiles
                    .iter()
                    .filter_map(|(tile, cover)| {
                        let count = (monster.per_square_kilometer * tile.area() / 1_000_000.0
                            * cover[index])
                            .round() as u32;
                        (count > 0).then(|| {
                            module(AreaModule::Population(Population {
                                npc: monster.npc,
                                count,
                                roams: Some(*tile),
                            }))
                        })
                    })
                    .collect();
                folder(&monster.path, modules)
            });
            folder(&biome.name, props.chain(monsters).collect())
        })
        .collect();
    folder(LAID_OUT, children)
}

/// The side of the blocks props are strewn in: small enough that a biome's props stay near its
/// ground, large enough that the scene stays small.
const PROP_BLOCK: f32 = CHUNK_SIDE.0 * 2.0;
/// How many blocks across a clump of props and the clearing beside it are.
const BLOCKS_PER_CLUMP: f32 = 3.0;
/// How many places a block is read at along each side to tell what covers it.
const READINGS_PER_SIDE: u32 = 8;
/// The fewest copies a block's scatter expects to strew to be laid out at all.
const HALF_A_COPY: f32 = 0.5;

/// Every block `side` metres square, the last row and column cut to fit, with the share of it
/// each biome covers where the ground can be walked.
fn covered(
    size: GroundSize,
    side: f32,
    relief: &Relief,
    climate: &Climate,
    biomes: &Biomes,
    max_slope: Degrees,
) -> Vec<(Rect<Meters>, Vec<f32>)> {
    blocks(size, side)
        .into_par_iter()
        .map(|block| {
            let mut cover = vec![0.0; biomes.all().len()];
            let step = |length: f32, at: u32| (at as f32 + 0.5) * length / READINGS_PER_SIDE as f32;
            let reading = 1.0 / (READINGS_PER_SIDE * READINGS_PER_SIDE) as f32;
            for row in 0..READINGS_PER_SIDE {
                for column in 0..READINGS_PER_SIDE {
                    let at = Pos::new(
                        block.origin.x + step(block.size.width, column),
                        block.origin.y + step(block.size.height, row),
                    );
                    if relief.slope(at).0 <= max_slope.0 {
                        cover[biomes.at(relief, climate, at)] += reading;
                    }
                }
            }
            (block, cover)
        })
        .collect()
}

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

/// A seed of its own for every block and kind, mixed from the land's seed and the block's place
/// (splitmix64's finalizer).
fn block_seed(seed: u64, block: &Rect<Meters>, kind: u64) -> u64 {
    let mut mixed = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in [
        u64::from(block.origin.x.to_bits()),
        u64::from(block.origin.y.to_bits()),
        kind,
    ] {
        mixed ^= part.wrapping_add(0x9E37_79B9_7F4A_7C15);
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

fn folder(label: &str, children: Vec<SceneNode>) -> SceneNode {
    SceneNode::Folder(SceneFolder {
        label: label.to_owned(),
        hidden: false,
        locked: false,
        children,
    })
}
