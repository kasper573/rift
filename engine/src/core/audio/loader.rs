use std::io::{self, Cursor};
use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::StreamingSoundData;

use crate::core::content::{AssetKind, sound};
use crate::core::units::Seconds;

/// A sound decoded whole, to start the moment it is asked for: an effect's or a voice's clip.
#[derive(Asset, TypePath, Clone)]
pub struct SoundClip(pub StaticSoundData);

/// A sound kept as its file, decoded only as it plays: a soundscape's long tracks.
#[derive(Asset, TypePath, Clone)]
pub struct SoundTrack {
    encoded: Arc<[u8]>,
    pub lasts: Seconds,
}

impl SoundTrack {
    pub fn stream(&self) -> Result<StreamingSoundData<FromFileError>, FromFileError> {
        StreamingSoundData::from_cursor(Cursor::new(self.encoded.clone()))
    }
}

#[derive(TypePath)]
pub struct ClipLoader;

impl AssetLoader for ClipLoader {
    type Asset = SoundClip;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<SoundClip, io::Error> {
        let mut encoded = Vec::new();
        reader.read_to_end(&mut encoded).await?;
        let clip = sound::decode(&encoded).map_err(io::Error::other)?;
        Ok(SoundClip(clip))
    }

    fn extensions(&self) -> &[&str] {
        AssetKind::Audio.extensions()
    }
}

#[derive(TypePath)]
pub struct TrackLoader;

impl AssetLoader for TrackLoader {
    type Asset = SoundTrack;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<SoundTrack, io::Error> {
        let mut encoded = Vec::new();
        reader.read_to_end(&mut encoded).await?;
        let encoded: Arc<[u8]> = encoded.into();
        let lasts = sound::length(encoded.clone()).map_err(io::Error::other)?;
        Ok(SoundTrack { encoded, lasts })
    }

    fn extensions(&self) -> &[&str] {
        AssetKind::Audio.extensions()
    }
}
