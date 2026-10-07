use bevy::prelude::*;
use bevy_kira_audio::prelude::AudioSource;
use serde::{Deserialize, Serialize};
use ui::{RichPiece, RichText, TextVoice, Typewriter, TypewriterReveal};

use crate::core::assets::AssetRef;
use crate::core::math::Rng;
use crate::core::sfx::AudioCategory;
use crate::core::sfx::playback::{PlayClip, SfxPlace, SfxTune};
use crate::core::time::{Hertz, PlaybackRate, Seconds};

pub use crate::data::babble::Id as BabbleId;
pub use crate::data::babble_bank::Id as BabbleBankId;

const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
const VOWELS: [char; 5] = ['a', 'e', 'i', 'o', 'u'];
const WORD_GAP: Seconds = Seconds(0.05);
const CLAUSE_GAP: Seconds = Seconds(0.12);
const SENTENCE_GAP: Seconds = Seconds(0.24);
const QUESTION_RISE: Semitones = Semitones(4.0);
const EXCLAIMED_LIFT: Semitones = Semitones(1.5);
const STATEMENT_FALL: Semitones = Semitones(-1.5);
const SHOUT_LIFT: Semitones = Semitones(1.0);
const WHISPER_DROP: Semitones = Semitones(-1.0);
const STEPS: u32 = 9;
const EXCLAIMED_LOUDNESS: f32 = 1.2;
const SHOUT_LOUDNESS: f32 = 1.35;
const WHISPER_LOUDNESS: f32 = 0.6;

pub struct BabbleDef {
    pub babble_bank: BabbleBankId,
    pub pitch: PlaybackRate,
    pub range: Semitones,
    pub pace: Hertz,
}

pub struct BabbleBankDef {
    pub letters: AssetRef,
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, derive_more::Add)]
pub struct Semitones(pub f32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Babbler(pub BabbleId);

#[derive(Component, Clone, Copy, Debug)]
pub struct Babbling {
    pub babble: BabbleId,
    pub place: SfxPlace,
    pub rank: BabbleRank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BabbleRank(pub u32);

impl BabbleRank {
    pub const DIALOGUE_BOX: BabbleRank = BabbleRank(u32::MAX);
}

pub fn register(app: &mut bevy_app::App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Babbler>();
}

pub struct BabblePlugin;

impl Plugin for BabblePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BabbleClips>()
            .add_systems(Startup, load_clips)
            .add_systems(Update, babble);
    }
}

pub fn letter_clips(bank: &BabbleBankDef) -> impl Iterator<Item = String> + '_ {
    LETTERS
        .chars()
        .map(move |letter| format!("{}/{letter}.wav", bank.letters.0))
}

#[derive(Resource, Default)]
struct BabbleClips(Vec<Vec<Handle<AudioSource>>>);

impl BabbleClips {
    fn letter(&self, bank: BabbleBankId, letter: char) -> Option<Handle<AudioSource>> {
        let index = LETTERS.find(letter)?;
        self.0.get(bank.index())?.get(index).cloned()
    }
}

fn load_clips(assets: Res<AssetServer>, mut clips: ResMut<BabbleClips>) {
    clips.0 = BabbleBankId::VARIANTS
        .iter()
        .map(|bank| {
            letter_clips(bank.get())
                .map(|path| assets.load(path))
                .collect()
        })
        .collect();
}

fn babble(
    mut revealed: MessageReader<TypewriterReveal>,
    typing: Query<(Entity, &Babbling, &Typewriter, &RichText)>,
    clips: Res<BabbleClips>,
    time: Res<Time>,
    mut free_at: Local<Seconds>,
    mut sounds: MessageWriter<PlayClip>,
) {
    let reveals: Vec<TypewriterReveal> = revealed.read().copied().collect();
    let heard = typing
        .iter()
        .filter(|(entity, _, typewriter, _)| {
            !typewriter.is_finished() || reveals.iter().any(|reveal| reveal.entity == *entity)
        })
        .max_by_key(|(_, babbling, ..)| babbling.rank);
    let Some((speaker, babbling, _, text)) = heard else {
        return;
    };
    let babble = babbling.babble.get();
    let babble_bank = babble.babble_bank.get();
    let now = Seconds(time.elapsed_secs());
    for reveal in reveals.iter().filter(|reveal| reveal.entity == speaker) {
        if now < *free_at {
            continue;
        }
        let Some(syllable) = Syllable::at(text, reveal.index) else {
            continue;
        };
        let Some(letter) = syllable.letter else {
            *free_at = now + syllable.pause;
            continue;
        };
        *free_at = now + Seconds(babble.pace.period().as_secs_f32());
        let Some(clip) = clips.letter(babble.babble_bank, letter) else {
            continue;
        };
        sounds.write(PlayClip {
            clip,
            category: AudioCategory::Voice,
            place: babbling.place,
            tune: SfxTune {
                pitch: babble.pitch_for(&syllable),
                volume: babble_bank.volume * syllable.loudness,
            },
        });
    }
}

