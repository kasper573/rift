pub mod mix;
pub mod playback;
pub mod soundscape;

use bevy_kira_audio::prelude::Decibels;

fn decibels(amplitude: f32) -> Decibels {
    Decibels(20.0 * amplitude.max(1e-4).log10())
}
