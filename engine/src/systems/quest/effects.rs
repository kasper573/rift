use crate::core::script::Reach;
use crate::core::script::Vocabulary;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Accepts, Completes, Decides, Fails, Objective, Offers, QuestId, TurnsIn};
use crate::core::content::SchemaMark;
use crate::core::content::{Content, Fixture};
use crate::core::script::ChoiceReveal;
use crate::core::script::{
    ACCEPTED, Answer, Arrow, Compiler, DECLINED, DONE, EffectDef, EffectStep, NEXT, REFUSED,
    ScriptFault,
};
use crate::core::script::{EffectCall, Expr};
use crate::core::text::LineText;
use crate::systems::aim::{self as effects, Reaching};
use crate::systems::dialogue::{self, CardOffer, ChoiceChip, ChoiceTag, ConversationPanel};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::messages;

const CANCELLED: &str = "cancelled";

/// Shows the offer card and pauses.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct OfferQuest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub accepted: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub declined: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Takes the quest at once.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct AcceptQuest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

/// Shows the turn-in card and pauses.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct TurnIn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
    /// Taken when the card is closed without handing in.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub cancelled: Arrow,
}

/// Fails an active quest.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct FailQuest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Completes a task objective.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct CompleteTask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    pub task: String,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Decides a quest's path.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("quest"))]
pub struct Decide {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub quest: QuestId,
    pub path: String,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for OfferQuest {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(ACCEPTED => accepted, DECLINED => declined, REFUSED => refused);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (quest, refused, camera) = (self.quest, self.refused.clone(), cx.camera());
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                let words = &messages::of(world.resource::<Content>()).quest;
                let reason = LineText::plain(words.nobody_to_offer());
                return effects::refuse(world, cx, &refused, reason);
            };
            if let Some(reason) = super::offer_refusal(world, player, quest) {
                return effects::refuse(world, cx, &refused, reason);
            }
            let offers = offer_card(world, player, quest);
            let panel = Some(ConversationPanel::QuestOffer(quest));
            if dialogue::card(
                world,
                flow,
                player,
                offers,
                Answer::Output(DECLINED),
                panel,
                camera,
            ) {
                EffectStep::Pause
            } else {
                EffectStep::Output(DECLINED)
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Offers(self.quest))]
    }
}

impl EffectDef for AcceptQuest {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let (quest, refused) = (self.quest, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let nobody = messages::of(world.resource::<Content>())
                .quest
                .nobody_to_accept()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: DONE,
                    refused: Some(&refused),
                    nobody: Some(&nobody),
                },
                move |world, player| super::accept(world, player, quest),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Accepts(self.quest))]
    }
}

impl EffectDef for TurnIn {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused, CANCELLED => cancelled);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (quest, refused, camera) = (self.quest, self.refused.clone(), cx.camera());
        let decides = quest.get(cx.content).decision().is_some();
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                let words = &messages::of(world.resource::<Content>()).quest;
                let reason = LineText::plain(words.nobody_to_hand_in());
                return effects::refuse(world, cx, &refused, reason);
            };
            if !super::ready_to_hand_in(world, player, quest) {
                let message = super::not_ready(world.resource::<Content>(), quest);
                return effects::refuse(world, cx, &refused, message);
            }
            if decides {
                let handed_in = super::turn_in(world, player, quest, None);
                return effects::settle(world, cx, &refused, handed_in);
            }
            let offers = turn_in_card(world, player, quest);
            let panel = Some(ConversationPanel::QuestTurnIn(quest));
            if dialogue::card(
                world,
                flow,
                player,
                offers,
                Answer::Output(CANCELLED),
                panel,
                camera,
            ) {
                EffectStep::Pause
            } else {
                EffectStep::Output(CANCELLED)
            }
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(TurnsIn(self.quest))]
    }
}

impl EffectDef for FailQuest {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let quest = self.quest;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                move |world, player| {
                    super::fail(world, player, quest);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Fails(self.quest))]
    }
}

impl EffectDef for CompleteTask {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let quest = self.quest;
        let task = self.task.as_str();
        let index = quest
            .get(cx.content)
            .objectives()
            .position(|objective| matches!(objective, Objective::Task { task: known, .. } if *known == task))
            .ok_or_else(|| {
                ScriptFault::new(
                    "unknown task",
                    format!("{} has no task {task}", quest.path(cx.content)),
                )
            })?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                move |world, player| {
                    super::complete_task(world, player, quest, index);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Completes(self.quest))]
    }
}

