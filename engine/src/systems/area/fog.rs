//! Fog as it lies in an area: banks of thicker air placed over the ground, each fogging what is
//! seen through it by how much of it the eye looks through, and lit by the sky over it.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{ContentProblem, Loading, RowCheck, serial};
use crate::core::ground::{Heightfield, WaterSurface};
use crate::core::math::{Pos, Rect, Rgb};
use crate::core::render::world::SIGHT_LENGTHS;
use crate::core::render::world::haze::HazeBox;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::area::sky::SkyCondition;
use crate::systems::area::{AreaDef, AreaModule, placed};

/// A bank of fog lying over the ground within `rect`: thickest at the lowest ground it lies
/// over, thinning upward until `depth` over that ground little of it is left, and thinning out
/// over `edge` metres at its sides.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FogBank {
    #[schemars(with = "serial::RectRow")]
    #[serde(
        serialize_with = "serial::rect",
        deserialize_with = "serial::rect_from"
    )]
    pub rect: Rect<Meters>,
    /// How far something still shows through its thickest air, looking level.
    pub sight: Meters,
    pub depth: Meters,
    pub edge: Meters,
    /// How much nearer saturated air and a cloudburst round the eye close its sight in, each a
    /// share of the way.
    pub moist_closer: f32,
    pub rain_closer: f32,
    pub shafts: FogShafts,
    /// The light the fog sheds under a dark sky, which has none to give it. Unused under the
    /// air, whose sky lights the fog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Rgb>,
    /// How thick it lies under the sky, of how thick it gets; as thick as it gets without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<SkyCondition>,
}

/// How thick the low air the sun's shafts show in stands in a bank round the eye, in dry air and
/// in saturated: dry air thin, so the shafts show only where the light breaks through.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FogShafts {
    pub dry: f32,
    pub moist: f32,
}

impl FogShafts {
    /// How thick the shafts' air stands in air as moist as `humidity`.
    pub fn air(&self, humidity: f32) -> f32 {
        self.dry + (self.moist - self.dry) * humidity.clamp(0.0, 1.0)
    }
}

/// A bank as it lies in an area: over ground and water that lie nowhere lower than `floor`
/// within it.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidFog {
    pub bank: FogBank,
    pub floor: Meters,
}

impl FogBank {
    /// A thin mist over the whole of `rect`, as deep as the low air: what a fresh bank starts as.
    pub fn light_over(rect: Rect<Meters>) -> FogBank {
        FogBank {
            rect,
            sight: Meters(600.0),
            depth: Meters(40.0),
            edge: Meters(20.0),
            moist_closer: 0.5,
            rain_closer: 0.2,
            shafts: FogShafts {
                dry: 0.006,
                moist: 0.04,
            },
            glow: None,
            when: None,
        }
    }

    /// The bank laid over `ground` under `water`: its floor the lowest that either lies within
    /// it, as near as a walk across it at every `FLOOR_STEPS`th of its sides finds.
    pub fn laid(self, ground: &Heightfield, water: &WaterSurface) -> LaidFog {
        let low = self.rect.min();
        let size = self.rect.size;
        let floor = (0..=FLOOR_STEPS)
            .flat_map(|row| (0..=FLOOR_STEPS).map(move |column| (column, row)))
            .map(|(column, row)| {
                let at = Pos::new(
                    low.x + size.width * column as f32 / FLOOR_STEPS as f32,
                    low.y + size.height * row as f32 / FLOOR_STEPS as f32,
                );
                ground.height_at(at).0 + water.depth_at(ground, at).0
            })
            .fold(f32::INFINITY, f32::min);
        LaidFog {
            bank: self,
            floor: Meters(floor),
        }
    }

