use std::ops::Not;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::ink;

use crate::core::content::{Content, ImageRef, SchemaMark};
use crate::core::script::{Expr, Value};
use crate::core::text::{
    KeyNames, LineText, PluggedSpan, SpanDef, SpanKinds, SpanText, StyledSpan, TextFx, TextInk,
    TextMotion, TextPiece, TextVoice,
};
use crate::core::units::Millis;
use crate::systems::input::map::InputAction;

/// Registers the kinds of span this system draws.
pub fn spans(kinds: &mut SpanKinds) {
    kinds.add::<SpeakerName>("speaker_name");
    kinds.add::<Picture>("picture");
}

/// The name of whoever says a line, set in the accent of a spoken line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpeakerName(pub String);

impl SpanDef for SpeakerName {
    fn pieces(&self, _: &Content) -> Vec<TextPiece> {
        let name = StyledSpan::plain(self.0.clone())
            .lit(ink::DIALOGUE_NAME)
            .bold();
        vec![name.into()]
    }

    fn words(&self, _: &Content, _: Option<&KeyNames>) -> String {
        self.0.clone()
    }
}

impl From<SpeakerName> for SpanText {
    fn from(name: SpeakerName) -> SpanText {
        SpanText::Part(PluggedSpan::new(name))
    }
}

/// A picture set among the words of a line, as tall as its letters, which reads as nothing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Picture(pub ImageRef);

impl SpanDef for Picture {
    fn pieces(&self, _: &Content) -> Vec<TextPiece> {
        vec![TextPiece::Image(self.0.path().into())]
    }

    fn words(&self, _: &Content, _: Option<&KeyNames>) -> String {
        String::new()
    }
}

impl From<Picture> for SpanText {
    fn from(picture: Picture) -> SpanText {
        SpanText::Part(PluggedSpan::new(picture))
    }
}

/// A span of the words a script says, as its files write them: plain or styled text, the cap of
/// an input, or a value the script computes.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(try_from = "SpanRow", into = "SpanRow")]
#[schemars(transform = SchemaMark::Span.with(true))]
pub enum ScriptSpan {
    Text { text: String, fx: Vec<TextFx> },
    Input(InputAction),
    Value { value: Box<Expr>, fx: Vec<TextFx> },
}

impl ScriptSpan {
    pub fn plain(text: impl Into<String>) -> ScriptSpan {
        ScriptSpan::Text {
            text: text.into(),
            fx: Vec::new(),
        }
    }

    /// The values `spans` read.
    pub fn values(spans: &[ScriptSpan]) -> Vec<&Expr> {
        spans
            .iter()
            .filter_map(|span| match span {
                ScriptSpan::Value { value, .. } => Some(&**value),
                ScriptSpan::Text { .. } | ScriptSpan::Input(_) => None,
            })
            .collect()
    }

    /// `spans` as a line, each value they read said as `values` has it, in order.
    pub fn said(spans: &[ScriptSpan], values: &[Value]) -> LineText {
        let mut values = values.iter();
        LineText(
            spans
                .iter()
                .map(|span| match span {
                    ScriptSpan::Text { text, fx } => SpanText::Text {
                        text: text.clone(),
                        fx: fx.clone(),
                    },
                    ScriptSpan::Value { fx, .. } => SpanText::Text {
                        text: values.next().map(Value::as_text).unwrap_or_default(),
                        fx: fx.clone(),
                    },
                    ScriptSpan::Input(action) => (*action).into(),
                })
                .collect(),
        )
    }
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
    value: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<InputAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ink: Option<TextInk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voice: Option<TextVoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    motion: Option<TextMotion>,
    #[serde(default, skip_serializing_if = "Not::not")]
    slow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pause_after: Option<Millis>,
}

impl TryFrom<SpanRow> for ScriptSpan {
    type Error = &'static str;

    fn try_from(row: SpanRow) -> Result<ScriptSpan, &'static str> {
        let styled = match row {
            SpanRow::Plain(text) => return Ok(ScriptSpan::plain(text)),
            SpanRow::Styled(styled) => styled,
        };
        let mut fx = Vec::new();
        fx.extend(styled.ink.map(TextFx::Ink));
        fx.extend(styled.voice.map(TextFx::Voice));
        fx.extend(styled.motion.map(TextFx::Motion));
        if styled.slow {
            fx.push(TextFx::Slow);
        }
        fx.extend(styled.pause_after.map(TextFx::PauseAfter));
        match (styled.text, styled.value, styled.input) {
            (Some(text), None, None) => Ok(ScriptSpan::Text { text, fx }),
            (None, Some(value), None) => Ok(ScriptSpan::Value {
                value: Box::new(value),
                fx,
            }),
            (None, None, Some(input)) if fx.is_empty() => Ok(ScriptSpan::Input(input)),
            _ => Err("a span is a string, { text }, { value } or { input }"),
        }
    }
}

impl From<ScriptSpan> for SpanRow {
    fn from(span: ScriptSpan) -> SpanRow {
        let (text, value, input, fx) = match span {
            ScriptSpan::Text { text, fx } if fx.is_empty() => return SpanRow::Plain(text),
            ScriptSpan::Text { text, fx } => (Some(text), None, None, fx),
            ScriptSpan::Value { value, fx } => (None, Some(*value), None, fx),
            ScriptSpan::Input(action) => (None, None, Some(action), Vec::new()),
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
                TextFx::Ink(ink) => row.ink = Some(ink),
                TextFx::Voice(voice) => row.voice = Some(voice),
                TextFx::Motion(motion) => row.motion = Some(motion),
                TextFx::Slow => row.slow = true,
                TextFx::PauseAfter(millis) => row.pause_after = Some(millis),
            }
        }
        SpanRow::Styled(row)
    }
}
