use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_kira_audio::prelude::AudioSource;
use bevy_replication::AppReplicationExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::{HushVoice, PlayClip, SfxPlace, SfxTune};
use crate::core::content::AudioRef;
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ContentRegistry, ErrorPlace,
    FileWriter, MISSING_FILE, Nearby, PartSpec, Parts, RowCheck, Shape,
};
use crate::core::math::Rng;
use crate::core::text::{
    LettersPerSecond, StyledLine, TextPiece, TextVoice, Typewriter, TypewriterReveal,
    advance_typewriters, reveal_times,
};
use crate::core::units::{Hertz, PlaybackRate, Seconds};

pub type BabbleId = ContentId<BabbleDef>;
pub type BabbleBankId = ContentId<BabbleBankDef>;

pub const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
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

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct BabbleDef {
    pub babble_bank: BabbleBankId,
    pub pitch: PlaybackRate,
    pub range: Semitones,
    pub pace: Hertz,
}

impl ContentFile for BabbleDef {
    const TABLE: &'static str = "babble";

    fn fresh(near: &Nearby) -> Result<BabbleDef, String> {
        Ok(BabbleDef {
            babble_bank: near.file()?,
            pitch: PlaybackRate(1.0),
            range: Semitones(1.0),
            pace: Hertz(11.0),
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct BabbleBankDef {
    /// The bundle's folder of one clip per letter.
    #[serde(skip)]
    pub letters: String,
    pub volume: f32,
}

pub const BABBLE_LETTERS: PartSpec = PartSpec::Keyed {
    folder: "letters",
    exts: &["wav"],
};

impl ContentFile for BabbleBankDef {
    const TABLE: &'static str = "babble_bank";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(_: &Nearby) -> Result<BabbleBankDef, String> {
        Ok(BabbleBankDef {
            letters: String::new(),
            volume: 1.0,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.range("/volume", self.volume, 0.0, 4.0, "a volume");
    }
}

impl Bundle for BabbleBankDef {
    const PARTS: &'static [PartSpec] = &[BABBLE_LETTERS];

    fn read_parts(&mut self, parts: &mut Parts) {
        self.letters = parts.path("letters");
        for (key, part) in parts.keyed(BABBLE_LETTERS) {
            let letter = key.len() == 1 && LETTERS.contains(key);
            if !letter {
                parts.error(
                    part,
                    ErrorPlace::WholeFile,
                    ContentProblem::new("babble letter", key, "a clip named by one letter, a to z"),
                );
            }
        }
        let missing: Vec<char> = LETTERS
            .chars()
            .filter(|letter| !parts.has(&BABBLE_LETTERS.named(&letter.to_string(), "wav")))
            .collect();
        if !missing.is_empty() {
            let missing: String = missing.into_iter().collect();
            parts.error(
                "",
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    MISSING_FILE,
                    format!("no clip for {missing}"),
                    "letters/a.wav to letters/z.wav in the bundle",
                ),
            );
        }
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn content(registry: &mut ContentRegistry) {
    registry.file::<BabbleDef>().bundle::<BabbleBankDef>();
}

#[derive(
    JsonSchema, Clone, Copy, Debug, PartialEq, PartialOrd, derive_more::Add, Serialize, Deserialize,
)]
pub struct Semitones(pub f32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Babbler(pub BabbleId);

#[derive(Component, Clone, Copy, Debug)]
pub struct Babbling {
    pub babble: BabbleId,
    pub place: SfxPlace,
    pub rank: BabbleRank,
}

/// The voice that babbles when several speak at once: the foremost one, else the highest rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BabbleRank {
    Ranked(u32),
    Foremost,
}

pub fn register(app: &mut App) {
    app.replicate_once::<Babbler>();
}

pub struct BabblePlugin;

impl Plugin for BabblePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BabbleClips>().add_systems(
            Update,
            (
                load_clips.run_if(resource_exists_and_changed::<Content>),
                babble
                    .run_if(resource_exists::<Content>)
                    .after(advance_typewriters),
            )
                .chain(),
        );
    }
}

pub fn letter_files(bank: &BabbleBankDef) -> impl Iterator<Item = AudioRef> + '_ {
    LETTERS
        .chars()
        .map(move |letter| AudioRef::new(format!("{}/{letter}.wav", bank.letters)))
}

#[derive(Resource, Default)]
struct BabbleClips(Vec<Vec<Handle<AudioSource>>>);

impl BabbleClips {
    fn letter(&self, bank: BabbleBankId, letter: char) -> Option<Handle<AudioSource>> {
        let index = LETTERS.find(letter)?;
        self.0.get(bank.index())?.get(index).cloned()
    }
}

fn load_clips(assets: Res<AssetServer>, content: Res<Content>, mut clips: ResMut<BabbleClips>) {
    let table = content.table::<BabbleBankDef>();
    clips.0 = table
        .ids()
        .map(|id| {
            letter_files(table.row(id))
                .map(|file| assets.load(file.path()))
                .collect()
        })
        .collect();
}

/// A line is voiced whole the moment it starts: every letter that will sound is handed to the
/// audio engine at once, each timed to its reveal moment on the line's own clock, so a line
/// babbles the same way every time it is spoken, whatever the frame rate. A line cut short before
/// its last letter is due is hushed.
#[derive(Default)]
struct Mouth {
    speaker: Option<Entity>,
    ends_at: Duration,
}

#[derive(SystemParam)]
struct Voice<'w> {
    clips: Res<'w, BabbleClips>,
    sounds: MessageWriter<'w, PlayClip>,
    hushes: MessageWriter<'w, HushVoice>,
}

