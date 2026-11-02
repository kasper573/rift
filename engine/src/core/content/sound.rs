use std::io::Cursor;

use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::StreamingSoundData;

use crate::core::units::Seconds;

/// The sound of `encoded`, decoded whole.
pub fn decode(encoded: &[u8]) -> Result<StaticSoundData, FromFileError> {
    StaticSoundData::from_cursor(Cursor::new(encoded))
}

/// How long the sound of `encoded` plays, read from its header alone.
pub fn length(encoded: &[u8]) -> Result<Seconds, FromFileError> {
    let sound = StreamingSoundData::from_cursor(Cursor::new(encoded.to_vec()))?;
    Ok(Seconds(sound.duration().as_secs_f32()))
}
