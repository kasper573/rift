//! The rings that spread over water from where its surface is broken. Whatever breaks it says
//! which rings it sends: rain, in an area's climate; a prop that stirs the water it stands in, as
//! a shoal of fish does; and, in the game's settings, whatever stands or floats in it at rest.
//! A water's shader draws them all alike, from what each tells here.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::RowCheck;
use crate::core::units::{Meters, Seconds};

/// The rings something sends over the water it breaks: each widens to `reach` over `lasts` and
/// is spent, a new one starts `every` so often, anywhere within `scatter` of where the water is
/// broken, and they are drawn `strength` strong, 0 to 1. With no `scatter` they all start at the
/// one place, ring within ring.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ripples {
    pub reach: Meters,
    pub lasts: Seconds,
    pub every: Seconds,
    #[serde(default, skip_serializing_if = "Meters::is_zero")]
    pub scatter: Meters,
    pub strength: f32,
}

/// How many rings from one place spread at once at most.
pub const MOST_RINGS: usize = 8;

/// Marks a thing standing in the world that keeps the water round it broken, with the rings it
/// sends.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RippleSpot(pub Ripples);

impl Ripples {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/reach"),
            self.reach.0,
            0.01,
            50.0,
            "meters",
        );
        cx.range(
            format!("{pointer}/lasts"),
            self.lasts.0,
            0.05,
            60.0,
            "seconds",
        );
        cx.range(
            format!("{pointer}/every"),
            self.every.0,
            self.lasts.0 / MOST_RINGS as f32,
            600.0,
            "seconds, no shorter than an eighth of `lasts`",
        );
        cx.range(
            format!("{pointer}/scatter"),
            self.scatter.0,
            0.0,
            50.0,
            "meters",
        );
        cx.range(
            format!("{pointer}/strength"),
            self.strength,
            0.0,
            1.0,
            "of a full ring",
        );
    }

    /// The rings as a shader reads them: how far they spread, for how long, how often one
    /// starts, and how far round the place.
    pub fn told(&self) -> Vec4 {
        Vec4::new(self.reach.0, self.lasts.0, self.every.0, self.scatter.0)
    }
}
