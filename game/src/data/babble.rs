use crate::core::babble::{BabbleBankId, BabbleDef, Semitones};
use crate::core::time::{Hertz, PlaybackRate};

crate::table! {
    OrcChief: BabbleDef { babble_bank: BabbleBankId::Gruff, pitch: PlaybackRate(0.75), range: Semitones(1.0), pace: Hertz(11.0) },
    Grisha: BabbleDef { babble_bank: BabbleBankId::Gruff, pitch: PlaybackRate(0.85), range: Semitones(1.5), pace: Hertz(12.0) },
    Ugra: BabbleDef { babble_bank: BabbleBankId::Gruff, pitch: PlaybackRate(0.90), range: Semitones(2.0), pace: Hertz(13.0) },
    Bram: BabbleDef { babble_bank: BabbleBankId::Gruff, pitch: PlaybackRate(0.95), range: Semitones(1.5), pace: Hertz(12.0) },
    Tobb: BabbleDef { babble_bank: BabbleBankId::Soft, pitch: PlaybackRate(1.00), range: Semitones(3.0), pace: Hertz(14.0) },
    Mara: BabbleDef { babble_bank: BabbleBankId::Soft, pitch: PlaybackRate(1.05), range: Semitones(2.5), pace: Hertz(13.0) },
    Ilsa: BabbleDef { babble_bank: BabbleBankId::Soft, pitch: PlaybackRate(1.15), range: Semitones(3.0), pace: Hertz(15.0) },
    Pell: BabbleDef { babble_bank: BabbleBankId::Soft, pitch: PlaybackRate(1.25), range: Semitones(4.0), pace: Hertz(16.0) },
    Wren: BabbleDef { babble_bank: BabbleBankId::Soft, pitch: PlaybackRate(1.35), range: Semitones(4.0), pace: Hertz(17.0) },
}
