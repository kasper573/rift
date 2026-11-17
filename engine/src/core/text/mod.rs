mod paint;

pub use paint::{Hand, ShownLine, TextLook, measure, show, show_linked};

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::egui::Color32;
use ui::ink::Prose;
use ui::keys::KeyCap;

use crate::core::content::SchemaMark;
use crate::core::units::{Radians, RadiansPerSec};

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct StyledLine {
    pub pieces: Vec<TextPiece>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextPiece {
    Span(StyledSpan),
    Pause(Duration),
    Key(TextKey),
    /// A picture of the game's set in the line as tall as its letters, named by its file.
    Image(Cow<'static, str>),
    /// Opens a run of the line that stands for `key`, up to the next `LinkEnd`: drawing the line
    /// tells where the run landed, so whoever draws it can let the run be pointed at and clicked.
    LinkStart(Cow<'static, str>),
    LinkEnd,
    /// Room this many points wide before the next letter, which whoever draws the line paints
    /// into.
    Room(f32),
}

/// A key drawn as a cap in a line: an input as it is bound now, or a cap given outright.
#[derive(Clone, Debug, PartialEq)]
pub enum TextKey {
    Input(InputRef),
    Cap(KeyCap),
}

impl TextKey {
    /// The cap to draw, none for an input that isn't bound.
    pub fn cap<'a>(&'a self, keys: &'a KeyNames) -> Option<&'a KeyCap> {
        match self {
            TextKey::Input(input) => keys.0.get(input),
            TextKey::Cap(cap) => Some(cap),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyledSpan {
    pub text: Cow<'static, str>,
    pub ink: Option<SpanInk>,
    pub voice: Option<TextVoice>,
    pub motion: Option<TextMotion>,
    pub slow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanInk {
    Prose(Prose),
    Fixed(Color32),
    /// A colour of the game's own, which shows only where the ground is dark, lifted there until
    /// it reads. On paper, where most colours would not, the text keeps its ink.
    Lit(Color32),
    /// A colour of the game's own on either ground, darkened on paper and lifted on dusk until it
    /// reads.
    Legible(Color32),
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TextVoice {
    Whisper,
    Shout,
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TextMotion {
    Wave,
    Shake,
    Pulse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputRef(pub u16);

/// The cap every bound input is drawn as.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct KeyNames(pub HashMap<InputRef, KeyCap>);

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
#[schemars(transform = SchemaMark::Unit.with("characters/s"))]
pub struct CharactersPerSecond(pub f32);

/// How styled text sets its voices, key caps, halos and motions; sizes are shares of the text's
/// own, distances in points.
#[derive(JsonSchema, Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Lettering {
    pub whisper: WhisperLettering,
    pub shout_size: f32,
    pub caps: CapLettering,
    /// How far a halo spreads around each character.
    pub halo: f32,
    /// How a waving word bobs up and down.
    pub wave: Swing,
    pub shake: ShakeLettering,
    /// How a pulsing word swells, its reach a share of its size.
    pub pulse: Swing,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WhisperLettering {
    pub size: f32,
    pub opacity: f32,
}

/// A key cap's letters, and the room around the cap in ems of the text.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapLettering {
    pub size: f32,
    pub room: f32,
}

/// How a shaking word trembles across and down.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShakeLettering {
    pub across: Swing,
    pub down: Swing,
}

/// A swing to and fro, as far as `reach` either way, turning at `rate`, each word of a line
/// `step` further round than the one before.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Swing {
    pub rate: RadiansPerSec,
    pub step: Radians,
    pub reach: f32,
}

/// How lines type out in the game: at its pace unless a player chooses another between its
/// slowest and fastest, a slow span's characters each taking `slow_by` times as long.
#[derive(JsonSchema, Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Typing {
    pub pace: CharactersPerSecond,
    pub slowest: CharactersPerSecond,
    pub fastest: CharactersPerSecond,
    pub slow_by: f32,
}

impl Typing {
    /// The pace lines type at for a player who `chose` one, or chose none.
    pub fn pace_for(self, chose: Option<CharactersPerSecond>) -> TypingPace {
        let characters = chose.map_or(self.pace, |chose| {
            CharactersPerSecond(chose.0.clamp(self.slowest.0, self.fastest.0))
        });
        TypingPace {
            characters,
            slow_by: self.slow_by,
        }
    }
}

/// How fast lines type out where they are read.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TypingPace {
    pub characters: CharactersPerSecond,
    pub slow_by: f32,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Typewriter {
    elapsed: Duration,
    shown: usize,
    finished: bool,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct TypewriterReveal {
    pub entity: Entity,
    pub index: usize,
}

impl Typewriter {
    pub fn finish(&mut self) {
        self.finished = true;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn shown(&self) -> Option<usize> {
        (!self.finished).then_some(self.shown)
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

impl StyledLine {
    pub fn new(pieces: Vec<TextPiece>) -> StyledLine {
        StyledLine { pieces }
    }

    pub fn plain_text(text: impl Into<Cow<'static, str>>) -> StyledLine {
        StyledLine::new(vec![TextPiece::text(text)])
    }

    pub fn plain(&self, keys: &KeyNames) -> String {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                TextPiece::Span(span) => Some(span.text.as_ref()),
                TextPiece::Pause(_)
                | TextPiece::Image(_)
                | TextPiece::LinkStart(_)
                | TextPiece::LinkEnd
                | TextPiece::Room(_) => None,
                TextPiece::Key(key) => key.cap(keys).map(|cap| cap.name.as_str()),
            })
            .collect()
    }
}

impl TextPiece {
    pub fn text(text: impl Into<Cow<'static, str>>) -> TextPiece {
        TextPiece::Span(StyledSpan::plain(text))
    }

    pub fn key(cap: KeyCap) -> TextPiece {
        TextPiece::Key(TextKey::Cap(cap))
    }
}

impl StyledSpan {
    pub fn plain(text: impl Into<Cow<'static, str>>) -> StyledSpan {
        StyledSpan {
            text: text.into(),
            ink: None,
            voice: None,
            motion: None,
            slow: false,
        }
    }

    pub fn ink(mut self, ink: Color32) -> StyledSpan {
        self.ink = Some(SpanInk::Fixed(ink));
        self
    }

    pub fn prose(mut self, prose: Prose) -> StyledSpan {
        self.ink = Some(SpanInk::Prose(prose));
        self
    }

    pub fn lit(mut self, color: Color32) -> StyledSpan {
        self.ink = Some(SpanInk::Lit(color));
        self
    }

    pub fn legible(mut self, color: Color32) -> StyledSpan {
        self.ink = Some(SpanInk::Legible(color));
        self
    }

    pub fn voice(mut self, voice: TextVoice) -> StyledSpan {
        self.voice = Some(voice);
        self
    }

    pub fn motion(mut self, motion: TextMotion) -> StyledSpan {
        self.motion = Some(motion);
        self
    }

    pub fn slow(mut self) -> StyledSpan {
        self.slow = true;
        self
    }
}

impl From<StyledSpan> for TextPiece {
    fn from(span: StyledSpan) -> TextPiece {
        TextPiece::Span(span)
    }
}

pub fn reveal_times(pieces: &[TextPiece], pace: TypingPace) -> Vec<Duration> {
    let step =
        |slowness: f32| Duration::from_secs_f64(f64::from(slowness) / f64::from(pace.characters.0));
    let mut clock = Duration::ZERO;
    let mut times = Vec::new();
    for piece in pieces {
        match piece {
            TextPiece::Pause(pause) => clock += *pause,
            TextPiece::LinkStart(_) | TextPiece::LinkEnd | TextPiece::Room(_) => {}
            TextPiece::Span(span) => {
                let slowness = if span.slow { pace.slow_by } else { 1.0 };
                for _ in span.text.chars() {
                    clock += step(slowness);
                    times.push(clock);
                }
            }
            TextPiece::Key(_) | TextPiece::Image(_) => {
                clock += step(1.0);
                times.push(clock);
            }
        }
    }
    times
}

pub fn revealed(pieces: &[TextPiece], pace: TypingPace, elapsed: Duration) -> usize {
    reveal_times(pieces, pace)
        .iter()
        .take_while(|time| **time <= elapsed)
        .count()
}

pub fn reveal_duration(pieces: &[TextPiece], pace: TypingPace) -> Duration {
    reveal_times(pieces, pace)
        .last()
        .copied()
        .unwrap_or(Duration::ZERO)
}

pub struct StyledTextPlugin;

impl Plugin for StyledTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyNames>()
            .add_message::<TypewriterReveal>()
            .add_systems(
                Update,
                advance_typewriters.run_if(resource_exists::<TypingPace>),
            );
    }
}

pub(crate) fn advance_typewriters(
    time: Res<Time>,
    pace: Res<TypingPace>,
    mut lines: Query<(Entity, Ref<StyledLine>, &mut Typewriter)>,
    mut reveals: MessageWriter<TypewriterReveal>,
) {
    for (entity, line, mut typewriter) in &mut lines {
        if line.is_changed() && !line.is_added() {
            *typewriter = Typewriter::default();
        }
        let shown = if typewriter.finished {
            reveal_times(&line.pieces, *pace).len()
        } else {
            typewriter.elapsed += time.delta();
            if typewriter.elapsed >= reveal_duration(&line.pieces, *pace) {
                typewriter.finished = true;
            }
            revealed(&line.pieces, *pace, typewriter.elapsed)
        };
        if shown > typewriter.shown {
            let newly = typewriter.shown..shown;
            reveals.write_batch(newly.map(|index| TypewriterReveal { entity, index }));
            typewriter.shown = shown;
        }
    }
}
