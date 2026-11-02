//! An area laid out as a land is generated: ground shaped from warped and ridged noise and worn by
//! rain, a climate over it that cools with height and grows wet where the rain gathers, and biomes
//! placed by that climate with borders that blend, each painting its ground, growing its props and
//! roaming its monsters by its own rules, so a game has a wide and varied land to fill in.
//!
//! Everything laid out goes into one folder of the area's scene, which a later run replaces; the
//! rest of the scene, such as the roads and warps a designer added, stays.

mod biomes;
mod place;
mod relief;

use std::path::Path;

use engine::core::content::ContentFile;
use engine::core::content::fresh::Nearby;
use engine::core::ground::{GroundSize, HeightRange};
use engine::core::units::{Degrees, Meters};
use engine::systems::area::AreaDef;
use engine::systems::area::prop::ScaleRange;
use engine::systems::area::scene::SceneNode;
use serde::Deserialize;

use crate::folder;
use biomes::{Biomes, Climate};
use relief::Relief;

/// How an area is laid out, read from a JSON file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Land {
    /// The area's name.
    pub name: String,
    /// The area's width, in metres.
    pub width: u32,
    /// The area's height, in metres.
    pub height: u32,
    /// What the noise and the rain are drawn from: the same seed lays out the same land.
    pub seed: u64,
    /// How far the ground rises from its lowest point to its highest.
    pub relief: Meters,
    pub shape: LandShape,
    pub erosion: LandErosion,
    pub climate: LandClimate,
    /// The kinds of country the land is made of. Each lies where the climate is nearest its own.
    pub biomes: Vec<LandBiome>,
}

/// How the ground rises and falls.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandShape {
    /// How wide the hills and valleys are: about this far apart.
    pub wavelength: Meters,
    /// How far the ground is bent before its shape is read, in wavelengths: ridges and valleys
    /// wander and fork rather than repeat.
    pub warp: f32,
    /// How much of the land rises in sharp-ridged mountain ranges rather than rolling hills, from
    /// 0 to 1.
    pub mountains: f32,
    /// How high the bumps of the ground underfoot rise, a few metres across.
    pub roughness: Meters,
}

/// How the rain wears the ground: each drop runs downhill, carving where it runs fast and
/// leaving silt where it slows, so valleys deepen into gullies and fans spread where they open.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandErosion {
    /// How many drops fall on each square metre.
    pub drops_per_square_meter: f32,
}

/// How warm and how wet the land is, place by place. Both are told as a share of the land: a
/// temperature of 0.25 is warmer than a quarter of the land and colder than the rest.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandClimate {
    /// How wide the warm and cold, wet and dry regions are.
    pub wavelength: Meters,
    /// How much colder the highest ground is than the lowest, on the scale from the coldest
    /// region to the warmest.
    pub height_chill: f32,
    /// How much wetter the ground is where the rain runs together, on the scale from the driest
    /// region to the wettest.
    pub runoff_wetness: f32,
    /// How far biomes reach into one another where they meet, in shares of the climate.
    pub blend: f32,
}

/// A kind of country: where on the climate it lies, its ground, its props and its monsters.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandBiome {
    pub name: String,
    /// The share of the land colder than its own ground, from 0 to 1.
    pub temperature: f32,
    /// The share of the land drier than its own ground, from 0 to 1.
    pub moisture: f32,
    /// The terrains that paint its ground, the first whose rules hold at a place painting it; the
    /// last paints wherever none does.
    pub ground: Vec<LandGround>,
    #[serde(default)]
    pub props: Vec<LandProp>,
    #[serde(default)]
    pub monsters: Vec<LandMonster>,
}

/// A terrain of a biome's ground, painted in patches over `share` of the ground its rules allow.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandGround {
    pub terrain: String,
    #[serde(default = "everywhere")]
    pub share: f32,
    /// How steep the ground it paints is.
    #[serde(default)]
    pub slope: Option<Between<Degrees>>,
    /// How high the ground it paints is, as a share of the relief from the lowest ground.
    #[serde(default)]
    pub elevation: Option<Between<f32>>,
}

