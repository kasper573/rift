use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::time::WallClock;
use crate::core::units::Seconds;
use crate::systems::aim::character;
use crate::systems::mention::Mention;
use crate::systems::quest;
use crate::systems::quest::Objective;
use crate::systems::quest::QuestId;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct OnQuest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub quest: QuestId,
}

impl QueryDef for OnQuest {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let OnQuest { of, quest } = self;
        Ok({
            let quest = *quest;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    quest::log_of(world, who).is_some_and(|log| log.active(quest).is_some())
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct QuestDone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub quest: QuestId,
}

impl QueryDef for QuestDone {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let QuestDone { of, quest } = self;
        Ok({
            let quest = *quest;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    quest::log_of(world, who).is_some_and(|log| {
                        log.finished(quest)
                            .is_some_and(|done| done.result == quest::QuestResult::Completed)
                    })
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct QuestCooling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub quest: QuestId,
}

impl QueryDef for QuestCooling {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let QuestCooling { of, quest } = self;
        Ok({
            let quest = *quest;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    let today = world.resource::<WallClock>().day();
                    quest::log_of(world, who).is_some_and(|log| {
                        quest.get(world.resource::<Content>()).daily()
                            && log.done_for_now(world.resource::<Content>(), quest, today)
                    })
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct TaskDone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub quest: QuestId,
    pub task: String,
}

impl QueryDef for TaskDone {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let TaskDone { of, quest, task } = self;
        Ok({
            let quest = *quest;
            let of = cx.actor(of.as_deref())?;
            let index = quest
                    .get(cx.content)
                    .objectives()
                    .position(|objective| matches!(objective, Objective::Task { task: known, .. } if known == task))
                    .ok_or_else(|| {
                        ScriptFault::new(
                            "unknown task",
                            format!("{} has no task {task}", quest.path(cx.content)),
                        )
                    })?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    quest::log_of(world, who).is_some_and(|log| {
                        log.active(quest).is_some_and(|active| {
                            active.counts.get(index).is_some_and(|count| *count > 0)
                        })
                    })
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct QuestResetsIn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub quest: QuestId,
}

impl QueryDef for QuestResetsIn {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Text)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let QuestResetsIn { of, quest } = self;
        Ok({
            let quest = *quest;
            if !quest.get(cx.content).daily() {
                return Err(ScriptFault::new(
                    "never resets",
                    format!("a daily quest, not {}", quest.path(cx.content)),
                ));
            }
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Text(
                    character(world, cx, &of)
                        .and_then(|who| quest::log_of(world, who))
                        .and_then(|log| log.finished(quest))
                        .and_then(|done| done.resets_in(world.resource::<WallClock>().now))
                        .map(Seconds::in_words)
                        .unwrap_or_default(),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<OnQuest>("on_quest")
        .query::<QuestDone>("quest_done")
        .query::<QuestCooling>("quest_cooling")
        .query::<TaskDone>("task_done")
        .query::<QuestResetsIn>("quest_resets_in");
    vocabulary
        .extension::<Describers>()
        .alone::<OnQuest>(|q, d| {
            d.words()
                .on_quest(d.names.mentioned(Mention::Quest(q.quest)))
        })
        .alone::<QuestDone>(|q, d| d.names.mentioned(Mention::Quest(q.quest)))
        .alone::<QuestCooling>(|q, d| {
            d.words()
                .quest_cooling(d.names.mentioned(Mention::Quest(q.quest)))
        })
        .alone::<TaskDone>(|q, d| {
            d.words().task_done(
                d.names.mentioned(Mention::Quest(q.quest)),
                q.task.to_string(),
            )
        })
        .alone::<QuestResetsIn>(|q, d| {
            d.words()
                .quest_resets(d.names.mentioned(Mention::Quest(q.quest)))
        });
}
