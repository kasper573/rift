//! The land's climate and the biomes it places: each place is as warm and as wet as its share of
//! the land, and belongs to the biome whose climate is nearest its own. Where biomes meet, each
//! place reads the climate a little off its own, by noise, so their borders interleave in tongues
//! and islands rather than run in lines.

use engine::core::content::FolderReader;
use engine::core::ground::{GroundSize, Heightfield, PaintMap};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::prop::{PropId, ScaleRange};
use engine::systems::area::terrain::TerrainId;
use engine::systems::npc::NpcId;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rayon::prelude::*;

use super::relief::Relief;
use super::{Land, LandGround};

/// How warm and how wet each node of the relief's lattice is, as a share of the land.
pub struct Climate {
    temperature: Vec<f32>,
    moisture: Vec<f32>,
}

impl Climate {
    pub fn over(relief: &Relief, land: &Land) -> Climate {
        let seed = land.seed as u32;
        let warmth = Fbm::<Perlin>::new(seed.wrapping_add(6)).set_octaves(4);
        let wetness = Fbm::<Perlin>::new(seed.wrapping_add(7)).set_octaves(4);
        let wavelength = f64::from(land.climate.wavelength.0);
        let (temperature, moisture): (Vec<f32>, Vec<f32>) = relief
            .nodes()
            .map(|at| {
                let reading = [f64::from(at.x) / wavelength, f64::from(at.y) / wavelength];
                (
                    warmth.get(reading) as f32
                        - land.climate.height_chill * relief.elevation(at) * CLIMATE_SPREAD,
                    wetness.get(reading) as f32
                        + land.climate.runoff_wetness * relief.runoff(at) * CLIMATE_SPREAD,
                )
            })
            .unzip();
        Climate {
            temperature: shares(&temperature),
            moisture: shares(&moisture),
        }
    }
}

/// The land's biomes, read against the game, and every terrain they paint with.
pub struct Biomes {
    biomes: Vec<Biome>,
    terrains: Vec<TerrainId>,
    border: Border,
}

pub struct Biome {
    temperature: f32,
    moisture: f32,
    ground: Vec<Ground>,
    pub props: Vec<BiomeProp>,
    pub monsters: Vec<BiomeMonster>,
    pub name: String,
}

pub struct BiomeProp {
    pub props: Vec<PropId>,
    pub label: String,
    pub per_square_meter: f32,
    pub scale: ScaleRange,
    pub clumping: f32,
}

pub struct BiomeMonster {
    pub npc: NpcId,
    pub path: String,
    pub per_square_kilometer: f32,
}

impl Biomes {
    pub fn resolve(land: &Land, reader: &FolderReader) -> Result<Biomes, String> {
        let mut paths: Vec<&str> = Vec::new();
        let mut biomes = Vec::new();
        for (index, biome) in land.biomes.iter().enumerate() {
            let ground = biome
                .ground
                .iter()
                .enumerate()
                .map(|(layer, ground)| {
                    let terrain = match paths.iter().position(|path| *path == ground.terrain) {
                        Some(known) => known,
                        None => {
                            paths.push(&ground.terrain);
                            paths.len() - 1
                        }
                    };
                    Ground::new(ground, terrain, land, index * 64 + layer)
                })
                .collect();
            let props = biome
                .props
                .iter()
                .map(|prop| {
                    Ok(BiomeProp {
                        props: prop
                            .props
                            .iter()
                            .map(|path| reader.id_at(path))
                            .collect::<Result<_, _>>()?,
                        label: prop.props.join(", "),
                        per_square_meter: prop.per_square_meter,
                        scale: prop.scale,
                        clumping: prop.clumping,
                    })
                })
                .collect::<Result<_, String>>()?;
            let monsters = biome
                .monsters
                .iter()
                .map(|monster| {
                    Ok(BiomeMonster {
                        npc: reader.id_at(&monster.npc)?,
                        path: monster.npc.clone(),
                        per_square_kilometer: monster.per_square_kilometer,
                    })
                })
                .collect::<Result<_, String>>()?;
            biomes.push(Biome {
                temperature: biome.temperature,
                moisture: biome.moisture,
                ground,
                props,
                monsters,
                name: biome.name.clone(),
            });
        }
        if paths.len() > usize::from(u8::MAX) + 1 {
            return Err(format!(
                "the biomes paint with {} terrains, more than an area holds",
                paths.len()
            ));
        }
        let terrains = paths
            .iter()
            .map(|path| reader.id_at::<TerrainId>(path))
            .collect::<Result<_, _>>()?;
        Ok(Biomes {
            biomes,
            terrains,
            border: Border::new(land),
        })
    }

    /// Every terrain the biomes paint with, in the order the paint numbers them.
    pub fn terrains(&self) -> &[TerrainId] {
        &self.terrains
    }

    pub fn all(&self) -> &[Biome] {
        &self.biomes
    }

