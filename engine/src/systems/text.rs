use std::ops::Not;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::ink::Prose;

use crate::core::script::{Expr, Value};
use crate::core::text::{StyledLine, StyledSpan, TextMotion, TextPiece, TextVoice};
use crate::core::units::{Millis, Seconds};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::script::RiftScripting;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(try_from = "SpanRow", into = "SpanRow")]
#[schemars(extend("x-span" = true))]
pub enum Span {
    Text {
        text: String,
        fx: Vec<Fx>,
    },
    Input(InputAction),
    Value {
        value: Box<Expr<RiftScripting>>,
        fx: Vec<Fx>,
    },
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(untagged)]
enum SpanRow {
    Plain(String),
    Styled(StyledRow),
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyledRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<Expr<RiftScripting>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<InputAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ink: Option<Ink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voice: Option<TextVoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    motion: Option<TextMotion>,
    #[serde(default, skip_serializing_if = "Not::not")]
    slow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pause_after: Option<Millis>,
}

impl TryFrom<SpanRow> for Span {
    type Error = &'static str;

    fn try_from(row: SpanRow) -> Result<Span, &'static str> {
        let styled = match row {
            SpanRow::Plain(text) => return Ok(plain(text)),
            SpanRow::Styled(styled) => styled,
        };
        let mut fx = Vec::new();
        fx.extend(styled.ink.map(Fx::Ink));
        fx.extend(styled.voice.map(Fx::Voice));
        fx.extend(styled.motion.map(Fx::Motion));
        if styled.slow {
            fx.push(Fx::Slow);
        }
        fx.extend(styled.pause_after.map(Fx::PauseAfter));
        match (styled.text, styled.value, styled.input) {
            (Some(text), None, None) => Ok(Span::Text { text, fx }),
            (None, Some(value), None) => Ok(Span::Value {
                value: Box::new(value),
                fx,
            }),
            (None, None, Some(input)) if fx.is_empty() => Ok(Span::Input(input)),
            _ => Err("a span is a string, { text }, { value } or { input }"),
        }
    }
}

impl From<Span> for SpanRow {
    fn from(span: Span) -> SpanRow {
        let (text, value, input, fx) = match span {
            Span::Text { text, fx } if fx.is_empty() => return SpanRow::Plain(text),
            Span::Text { text, fx } => (Some(text), None, None, fx),
            Span::Value { value, fx } => (None, Some(*value), None, fx),
            Span::Input(action) => (None, None, Some(action), Vec::new()),
        };
        let mut row = StyledRow {
            text,
            value,
            input,
            ink: None,
            voice: None,
            motion: None,
            slow: false,
            pause_after: None,
        };
        for fx in fx {
            match fx {
                Fx::Ink(ink) => row.ink = Some(ink),
                Fx::Voice(voice) => row.voice = Some(voice),
                Fx::Motion(motion) => row.motion = Some(motion),
                Fx::Slow => row.slow = true,
                Fx::PauseAfter(millis) => row.pause_after = Some(millis),
            }
        }
        SpanRow::Styled(row)
    }
}

/// A count of a noun: "1 rank", "3 ranks".
/// A span of time at a glance: whole seconds under a minute, whole minutes from there.
pub fn duration(span: Seconds) -> String {
    if span.0 >= 60.0 {
        format!("{} min", (span.0 / 60.0).ceil() as u32)
    } else {
        format!("{} s", span.0.ceil().max(1.0) as u32)
    }
}

pub fn plain(text: impl Into<String>) -> Span {
    Span::Text {
        text: text.into(),
        fx: Vec::new(),
    }
}

pub fn values(spans: &[Span]) -> Vec<&Expr<RiftScripting>> {
    spans
        .iter()
        .filter_map(|span| match span {
            Span::Value { value, .. } => Some(&**value),
            Span::Text { .. } | Span::Input(_) => None,
        })
        .collect()
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Fx {
    Ink(Ink),
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
pub enum Ink {
    Danger,
    Item,
    Place,
    Name,
    Magic,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LineText(pub Vec<SpanText>);

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SpanText {
    Text { text: String, fx: Vec<Fx> },
    Input(InputAction),
}

impl LineText {
    pub fn spoken(spans: &[Span], values: &[Value]) -> LineText {
        let mut values = values.iter();
        LineText(
            spans
                .iter()
                .map(|span| match span {
                    Span::Text { text, fx } => SpanText::Text {
                        text: text.clone(),
                        fx: fx.clone(),
                    },
                    Span::Value { fx, .. } => SpanText::Text {
                        text: values.next().map(Value::as_text).unwrap_or_default(),
                        fx: fx.clone(),
                    },
                    Span::Input(action) => SpanText::Input(*action),
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

    pub fn plain_words(&self) -> String {
        self.0
            .iter()
            .map(|span| match span {
                SpanText::Text { text, .. } => text.as_str(),
                SpanText::Input(_) => "",
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

    pub fn styled(&self) -> StyledLine {
        StyledLine::new(self.0.iter().flat_map(SpanText::pieces).collect())
    }
}

impl SpanText {
    fn pieces(&self) -> Vec<TextPiece> {
        let (text, fx) = match self {
            SpanText::Text { text, fx } => (text, fx),
            SpanText::Input(action) => return vec![map::input(*action)],
        };
        let mut span = StyledSpan::plain(text.clone());
        let mut pause = None;
        for fx in fx {
            match *fx {
                Fx::Ink(ink) => span = span.prose(ink.prose()),
                Fx::Voice(voice) => span = span.voice(voice),
                Fx::Motion(motion) => span = span.motion(motion),
                Fx::Slow => span = span.slow(),
                Fx::PauseAfter(wait) => {
                    pause = Some(TextPiece::Pause(Duration::from_secs_f32(wait.seconds().0)));
                }
            }
        }
        std::iter::once(span.into()).chain(pause).collect()
    }
}

impl Ink {
    pub fn prose(self) -> Prose {
        match self {
            Ink::Danger => Prose::Danger,
            Ink::Item => Prose::Item,
            Ink::Place => Prose::Place,
            Ink::Name => Prose::Name,
            Ink::Magic => Prose::Magic,
        }
    }
}