/// Props grown over a biome, any of `props` alike, `per_square_meter` of the ground it can be
/// walked to, their copies scaled over `scale`; with `clumping` they gather in thickets and leave
/// clearings between.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandProp {
    pub props: Vec<String>,
    pub per_square_meter: f32,
    #[serde(default)]
    pub scale: ScaleRange,
    /// From 0, as even as the ground allows, to 1, all in clumps.
    #[serde(default)]
    pub clumping: f32,
}

/// An npc roaming a biome, `per_square_kilometer` of it.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LandMonster {
    pub npc: String,
    pub per_square_kilometer: f32,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Between<T> {
    pub min: T,
    pub max: T,
}

/// The land the JSON file at `path` lays out.
pub fn read_land(path: &Path) -> Result<Land, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{} doesn't read: {error}", path.display()))?;
    let land: Land =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    land.check()
        .map_err(|problem| format!("{}: {problem}", path.display()))?;
    Ok(land)
}

/// Lays out the area at `area_path` in the game folder `game` as `land` asks, keeping what of its
/// scene the land didn't lay out.
pub fn run(game: &Path, area_path: &str, land: &Land) -> Result<(), String> {
    let reader = folder::reader_of(game);
    let mut area = if game.join(area_path).exists() {
        reader
            .read::<AreaDef>(area_path)
            .map_err(|errors| format!("{area_path} doesn't read: {} problem(s)", errors.len()))?
    } else {
        AreaDef::fresh(&Nearby::new(&reader, area_path))?
    };
    let biomes = Biomes::resolve(land, &reader)?;
    area.name = land.name.clone();
    area.size = GroundSize {
        width: land.width,
        height: land.height,
    };
    area.height_range = HeightRange {
        lowest: Meters(0.0),
        highest: land.relief,
    };
    area.terrains = biomes.terrains().to_vec();

    let mut relief = Relief::shaped(area.size, land);
    relief.erode(land);
    let climate = Climate::over(&relief, land);
    area.heights = relief.heightfield(area.size, area.height_range, land);
    area.paint = biomes.paint(area.size, &relief, &climate, &area.heights, land);
    let laid = place::lay_out(area.size, &relief, &climate, &biomes, area.max_slope, land);
    area.scene
        .retain(|node| !matches!(node, SceneNode::Folder(folder) if folder.label == LAID_OUT));
    area.scene.push(laid);
    folder::write(game, reader.write(area_path, &area))
}

/// The label of the scene folder holding all a run lays out.
pub(super) const LAID_OUT: &str = "Landscape";

fn everywhere() -> f32 {
    1.0
}

impl Land {
    fn check(&self) -> Result<(), String> {
        let share = |value: f32, what: &str| {
            (0.0..=1.0)
                .contains(&value)
                .then_some(())
                .ok_or_else(|| format!("{what} is {value}, outside 0 to 1"))
        };
        if self.relief.0 <= 0.0 {
            return Err(format!("a relief of {} m lays out no land", self.relief.0));
        }
        if self.shape.wavelength.0 <= 0.0 || self.climate.wavelength.0 <= 0.0 {
            return Err("every wavelength is longer than nothing".to_owned());
        }
        if self.climate.blend <= 0.0 {
            return Err("biomes blend over more than nothing".to_owned());
        }
        share(self.shape.mountains, "the share of mountains")?;
        share(self.climate.height_chill, "the height's chill")?;
        share(self.climate.runoff_wetness, "the runoff's wetness")?;
        if self.biomes.is_empty() {
            return Err("a land is made of at least one biome".to_owned());
        }
        for biome in &self.biomes {
            share(biome.temperature, &format!("{}'s temperature", biome.name))?;
            share(biome.moisture, &format!("{}'s moisture", biome.name))?;
            if biome.ground.is_empty() {
                return Err(format!("{} paints its ground with no terrain", biome.name));
            }
            for ground in &biome.ground {
                share(
                    ground.share,
                    &format!("{}'s share in {}", ground.terrain, biome.name),
                )?;
            }
            for prop in &biome.props {
                if prop.props.is_empty() {
                    return Err(format!("{} grows a scatter of no props", biome.name));
                }
                share(
                    prop.clumping,
                    &format!("{}'s clumping", prop.props.join(", ")),
                )?;
            }
        }
        Ok(())
    }
}