impl EffectDef for Decide {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let (quest, path) = (self.quest, self.path.clone());
        let known = quest
            .get(cx.content)
            .decision()
            .is_some_and(|paths| paths.iter().any(|known| known.choice == path));
        if !known {
            return Err(ScriptFault::new(
                "unknown path",
                format!("{} has no path {path:?}", quest.path(cx.content)),
            ));
        }
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            let path = path.clone();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                move |world, player| {
                    super::decide(world, player, quest, &path);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Decides(self.quest))]
    }
}

fn offer_card(world: &World, player: Entity, quest: QuestId) -> Vec<CardOffer> {
    let content = world.resource::<Content>();
    let words = &messages::of(content).quest;
    let def = quest.get(content);
    let open = super::open_to(world, player, quest);
    let chips = match def.requires().filter(|_| def.reveals(ChoiceReveal::Needs)) {
        Some(requires) => ChoiceChip::needs(world, player, requires),
        None => Vec::new(),
    };
    vec![
        CardOffer {
            label: LineText::plain(words.accept()),
            tag: ChoiceTag::Quest,
            icon: Some(super::offered_mark(def).get(content).icon.clone()),
            chips,
            locked: !open,
            warn: None,
            pick: Arc::new(move |world, player| {
                super::accept(world, player, quest).map(|()| Answer::Output(ACCEPTED))
            }),
        },
        CardOffer {
            label: LineText::plain(words.decline()),
            tag: ChoiceTag::Dialogue,
            icon: None,
            chips: Vec::new(),
            locked: false,
            warn: None,
            pick: Arc::new(|_, _| Ok(Answer::Output(DECLINED))),
        },
    ]
}

fn turn_in_card(world: &World, player: Entity, quest: QuestId) -> Vec<CardOffer> {
    let content = world.resource::<Content>();
    let def = quest.get(content);
    let inventory = world.get::<Inventory>(player);
    let have = |item| inventory.map_or(0, |inventory| inventory.count(item));
    let chips = |pick: Option<ItemStack>| {
        let mut chips = Vec::new();
        if def.reveals(ChoiceReveal::Costs) {
            chips.extend(def.hand_in().iter().map(|stack| ChoiceChip::Pays {
                item: stack.item,
                count: stack.count,
                have: have(stack.item),
            }));
        }
        if def.reveals(ChoiceReveal::Gains) {
            chips.extend(def.rewards().chain(pick).map(|stack| ChoiceChip::Gets {
                item: stack.item,
                count: stack.count,
            }));
        }
        chips
    };
    let mut offers: Vec<CardOffer> = if def.pick_one().is_empty() {
        vec![CardOffer {
            label: dialogue::spoken(world, player, def.hand_over()),
            tag: ChoiceTag::Quest,
            icon: Some(super::ready_mark(def).get(content).icon.clone()),
            chips: chips(None),
            locked: false,
            warn: None,
            pick: Arc::new(move |world, player| {
                super::turn_in(world, player, quest, None).map(|()| Answer::Output(DONE))
            }),
        }]
    } else {
        def.pick_one()
            .iter()
            .map(|&pick| CardOffer {
                label: super::pick_label(content, pick),
                tag: ChoiceTag::Quest,
                icon: None,
                chips: chips(Some(pick)),
                locked: false,
                warn: None,
                pick: Arc::new(move |world, player| {
                    super::turn_in(world, player, quest, Some(pick)).map(|()| Answer::Output(DONE))
                }),
            })
            .collect()
    };
    offers.push(CardOffer {
        label: LineText::plain(messages::of(world.resource::<Content>()).quest.not_yet()),
        tag: ChoiceTag::Dialogue,
        icon: None,
        chips: Vec::new(),
        locked: false,
        warn: None,
        pick: Arc::new(|_, _| Ok(Answer::Output(CANCELLED))),
    });
    offers
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<OfferQuest>("offer_quest")
        .effect::<AcceptQuest>("accept_quest")
        .effect::<TurnIn>("turn_in")
        .effect::<FailQuest>("fail_quest")
        .effect::<CompleteTask>("complete_task")
        .effect::<Decide>("decide");
}
