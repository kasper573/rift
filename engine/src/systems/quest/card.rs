use bevy::prelude::*;
use ui::card::{self, Art, Card, CardResponse};
use ui::controls::{self, Pill};
use ui::egui::{Align, Label, Layout, Response, Sense, Ui};
use ui::face::{self, TextRole};
use ui::glyph::{self, Kind, Mark};
use ui::ink;

use super::{
    ActiveQuest, Objective, Parties, Progress, QuestDef, QuestId, QuestLog, clock_label,
    resets_label,
};
use crate::core::content::Content;
use crate::core::gui::ImageSource;
use crate::core::time::UnixMillis;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::dialogue::{ChoiceReveal, Conversation, ConversationPanel};
use crate::systems::hud;
use crate::systems::item::widget::{SlotLook, slot};
use crate::systems::item::{self, Inventory, ItemId, ItemStack};
use crate::systems::player::session;

const ICON: f32 = 20.0;
const MARK: f32 = 13.0;
const CHIP_ICON: f32 = 16.0;

pub struct QuestCardPlugin;

impl Plugin for QuestCardPlugin {
    fn build(&self, app: &mut App) {
        stage::dock(
            app,
            DockedPanel {
                head: card_head,
                show: show_docked,
            },
        );
    }
}

pub struct QuestCardLook {
    pub width: f32,
    pub max_height: f32,
    pub moment: Option<Moment>,
}

/// A quest's card: its face, and whatever `plate` writes under the overview.
pub fn show<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    quest: QuestId,
    look: QuestCardLook,
    plate: impl FnOnce(&mut Ui, &mut I),
) -> CardResponse {
    let def = quest.get(content);
    let band = match look.moment {
        Some(moment) => format!("{} · {}", moment.band(), def.category.to_lowercase()),
        None => def.category.to_lowercase(),
    };
    let icon = images.image(def.icon(content)).map(|image| image.texture);
    let mut shown = Card::new(Kind::Quest, &def.title);
    shown.band = &band;
    shown.art = Art {
        icon,
        height: card::ART_QUEST,
    };
    shown.width = look.width;
    shown.max_height = look.max_height;
    shown.show(ui, |ui, written| {
        if written.name_cut {
            card::full_name(ui, &def.title);
        }
        card::overview(ui, &overview(def));
        plate(ui, images);
    })
}

/// The body of a quest's offer: what it asks, and what it gives when the quest lets on. Answers
/// the rewards shown, for a reader who may inspect them.
pub fn offer<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    parties: &Parties,
    quest: QuestId,
    inventory: Option<&Inventory>,
) -> Vec<(Response, ItemId)> {
    let def = quest.get(content);
    let progress = def.progress(
        content,
        parties,
        &ActiveQuest::fresh(content, quest),
        inventory,
    );
    card::overview(ui, &def.meta(parties, quest));
    card::heading(ui, "objectives");
    for (index, (objective, progress)) in def.objectives().zip(&progress).enumerate() {
        ui.push_id(("objective", index), |ui| {
            objective_row(ui, images, content, objective, progress, Standing::Open);
        });
    }
    let mut shown = Vec::new();
    if def.reveals(ChoiceReveal::Gains) {
        card::heading(ui, "rewards");
        shown.extend(reward_chips(ui, images, content, def));
        if !def.pick_one().is_empty() {
            card::heading(ui, "pick one when you return");
            shown.extend(picks(ui, images, content, def));
        }
    }
    card::foot(ui, "Accept or decline in the conversation");
    shown
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    Offer,
    TurnIn,
    Decision,
}

impl Moment {
    pub fn band(self) -> &'static str {
        match self {
            Moment::Offer => "quest offer",
            Moment::TurnIn => "quest complete",
            Moment::Decision => "a permanent choice",
        }
    }
}

fn moment(world: &World) -> Option<(QuestId, Moment)> {
    let seen = session::me(world)?;
    let panel = world.get::<Conversation>(seen)?.panel.as_ref()?;
    Some(match *panel {
        ConversationPanel::QuestOffer(quest) => (quest, Moment::Offer),
        ConversationPanel::QuestTurnIn(quest) => (quest, Moment::TurnIn),
        ConversationPanel::QuestDecision(quest) => (quest, Moment::Decision),
    })
}

fn card_head(world: &World) -> Option<PanelHead> {
    moment(world)?;
    Some(PanelHead {
        width: card::DOCKED_WIDTH,
        chrome: PanelChrome::Card,
    })
}

fn show_docked(ui: &mut Ui, world: &mut World, width: f32, most: f32) {
    let Some((quest, moment)) = moment(world) else {
        return;
    };
    let inventory = session::me(world)
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let look = QuestCardLook {
        width,
        max_height: most.min(card::MAX_HEIGHT),
        moment: Some(moment),
    };
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    show(ui, world, &content, quest, look, |ui, world| {
        let shown = match moment {
            Moment::Offer => offer(ui, world, &content, &parties, quest, Some(&inventory)),
            Moment::TurnIn => turn_in(ui, world, &content, quest, &inventory),
            Moment::Decision => {
                decision(ui, &content, quest);
                Vec::new()
            }
        };
        inspectable(world, shown);
    });
}

/// Lets the reader inspect each item a card shows.
pub(super) fn inspectable(world: &mut World, shown: Vec<(Response, ItemId)>) {
    for (response, item) in shown {
        item::card::inspect(world, response, item);
    }
}

