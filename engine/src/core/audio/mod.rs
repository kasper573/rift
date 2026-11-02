pub mod loader;
pub mod mix;
pub mod output;
pub mod playback;
pub mod soundscape;

use bevy::prelude::Resource;
use kira::Decibels;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::units::{Meters, Seconds};

/// How the game is heard: a sound in the world out to `wide` either side of the listener and
/// `deep` ahead and behind, panned across the width; a sound asked for again within
/// `repeats_after` of its last start not doubled; and the soundscape gliding from one mix to the
/// next over `glide` at the quickest, so a jump such as a trip between areas fades rather than cuts.
#[derive(JsonSchema, Resource, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hearing {
    pub wide: Meters,
    pub deep: Meters,
    pub repeats_after: Seconds,
    pub glide: Seconds,
}

pub fn decibels(amplitude: f32) -> Decibels {
    Decibels(20.0 * amplitude.max(1e-4).log10())
}
