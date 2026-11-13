//! Fog as it lies in an area: banks of it placed over the ground, each hazing what is seen
//! through it by how much of it the eye looks through.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{RowCheck, serial};
use crate::core::math::Rect;
use crate::core::render::world::haze::HazeBox;
use crate::core::render::world::sky::Fog;
use crate::core::units::Meters;
use crate::systems::script::SkyCondition;

/// A bank of fog lying over the ground: over `rect`, standing `height` deep over the ground under
/// it, hazing what is seen through it as `haze` says.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FogBank {
    #[schemars(with = "serial::RectRow")]
    #[serde(
        serialize_with = "serial::rect",
        deserialize_with = "serial::rect_from"
    )]
    pub rect: Rect<Meters>,
    pub height: Meters,
    pub haze: Fog,
    /// How thick it lies under the sky, of how thick it gets; as thick as it gets without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<SkyCondition>,
}

impl FogBank {
    /// A light haze over the whole of `rect`, as deep as the low air: what a fresh area starts
    /// with.
    pub fn light_over(rect: Rect<Meters>) -> FogBank {
        FogBank {
            rect,
            height: Meters(40.0),
            haze: Fog {
                clear: Meters(50.0),
                thick: Meters(170.0),
                thickest: 0.85,
                moist_closer: 0.5,
                rain_closer: 0.2,
                dry_air: 0.006,
                moist_air: 0.04,
            },
            when: None,
        }
    }

    /// The bank as it hazes what is seen through it in air as moist as `humidity` while it
    /// `rains` round the eye, lying `share` as thick as it gets.
    pub fn seen(&self, humidity: f32, rains: f32, share: f32) -> HazeBox {
        let low = self.rect.min();
        let high = self.rect.max();
        let lying = self.haze.lying(
            Vec2::new(low.x, low.y),
            Vec2::new(high.x, high.y),
            self.height,
            humidity,
            rains,
        );
        HazeBox {
            thickest: lying.thickest * share,
            ..lying
        }
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
        cx.range(
            format!("{pointer}/height"),
            self.height.0,
            0.5,
            2000.0,
            "metres deep over the ground",
        );
        let haze = &self.haze;
        cx.range(
            format!("{pointer}/haze/clear"),
            haze.clear.0,
            0.0,
            haze.thick.0,
            "metres, up to where the haze is thickest",
        );
        cx.range(
            format!("{pointer}/haze/thickest"),
            haze.thickest,
            0.0,
            1.0,
            "of the way to the haze's colour",
        );
        for (field, share) in [
            ("moist_closer", haze.moist_closer),
            ("rain_closer", haze.rain_closer),
        ] {
            cx.range(
                format!("{pointer}/haze/{field}"),
                share,
                0.0,
                1.0,
                "of the way nearer",
            );
        }
        for (field, value) in [("dry_air", haze.dry_air), ("moist_air", haze.moist_air)] {
            cx.at_least(format!("{pointer}/haze/{field}"), value, 0.0, "thick");
        }
    }
}
