use std::time::Duration;

use bevy::color::Color;
use serde::{Deserialize, Serialize};
use ui::tokens::palette;
use ui::{RichPiece, RichSpan, RichText, TextMotion, TextVoice};

use crate::core::time::Millis;
use crate::systems::input::map::{self, InputAction, InputMap};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Span {
    Text {
        text: &'static str,
        fx: &'static [Fx],
    },
    Input(InputAction),
}

pub const fn plain(text: &'static str) -> Span {
    Span::Text { text, fx: &[] }
}

pub const fn styled(text: &'static str, fx: &'static [Fx]) -> Span {
    Span::Text { text, fx }
}

pub const fn input(action: InputAction) -> Span {
    Span::Input(action)
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Fx {
    Ink(Ink),
    Voice(Voice),
    Motion(Motion),
    Slow,
    PauseAfter(Millis),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Danger,
    Item,
    Place,
    Name,
    Magic,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    Whisper,
    Shout,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Wave,
    Shake,
    Pulse,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LineText(pub Vec<SpanText>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SpanText {
    Text { text: String, fx: Vec<Fx> },
    Input(InputAction),
}

impl LineText {
    pub fn of(spans: &[Span]) -> LineText {
        LineText(
            spans
                .iter()
                .map(|span| match *span {
                    Span::Text { text, fx } => SpanText::Text {
                        text: text.to_owned(),
                        fx: fx.to_vec(),
                    },
                    Span::Input(action) => SpanText::Input(action),
                })
                .collect(),
        )
    }

    pub fn plain(text: impl Into<String>) -> LineText {
        LineText(vec![SpanText::Text {
            text: text.into(),
            fx: Vec::new(),
        }])
    }

    pub fn words(&self, inputs: &InputMap) -> String {
        self.0
            .iter()
            .map(|span| match span {
                SpanText::Text { text, .. } => text.clone(),
                SpanText::Input(action) => inputs.name(*action).unwrap_or_default(),
            })
            .collect()
    }

    pub fn word_count(&self) -> usize {
        self.0
            .iter()
            .map(|span| match span {
                SpanText::Text { text, .. } => text.split_whitespace().count(),
                SpanText::Input(_) => 1,
            })
            .sum()
    }

    pub fn rich(&self) -> RichText {
        RichText::new(self.0.iter().flat_map(SpanText::pieces).collect())
    }
}

impl SpanText {
    fn pieces(&self) -> Vec<RichPiece> {
        let (text, fx) = match self {
            SpanText::Text { text, fx } => (text, fx),
            SpanText::Input(action) => return vec![map::input(*action)],
        };
        let mut span = RichSpan::plain(text.clone());
        let mut pause = None;
        for fx in fx {
            match *fx {
                Fx::Ink(ink) => span = span.color(ink.color()),
                Fx::Voice(Voice::Whisper) => span = span.voice(TextVoice::Whisper),
                Fx::Voice(Voice::Shout) => span = span.voice(TextVoice::Shout),
                Fx::Motion(Motion::Wave) => span = span.motion(TextMotion::Wave),
                Fx::Motion(Motion::Shake) => span = span.motion(TextMotion::Shake),
                Fx::Motion(Motion::Pulse) => span = span.motion(TextMotion::Pulse),
                Fx::Slow => span = span.slow(),
                Fx::PauseAfter(wait) => {
                    pause = Some(RichPiece::Pause(Duration::from_secs_f32(wait.seconds().0)));
                }
            }
        }
        std::iter::once(span.into()).chain(pause).collect()
    }
}

impl Ink {
    pub fn color(self) -> Color {
        match self {
            Ink::Danger => palette::CRIMSON_80,
            Ink::Item => palette::AMBER_80,
            Ink::Place => palette::AZURE_80,
            Ink::Name => palette::EMERALD_80,
            Ink::Magic => palette::VIOLET_80,
        }
    }
}
