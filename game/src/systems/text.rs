use std::time::Duration;

use bevy::color::Color;
use serde::{Deserialize, Serialize};
use ui::tokens::palette;
use ui::{RichPiece, RichSpan, RichText, TextMotion, TextVoice};

use crate::core::time::Millis;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub text: &'static str,
    pub fx: &'static [Fx],
}

pub const fn plain(text: &'static str) -> Span {
    Span { text, fx: &[] }
}

pub const fn styled(text: &'static str, fx: &'static [Fx]) -> Span {
    Span { text, fx }
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
pub struct SpanText {
    pub text: String,
    pub fx: Vec<Fx>,
}

impl LineText {
    pub fn of(spans: &[Span]) -> LineText {
        LineText(
            spans
                .iter()
                .map(|span| SpanText {
                    text: span.text.to_owned(),
                    fx: span.fx.to_vec(),
                })
                .collect(),
        )
    }

    pub fn plain(text: impl Into<String>) -> LineText {
        LineText(vec![SpanText {
            text: text.into(),
            fx: Vec::new(),
        }])
    }

    pub fn words(&self) -> String {
        self.0.iter().map(|span| span.text.as_str()).collect()
    }

    pub fn rich(&self) -> RichText {
        RichText::new(self.0.iter().flat_map(SpanText::pieces).collect())
    }
}

impl SpanText {
    fn pieces(&self) -> Vec<RichPiece> {
        let mut span = RichSpan::plain(self.text.clone());
        let mut pause = None;
        for fx in &self.fx {
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