    /// The biome `at` belongs to.
    pub fn at(&self, relief: &Relief, climate: &Climate, at: Pos<Meters>) -> usize {
        let temperature = relief.between_nodes(&climate.temperature, at);
        let moisture = relief.between_nodes(&climate.moisture, at);
        let (temperature, moisture) = self.border.jostled(temperature, moisture, at);
        self.biomes
            .iter()
            .enumerate()
            .map(|(index, biome)| {
                let (dt, dm) = (biome.temperature - temperature, biome.moisture - moisture);
                (index, dt * dt + dm * dm)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(index, _)| index)
    }

    /// The ground painted by the biome of each place, by the first of its terrains whose rules
    /// hold there.
    pub fn paint(
        &self,
        size: GroundSize,
        relief: &Relief,
        climate: &Climate,
        heights: &Heightfield,
        land: &Land,
    ) -> PaintMap {
        let relief_height = land.relief.0;
        PaintMap::from_fn(size, |column, row| {
            let at = PaintMap::pixel_pos(column, row);
            let biome = &self.biomes[self.at(relief, climate, at)];
            let elevation = heights.height_at(at).0 / relief_height;
            let slope = ground_slope(heights, at);
            biome
                .ground
                .iter()
                .find(|ground| ground.paints(at, elevation, slope))
                .unwrap_or_else(|| biome.ground.last().expect("a biome has ground"))
                .terrain
        })
    }
}

/// How far apart the warmest and coldest, wettest and driest regions read before they are taken
/// as shares, as fractal noise spreads: the height's chill and the runoff's wetness are told on
/// this scale.
const CLIMATE_SPREAD: f32 = 0.6;
/// How many patches of a biome's terrains and tongues of its border fit across a wavelength of
/// the climate.
const PATCHES_PER_WAVELENGTH: f64 = 48.0;

/// A terrain of a biome's ground, with its rules and the noise its patches are cut from.
struct Ground {
    terrain: u8,
    share: f32,
    slope: Option<(f32, f32)>,
    elevation: Option<(f32, f32)>,
    patches: Option<Fbm<Perlin>>,
}

impl Ground {
    fn new(ground: &LandGround, terrain: usize, land: &Land, salt: usize) -> Ground {
        Ground {
            terrain: terrain as u8,
            share: ground.share,
            slope: ground.slope.map(|slope| (slope.min.0, slope.max.0)),
            elevation: ground
                .elevation
                .map(|elevation| (elevation.min, elevation.max)),
            patches: (ground.share < 1.0).then(|| {
                Fbm::<Perlin>::new((land.seed as u32).wrapping_add(100 + salt as u32))
                    .set_octaves(3)
                    .set_frequency(PATCHES_PER_WAVELENGTH / f64::from(land.climate.wavelength.0))
            }),
        }
    }

    fn paints(&self, at: Pos<Meters>, elevation: f32, slope: f32) -> bool {
        let within = |range: Option<(f32, f32)>, value: f32| {
            range.is_none_or(|(min, max)| (min..=max).contains(&value))
        };
        within(self.slope, slope)
            && within(self.elevation, elevation)
            && self.patches.as_ref().is_none_or(|noise| {
                (noise.get([f64::from(at.x), f64::from(at.y)]) as f32 * 0.5 + 0.5) < self.share
            })
    }
}

/// The noise that moves each place's reading of the climate, so borders wander.
struct Border {
    temperature: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    blend: f32,
}

impl Border {
    fn new(land: &Land) -> Border {
        let seed = land.seed as u32;
        let wavelength = f64::from(land.climate.wavelength.0) / PATCHES_PER_WAVELENGTH;
        Border {
            temperature: Fbm::<Perlin>::new(seed.wrapping_add(8))
                .set_octaves(3)
                .set_frequency(1.0 / wavelength),
            moisture: Fbm::<Perlin>::new(seed.wrapping_add(9))
                .set_octaves(3)
                .set_frequency(1.0 / wavelength),
            blend: land.climate.blend,
        }
    }

    fn jostled(&self, temperature: f32, moisture: f32, at: Pos<Meters>) -> (f32, f32) {
        let reading = [f64::from(at.x), f64::from(at.y)];
        (
            temperature + self.temperature.get(reading) as f32 * self.blend,
            moisture + self.moisture.get(reading) as f32 * self.blend,
        )
    }
}

/// How steep the ground at `at` is, across the samples a metre either side: the ground as it is
/// laid, so a slope's edge follows the ground's own shape.
fn ground_slope(heights: &Heightfield, at: Pos<Meters>) -> f32 {
    let height = |dx: f32, dy: f32| heights.height_at(Pos::new(at.x + dx, at.y + dy)).0;
    let across = (height(1.0, 0.0) - height(-1.0, 0.0)) / 2.0;
    let down = (height(0.0, 1.0) - height(0.0, -1.0)) / 2.0;
    across.hypot(down).atan().to_degrees()
}

/// Each of `values` as the share of them below it.
fn shares(values: &[f32]) -> Vec<f32> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.par_sort_unstable_by(|&a, &b| values[a].total_cmp(&values[b]));
    let last = (values.len().max(2) - 1) as f32;
    let mut shares = vec![0.0; values.len()];
    for (rank, index) in order.into_iter().enumerate() {
        shares[index] = rank as f32 / last;
    }
    shares
}
