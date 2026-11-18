use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::script::{DialogueCamera, Expr, ScriptFault, WordsDef};
use crate::systems::actor::skeleton::{Declared, GestureName};
use crate::systems::dialogue::span::ScriptSpan;
use crate::systems::npc::NpcId;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(from = "LineRow", into = "LineRow")]
pub struct ScriptLine {
    pub by: Speaker,
    pub gesture: Option<GestureName>,
    pub camera: Option<DialogueCamera>,
    pub text: Vec<ScriptSpan>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LineRow {
    by: SpeakerRow,
    /// How the line is seen, when not as the rest of its script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<DialogueCamera>,
    text: Vec<ScriptSpan>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SpeakerRow {
    Npc {
        npc: NpcId,
        /// The gesture the npc makes as it says the line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gesture: Option<GestureName>,
    },
    Object,
    Narrator,
    Character,
}

impl From<LineRow> for ScriptLine {
    fn from(row: LineRow) -> ScriptLine {
        let (by, gesture) = match row.by {
            SpeakerRow::Npc { npc, gesture } => (Speaker::Npc(npc), gesture),
            SpeakerRow::Object => (Speaker::Object, None),
            SpeakerRow::Narrator => (Speaker::Narrator, None),
            SpeakerRow::Character => (Speaker::Character, None),
        };
        ScriptLine {
            by,
            gesture,
            camera: row.camera,
            text: row.text,
        }
    }
}

impl From<ScriptLine> for LineRow {
    fn from(line: ScriptLine) -> LineRow {
        LineRow {
            by: match line.by {
                Speaker::Npc(npc) => SpeakerRow::Npc {
                    npc,
                    gesture: line.gesture,
                },
                Speaker::Object => SpeakerRow::Object,
                Speaker::Narrator => SpeakerRow::Narrator,
                Speaker::Character => SpeakerRow::Character,
            },
            camera: line.camera,
            text: line.text,
        }
    }
}

impl ScriptLine {
    /// What refuses the line: a gesture no skeleton's set declares.
    pub fn fault(&self, content: &Content) -> Option<ScriptFault> {
        let gesture = self.gesture.as_ref()?;
        let declared = content.built::<Declared>();
        (!declared.gestures.contains(gesture)).then(|| {
            ScriptFault::new(
                "undeclared gesture",
                format!(
                    "a gesture some skeleton's set declares; a line is said with gesture {}, which no skeleton's set declares",
                    gesture.as_str()
                ),
            )
        })
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    Npc(NpcId),
    Object,
    Narrator,
    Character,
}

impl WordsDef for ScriptLine {
    fn values(&self) -> Vec<&Expr> {
        ScriptSpan::values(&self.text)
    }

    fn check(&self, content: &Content) -> Result<(), ScriptFault> {
        self.fault(content).map_or(Ok(()), Err)
    }
}

impl WordsDef for Vec<ScriptSpan> {
    fn values(&self) -> Vec<&Expr> {
        ScriptSpan::values(self)
    }
}