impl BabbleDef {
    fn pitch_for(&self, syllable: &Syllable) -> PlaybackRate {
        let step = Semitones(self.range.0 * syllable.step);
        PlaybackRate(self.pitch.0 * (step + syllable.intonation).ratio())
    }
}

impl Semitones {
    fn ratio(self) -> f32 {
        2f32.powf(self.0 / 12.0)
    }
}

struct Syllable {
    letter: Option<char>,
    pause: Seconds,
    step: f32,
    intonation: Semitones,
    loudness: f32,
}

#[derive(Clone, Copy)]
struct Spoken {
    glyph: Glyph,
    voice: TextVoice,
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Char(char),
    Input,
}

impl Glyph {
    fn in_word(self) -> bool {
        match self {
            Glyph::Char(ch) => ch.is_alphanumeric(),
            Glyph::Input => true,
        }
    }

    fn ends_sentence(self) -> bool {
        matches!(self, Glyph::Char('.' | '!' | '?'))
    }
}

impl Syllable {
    fn at(text: &RichText, index: usize) -> Option<Syllable> {
        let spoken = spoken(text);
        let here = *spoken.get(index)?;
        let word_start = spoken[..index]
            .iter()
            .rposition(|earlier| !earlier.glyph.in_word())
            .map_or(0, |gap| gap + 1);
        let sentence_end = spoken[index..]
            .iter()
            .position(|later| later.glyph.ends_sentence())
            .map(|offset| index + offset);
        let ending = sentence_end.map(|end| spoken[end].glyph);
        let last_word = sentence_end.is_some_and(|end| {
            spoken[index..end]
                .iter()
                .all(|between| between.glyph.in_word())
        });
        let word_length = spoken[word_start..]
            .iter()
            .take_while(|later| later.glyph.in_word())
            .count()
            .max(1);
        let through_word = (index - word_start + 1) as f32 / word_length as f32;
        let mut intonation = match (ending, last_word) {
            (Some(Glyph::Char('?')), true) => Semitones(QUESTION_RISE.0 * through_word),
            (Some(Glyph::Char('!')), _) => EXCLAIMED_LIFT,
            (Some(Glyph::Char('.')), true) => Semitones(STATEMENT_FALL.0 * through_word),
            _ => Semitones(0.0),
        };
        let mut loudness = match ending {
            Some(Glyph::Char('!')) => EXCLAIMED_LOUDNESS,
            _ => 1.0,
        };
        match here.voice {
            TextVoice::Shout => {
                intonation = intonation + SHOUT_LIFT;
                loudness *= SHOUT_LOUDNESS;
            }
            TextVoice::Whisper => {
                intonation = intonation + WHISPER_DROP;
                loudness *= WHISPER_LOUDNESS;
            }
            TextVoice::Normal => {}
        }
        let letter = letter_of(here.glyph);
        Some(Syllable {
            letter,
            pause: pause_after(here.glyph),
            step: letter.map_or(0.0, |letter| step(letter, index - word_start)),
            intonation,
            loudness,
        })
    }
}

fn spoken(text: &RichText) -> Vec<Spoken> {
    text.pieces
        .iter()
        .flat_map(|piece| match piece {
            RichPiece::Span(span) => span
                .text
                .chars()
                .map(|ch| Spoken {
                    glyph: Glyph::Char(ch),
                    voice: span.voice,
                })
                .collect(),
            RichPiece::Input(_) => vec![Spoken {
                glyph: Glyph::Input,
                voice: TextVoice::Normal,
            }],
            RichPiece::Pause(_) => Vec::new(),
        })
        .collect()
}

fn letter_of(glyph: Glyph) -> Option<char> {
    let Glyph::Char(ch) = glyph else {
        return Some(VOWELS[0]);
    };
    if ch.is_ascii_alphabetic() {
        return Some(ch.to_ascii_lowercase());
    }
    if let Some(digit) = ch.to_digit(10) {
        return Some(VOWELS[digit as usize % VOWELS.len()]);
    }
    ch.is_alphanumeric().then_some(VOWELS[0])
}

fn pause_after(glyph: Glyph) -> Seconds {
    match glyph {
        Glyph::Char('.' | '!' | '?') => SENTENCE_GAP,
        Glyph::Char(',' | ';' | ':' | '-' | '\u{2014}') => CLAUSE_GAP,
        _ => WORD_GAP,
    }
}

fn step(letter: char, place_in_word: usize) -> f32 {
    let seed = (u64::from(letter) << 32) | place_in_word as u64;
    let notch = Rng::new(seed).rand_range(0..STEPS);
    notch as f32 / ((STEPS - 1) as f32 / 2.0) - 1.0
}