fn turn_in(
    ui: &mut Ui,
    images: &mut World,
    content: &Content,
    quest: QuestId,
    inventory: &Inventory,
) -> Vec<(Response, ItemId)> {
    let def = quest.get(content);
    if def.reveals(ChoiceReveal::Costs) {
        card::heading(ui, "hand in");
        for (index, &stack) in def.hand_in().iter().enumerate() {
            ui.push_id(("hand in", index), |ui| {
                hand_in_row(ui, images, content, stack, inventory.count(stack.item));
            });
        }
    }
    if !def.reveals(ChoiceReveal::Gains) {
        return Vec::new();
    }
    card::heading(ui, "rewards");
    let shown = reward_chips(ui, images, content, def);
    if !def.pick_one().is_empty() {
        card::foot(ui, "Your pick is offered in the conversation");
    }
    shown
}

fn decision(ui: &mut Ui, content: &Content, quest: QuestId) {
    let def = quest.get(content);
    let paths = def.decision().unwrap_or_default();
    ui.columns(paths.len().max(1), |columns| {
        for (column, path) in columns.iter_mut().zip(paths) {
            column.label(face::bold(&path.choice).size(TextRole::CardBody.size()));
            for mean in &path.means {
                column.horizontal_wrapped(|ui| {
                    glyph::show(ui, Mark::Dot, 8.0, ink::INK2);
                    ui.add(Label::new(TextRole::CardBody.rich(mean.as_str())).wrap());
                });
            }
        }
    });
}

fn hand_in_row(ui: &mut Ui, images: &mut World, content: &Content, stack: ItemStack, held: u32) {
    let short = held < stack.count;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if short { ink::RUBRIC } else { ink::INK3 };
            ui.label(
                TextRole::Small
                    .rich(format!("{held} in your bag"))
                    .color(color),
            );
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                hud::icon(ui, images, &stack.item.get(content).icon, ICON);
                ui.add(Label::new(TextRole::CardBody.rich(counted(content, stack))).wrap());
            });
        });
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    Open,
    Done,
    Failed,
}

pub(super) fn objective_row(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    objective: &Objective,
    progress: &Progress,
    standing: Standing,
) {
    let standing = match standing {
        Standing::Open if progress.done() => Standing::Done,
        other => other,
    };
    let (mark, color) = match standing {
        Standing::Open => (Mark::Flag, ink::INK),
        Standing::Done => (Mark::Check, ink::INK),
        Standing::Failed => (Mark::Cross, ink::INK3),
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if progress.counted {
                ui.label(
                    TextRole::CardBody
                        .rich(format!("{} / {}", progress.have, progress.need))
                        .color(color),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                glyph::show(ui, mark, MARK, color);
                if let Some(icon) = objective.icon(content) {
                    hud::icon(ui, images, icon, ICON);
                }
                ui.add(
                    Label::new(
                        TextRole::CardBody
                            .rich(progress.label.as_str())
                            .color(color),
                    )
                    .wrap(),
                );
            });
        });
    });
}

pub(super) fn reward_chips(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<(Response, ItemId)> {
    ui.horizontal_wrapped(|ui| {
        if def.xp > 0 {
            controls::pill(ui, &format!("{} XP", def.xp), Pill::Gold);
        }
        def.rewards()
            .iter()
            .map(|&stack| (item_chip(ui, images, content, stack), stack.item))
            .collect()
    })
    .inner
}

pub(super) fn picks(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<(Response, ItemId)> {
    ui.horizontal(|ui| {
        def.pick_one()
            .iter()
            .enumerate()
            .map(|(index, stack)| {
                let look = SlotLook {
                    icon: Some(&stack.item.get(content).icon),
                    count: stack.count,
                    ..SlotLook::EMPTY
                };
                let response = ui
                    .push_id(("pick", index), |ui| slot(ui, images, look, Sense::click()))
                    .inner;
                (response, stack.item)
            })
            .collect()
    })
    .inner
}

pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.add(Label::new(face::small_faint(text.into())).wrap());
}

fn overview(def: &QuestDef) -> String {
    let kinds = def.kinds();
    let mut letters = kinds.chars();
    let kinds = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect::<String>())
        .unwrap_or_default();
    match def.xp {
        0 => kinds,
        xp if kinds.is_empty() => format!("{xp} xp"),
        xp => format!("{kinds} · {xp} xp"),
    }
}

fn counted(content: &Content, stack: ItemStack) -> String {
    let name = &stack.item.get(content).name;
    match stack.count {
        1 => name.to_owned(),
        count => format!("{name} \u{d7}{count}"),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Countdown {
    #[default]
    Left,
    LeftDetail,
    Resets,
    ResetsDetail,
}

impl Countdown {
    pub(super) fn label(self, log: &QuestLog, quest: QuestId, now: UnixMillis) -> Option<String> {
        match self {
            Countdown::Left => log.active(quest)?.left(now).map(clock_label),
            Countdown::LeftDetail => log
                .active(quest)?
                .left(now)
                .map(|left| format!("{} left", clock_label(left))),
            Countdown::Resets => log.finished(quest)?.resets_in(now).map(resets_label),
            Countdown::ResetsDetail => log
                .finished(quest)?
                .resets_in(now)
                .map(|left| format!("Done for today · {}", resets_label(left))),
        }
    }
}

pub(super) fn clock_now() -> UnixMillis {
    crate::core::time::LocalClock::of_this_machine().now
}

pub(super) fn time_left(content: &Content, log: &QuestLog, quest: QuestId, now: UnixMillis) -> f32 {
    let left = log
        .active(quest)
        .and_then(|active| active.left(now))
        .map_or(0.0, |left| left.0);
    let limit = quest.get(content).time_limit().map_or(1.0, |limit| limit.0);
    (left / limit).clamp(0.0, 1.0)
}

fn item_chip(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    stack: ItemStack,
) -> Response {
    controls::pill_with(ui, Pill::Gold, |ui| {
        hud::icon(ui, images, &stack.item.get(content).icon, CHIP_ICON);
        ui.label(counted(content, stack));
    })
    .response
    .interact(Sense::click())
}
