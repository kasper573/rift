use serde::{Deserialize, Serialize};

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
}
