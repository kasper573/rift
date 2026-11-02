use bevy::prelude::*;
use kira::sound::SoundData;
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use strum::VariantArray;

use super::decibels;
use super::mix::{AudioCategory, AudioFader, AudioMix};

/// The audio engine the app plays through, with a track of its own for each category, so a fader
/// turns its sounds up and down while they play. An app on a machine without a sound device has
/// none, and stays silent.
pub struct AudioOutput {
    manager: AudioManager,
    tracks: Vec<(AudioCategory, TrackHandle)>,
}

impl AudioOutput {
    pub fn plugin(app: &mut App) {
        match AudioOutput::open() {
            Ok(output) => {
                app.insert_non_send(output);
            }
            Err(error) => warn!("no sound: {error}"),
        }
        app.add_systems(PostUpdate, turn_faders);
    }

    fn open() -> Result<AudioOutput, String> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| error.to_string())?;
        let tracks = AudioCategory::VARIANTS
            .iter()
            .map(|&category| {
                let track = manager
                    .add_sub_track(TrackBuilder::new())
                    .map_err(|error| error.to_string())?;
                Ok((category, track))
            })
            .collect::<Result<_, String>>()?;
        Ok(AudioOutput { manager, tracks })
    }

    /// Starts `sound` on the track of `category`; none when the engine has no room for it.
    pub fn play<D: SoundData>(&mut self, category: AudioCategory, sound: D) -> Option<D::Handle> {
        self.track(category).play(sound).ok()
    }

    fn track(&mut self, category: AudioCategory) -> &mut TrackHandle {
        &mut self
            .tracks
            .iter_mut()
            .find(|(of, _)| *of == category)
            .expect("a track for every category")
            .1
    }
}

fn turn_faders(mix: Res<AudioMix>, output: Option<NonSendMut<AudioOutput>>) {
    let Some(mut output) = output else {
        return;
    };
    if !mix.is_changed() && !output.is_added() {
        return;
    }
    let master = mix.amplitude(AudioFader::Master);
    output
        .manager
        .main_track()
        .set_volume(decibels(master), Tween::default());
    for &category in AudioCategory::VARIANTS {
        let level = mix.amplitude(AudioFader::Category(category));
        output
            .track(category)
            .set_volume(decibels(level), Tween::default());
    }
}
