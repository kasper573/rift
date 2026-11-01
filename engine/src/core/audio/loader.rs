use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use bevy_kira_audio::AudioSource;
use symphonia::core::errors::Error;

use crate::core::content::AssetKind;
use crate::core::content::sound::decode;

/// Reads a sound whole.
#[derive(TypePath)]
pub struct AudioLoader;

impl AssetLoader for AudioLoader {
    type Asset = AudioSource;
    type Settings = ();
    type Error = Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<AudioSource, Error> {
        let mut encoded = Vec::new();
        reader.read_to_end(&mut encoded).await?;
        Ok(AudioSource {
            sound: decode(encoded)?.into_sound()?,
        })
    }

    fn extensions(&self) -> &[&str] {
        AssetKind::Audio.extensions()
    }
}
