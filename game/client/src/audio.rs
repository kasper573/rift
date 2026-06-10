//! Sound output. Each platform uses its native-best engine behind one `play(index, ...)`: native
//! drives `kira`, the browser drives the Web Audio plugin in `js/rift_audio.js` (mirroring the
//! `js/rift_ws.js` transport plugin). `index` is the row in `world::sfx::sfx_table`.

use world::sfx::SfxDef;

pub struct Audio(Backend);

impl Audio {
    pub fn load(table: &'static [SfxDef]) -> Audio {
        Audio(Backend::load(table))
    }

    /// Play one shot of table row `index`: `volume` is a linear gain, `pitch` a playback-rate
    /// factor (1.0 = normal), `pan` is -1 (left) .. 1 (right).
    pub fn play(&mut self, index: usize, volume: f32, pitch: f32, pan: f32) {
        self.0.play(index, volume, pitch, pan);
    }
}

#[cfg(not(target_arch = "wasm32"))]
use native::Backend;
#[cfg(target_arch = "wasm32")]
use web::Backend;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Cursor;

    use kira::sound::static_sound::StaticSoundData;
    use kira::{AudioManager, AudioManagerSettings, Decibels, Panning, backend::DefaultBackend};
    use world::assets;
    use world::sfx::SfxDef;

    // `manager` is None when there is no output device (e.g. a headless host): the game runs
    // silently rather than failing to start.
    pub struct Backend {
        manager: Option<AudioManager<DefaultBackend>>,
        sounds: Vec<Option<StaticSoundData>>,
    }

    impl Backend {
        pub fn load(table: &'static [SfxDef]) -> Backend {
            let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok();
            let sounds = if manager.is_some() {
                table
                    .iter()
                    .map(|def| {
                        let bytes = assets::bytes(&def.src)?;
                        StaticSoundData::from_cursor(Cursor::new(bytes.to_vec())).ok()
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Backend { manager, sounds }
        }

        pub fn play(&mut self, index: usize, volume: f32, pitch: f32, pan: f32) {
            let Some(manager) = &mut self.manager else {
                return;
            };
            let Some(Some(sound)) = self.sounds.get(index) else {
                return;
            };
            // kira gains are decibels; ours is a linear amplitude.
            let decibels = Decibels(20.0 * volume.max(1e-4).log10());
            let _ = manager.play(
                sound
                    .volume(decibels)
                    .playback_rate(pitch as f64)
                    .panning(Panning::from(pan)),
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use world::assets;
    use world::sfx::SfxDef;

    // The contract with js/rift_audio.js (appended to mq_js_bundle.js at staging). Args are u32/f32
    // (JS Numbers); `index` keys the same sfx-table row on both sides.
    unsafe extern "C" {
        fn rift_audio_load(index: u32, pointer: *const u8, length: u32);
        fn rift_audio_play(index: u32, volume: f32, pitch: f32, pan: f32);
    }

    pub struct Backend;

    impl Backend {
        pub fn load(table: &'static [SfxDef]) -> Backend {
            for (index, def) in table.iter().enumerate() {
                if let Some(bytes) = assets::bytes(&def.src) {
                    unsafe { rift_audio_load(index as u32, bytes.as_ptr(), bytes.len() as u32) };
                }
            }
            Backend
        }

        pub fn play(&mut self, index: usize, volume: f32, pitch: f32, pan: f32) {
            unsafe { rift_audio_play(index as u32, volume, pitch, pan) };
        }
    }
}
