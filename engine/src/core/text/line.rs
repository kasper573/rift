use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::ink::Prose;

use super::{KeyNames, PluggedSpan, StyledLine, StyledSpan, TextMotion, TextPiece, TextVoice};
use crate::core::content::Content;
use crate::core::units::Millis;

/// A line of text as the game says it to a player, made of spans: the plain and styled words of
/// its own, and whatever kinds of span the features of the game plugged in. It crosses the network
/// and the store whole, as every kind of span reads and writes itself.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LineText(pub Vec<SpanText>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SpanText {
    Text {
        text: String,
        fx: Vec<TextFx>,
    },
    /// The spans of a spoken line, which type out while what is said around them shows whole.
    Typed(Vec<SpanText>),
    Part(PluggedSpan),
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum TextFx {
    Ink(TextInk),
    Voice(TextVoice),
    Motion(TextMotion),
    Slow,
    PauseAfter(Millis),
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
pub enum TextInk {
    Danger,
    Item,
    Place,
    Name,
    Magic,
}

impl TextInk {
    pub fn prose(self) -> Prose {
        match self {
            TextInk::Danger => Prose::Danger,
            TextInk::Item => Prose::Item,
            TextInk::Place => Prose::Place,
            TextInk::Name => Prose::Name,
            TextInk::Magic => Prose::Magic,
        }
    }
}

impl From<&str> for LineText {
    fn from(text: &str) -> LineText {
        LineText::plain(text)
    }
}

impl From<String> for LineText {
    fn from(text: String) -> LineText {
        LineText::plain(text)
    }
}

impl From<SpanText> for LineText {
    fn from(span: SpanText) -> LineText {
        LineText(vec![span])
    }
}

impl LineText {
    pub fn plain(text: impl Into<String>) -> LineText {
        LineText(vec![SpanText::Text {
            text: text.into(),
            fx: Vec::new(),
        }])
    }

    /// This line with `more` after it.
    pub fn then(mut self, more: impl Into<LineText>) -> LineText {
        self.0.extend(more.into().0);
        self
    }

    /// This line with `with` wherever its words read `hole`: how a line worded by the game's
    /// settings takes what it is about.
    pub fn filling(self, hole: &str, with: impl Into<LineText>) -> LineText {
        let with = with.into();
        let mut filled = Vec::new();
        for span in self.0 {
            let SpanText::Text { text, fx } = span else {
                filled.push(span);
                continue;
            };
            let mut parts = text.split(hole).peekable();
            while let Some(part) = parts.next() {
                if !part.is_empty() {
                    filled.push(SpanText::Text {
                        text: part.to_owned(),
                        fx: fx.clone(),
                    });
                }
                if parts.peek().is_some() {
                    filled.extend(with.0.iter().cloned());
                }
            }
        }
        LineText(filled)
    }

    /// What the line reads as in words, the keys it shows named as `keys` names them.
    pub fn words(&self, content: &Content, keys: &KeyNames) -> String {
        self.read(content, Some(keys))
    }

    /// What the line reads as in words, leaving out the keys it shows.
    pub fn plain_words(&self, content: &Content) -> String {
        self.read(content, None)
    }

    pub fn word_count(&self, content: &Content) -> usize {
        self.0
            .iter()
            .map(|span| match span {
                SpanText::Text { text, .. } => text.split_whitespace().count(),
                SpanText::Typed(spans) => LineText(spans.clone()).word_count(content),
                SpanText::Part(part) => part.word_count(content),
            })
            .sum()
    }

    pub fn styled(&self, content: &Content) -> StyledLine {
        StyledLine::new(
            self.0
                .iter()
                .flat_map(|span| span.pieces(content))
                .collect(),
        )
    }

    fn read(&self, content: &Content, keys: Option<&KeyNames>) -> String {
        self.0
            .iter()
            .map(|span| match span {
                SpanText::Text { text, .. } => text.clone(),
                SpanText::Typed(spans) => LineText(spans.clone()).read(content, keys),
                SpanText::Part(part) => part.words(content, keys),
            })
            .collect()
    }
}

impl SpanText {
    fn pieces(&self, content: &Content) -> Vec<TextPiece> {
        let (text, fx) = match self {
            SpanText::Text { text, fx } => (text, fx),
            SpanText::Part(part) => return part.pieces(content),
            SpanText::Typed(spans) => {
                let line = LineText(spans.clone()).styled(content);
                return std::iter::once(TextPiece::TypingStart)
                    .chain(line.pieces)
                    .chain([TextPiece::TypingEnd])
                    .collect();
            }
        };
        let mut span = StyledSpan::plain(text.clone());
        let mut pause = None;
        for fx in fx {
            match *fx {
                TextFx::Ink(ink) => span = span.prose(ink.prose()),
                TextFx::Voice(voice) => span = span.voice(voice),
                TextFx::Motion(motion) => span = span.motion(motion),
                TextFx::Slow => span = span.slow(),
                TextFx::PauseAfter(wait) => {
                    pause = Some(TextPiece::Pause(Duration::from_secs_f32(wait.seconds().0)));
                }
            }
        }
        std::iter::once(span.into()).chain(pause).collect()
    }
}