    /// How far something still shows through the bank's thickest air in air as moist as
    /// `humidity` while it `rains` round the eye (0 none to 1 a cloudburst): the moister and
    /// the harder it rains, the nearer.
    pub fn sight_in(&self, humidity: f32, rains: f32) -> Meters {
        let closer = (1.0 - self.moist_closer * humidity.clamp(0.0, 1.0))
            * (1.0 - self.rain_closer * rains.clamp(0.0, 1.0));
        Meters(self.sight.0 * closer)
    }

    /// How far from `at` (x east, y south) the bank lies over the ground, none inside it.
    pub fn distance(&self, at: Vec2) -> f32 {
        let low = self.rect.min();
        let high = self.rect.max();
        let nearest = at.clamp(Vec2::new(low.x, low.y), Vec2::new(high.x, high.y));
        at.distance(nearest)
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.at_least(
            format!("{pointer}/rect/w"),
            self.rect.size.width,
            0.5,
            "metres",
        );
        cx.at_least(
            format!("{pointer}/rect/h"),
            self.rect.size.height,
            0.5,
            "metres",
        );
        cx.at_least(format!("{pointer}/sight"), self.sight.0, 1.0, "metres");
        cx.range(
            format!("{pointer}/depth"),
            self.depth.0,
            0.5,
            2000.0,
            "metres deep over the lowest ground it lies over",
        );
        cx.at_least(format!("{pointer}/edge"), self.edge.0, 0.0, "metres");
        for (field, share) in [
            ("moist_closer", self.moist_closer),
            ("rain_closer", self.rain_closer),
        ] {
            cx.range(
                format!("{pointer}/{field}"),
                share,
                0.0,
                0.95,
                "of the way nearer",
            );
        }
        for (field, value) in [("dry", self.shafts.dry), ("moist", self.shafts.moist)] {
            cx.at_least(format!("{pointer}/shafts/{field}"), value, 0.0, "thick");
        }
    }
}

impl LaidFog {
    /// The bank as it fogs what is seen through it in air as moist as `humidity` while it
    /// `rains` round the eye, lying `share` as thick as it gets, a glow of full strength
    /// shining `white` candela per square metre.
    pub fn seen(&self, humidity: f32, rains: f32, share: f32, white: f32) -> HazeBox {
        let bank = &self.bank;
        let (low, high) = (bank.rect.min(), bank.rect.max());
        let glow = bank.glow.map_or(Vec3::ZERO, |glow| {
            glow.color().to_linear().to_vec3() * white
        });
        HazeBox {
            low: Vec2::new(low.x, low.y),
            high: Vec2::new(high.x, high.y),
            floor: self.floor.0,
            fall: bank.depth.0 / SIGHT_LENGTHS,
            thickest: SIGHT_LENGTHS / bank.sight_in(humidity, rains).0 * share,
            edge: bank.edge.0,
            glow,
        }
    }
}

/// Fog never hides what the game lets a player act on: a bank whose sight is shorter than the
/// game's own is refused. A bank under a dark sky that sheds no glow only dims what is seen
/// through it, which is rarely what was meant, and is warned of.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let sight = content.built::<GameSettings>().sight;
    for (id, area) in loading.rows::<AreaDef>(&content) {
        for module in placed(&content, id) {
            let AreaModule::Fog(bank) = &module.module else {
                continue;
            };
            let (file, pointer) = module.site(&content, id);
            if bank.sight.0 < sight.0 {
                loading.json_error(
                    &file,
                    &format!("{pointer}/sight"),
                    ContentProblem::new(
                        "fog thicker than the game's sight",
                        format!("{} metres", bank.sight.0),
                        format!("at least the game's sight, {} metres", sight.0),
                    ),
                );
            }
            if !area.lighting.sky.is_air() && bank.glow.is_none() {
                loading.json_warning(
                    &file,
                    &format!("{pointer}/glow"),
                    ContentProblem::new(
                        "fog without light",
                        "no glow under a dark sky",
                        "a glow, for the bank to show as more than a dimming",
                    ),
                );
            }
        }
    }
}

/// How finely a bank's rect is walked for its floor, along each side.
const FLOOR_STEPS: u32 = 48;