fn babble(
    content: Res<Content>,
    mut revealed: MessageReader<TypewriterReveal>,
    typing: Query<(Entity, &Babbling, &Typewriter, &StyledLine)>,
    speed: Res<LettersPerSecond>,
    time: Res<Time>,
    mut mouth: Local<Mouth>,
    mut voice: Voice,
) {
    let heard = typing
        .iter()
        .filter(|(_, _, typewriter, _)| !typewriter.is_finished())
        .max_by_key(|(entity, babbling, ..)| (babbling.rank, mouth.speaker == Some(*entity)));
    if let Some(speaker) = mouth.speaker
        && heard.is_none_or(|(entity, ..)| entity != speaker)
    {
        if time.elapsed() < mouth.ends_at {
            voice.hushes.write(HushVoice);
        }
        mouth.speaker = None;
    }
    let Some((speaker, babbling, typewriter, text)) = heard else {
        return;
    };
    if mouth.speaker == Some(speaker) {
        return;
    }
    let moments = reveal_times(&text.pieces, *speed);
    let first_shown = revealed
        .read()
        .filter(|reveal| reveal.entity == speaker)
        .map(|reveal| reveal.index)
        .min()
        .and_then(|index| moments.get(index).copied());
    let babble = babbling.babble.get(&content);
    let babble_bank = babble.babble_bank.get(&content);
    let now = time.elapsed();
    let mut ends_at = now;
    let mut busy_until = first_shown.unwrap_or(typewriter.elapsed());
    for (index, moment) in moments.iter().copied().enumerate() {
        if moment < busy_until {
            continue;
        }
        let Some(syllable) = Syllable::at(text, index) else {
            continue;
        };
        let Some(letter) = syllable.letter else {
            busy_until = moment + Duration::from_secs_f32(syllable.pause.0);
            continue;
        };
        busy_until = moment + babble.pace.period();
        let Some(clip) = voice.clips.letter(babble.babble_bank, letter) else {
            continue;
        };
        ends_at = now + moment.saturating_sub(typewriter.elapsed());
        voice.sounds.write(PlayClip {
            clip,
            category: AudioCategory::Voice,
            place: babbling.place,
            tune: SfxTune {
                pitch: babble.pitch_for(&syllable),
                volume: babble_bank.volume * syllable.loudness,
            },
            due: ends_at,
        });
    }
    *mouth = Mouth {
        speaker: Some(speaker),
        ends_at,
    };
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
    voice: Option<TextVoice>,
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
    fn at(text: &StyledLine, index: usize) -> Option<Syllable> {
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
            Some(TextVoice::Shout) => {
                intonation = intonation + SHOUT_LIFT;
                loudness *= SHOUT_LOUDNESS;
            }
            Some(TextVoice::Whisper) => {
                intonation = intonation + WHISPER_DROP;
                loudness *= WHISPER_LOUDNESS;
            }
            None => {}
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

fn spoken(text: &StyledLine) -> Vec<Spoken> {
    text.pieces
        .iter()
        .flat_map(|piece| match piece {
            TextPiece::Span(span) => span
                .text
                .chars()
                .map(|ch| Spoken {
                    glyph: Glyph::Char(ch),
                    voice: span.voice,
                })
                .collect(),
            TextPiece::Key(_) => vec![Spoken {
                glyph: Glyph::Input,
                voice: None,
            }],
            TextPiece::Pause(_) => Vec::new(),
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
