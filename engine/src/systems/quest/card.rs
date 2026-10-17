use bevy::prelude::*;
use design_tokens::{CHIP_RADIUS, HAIRLINE, TextRole, color, family, widgets};
use egui::{Align, Color32, Label, Layout, Margin, Response, RichText, Sense, Ui};

use super::{
    ActiveQuest, Objective, Parties, Progress, QuestDef, QuestId, QuestLog, clock_label,
    resets_label,
};
use crate::core::content::Content;
use crate::core::platform::ClientPlatform;
use crate::core::time::UnixMillis;
use crate::systems::dialogue::ChoiceReveal;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelHead};
use crate::systems::dialogue::{Conversation, ConversationPanel};
use crate::systems::hud;
use crate::systems::item::widget::{SlotLook, slot};
use crate::systems::item::{self, Inventory, ItemStack};
use crate::systems::player::session;

const WIDTH: f32 = 480.0;
const ICON: f32 = 20.0;
const PICK: f32 = 32.0;
const PADDING: f32 = 8.0;

pub struct QuestCardPlugin;

impl Plugin for QuestCardPlugin {
    fn build(&self, app: &mut App) {
        stage::dock(
            app,
            DockedPanel {
                head: card_head,
                show: show_card,
            },
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moment {
    Offer,
    TurnIn,
    Decision,
}

impl Moment {
    fn title(self) -> &'static str {
        match self {
            Moment::Offer => "Quest offer",
            Moment::TurnIn => "Quest complete",
            Moment::Decision => "A permanent choice",
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
    let (_, moment) = moment(world)?;
    Some(PanelHead {
        title: moment.title().to_owned(),
        width: WIDTH,
        fixed_height: None,
    })
}

fn show_card(ui: &mut Ui, world: &mut World) {
    let Some((quest, moment)) = moment(world) else {
        return;
    };
    let inventory = session::me(world)
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    egui::Frame::new().inner_margin(PADDING).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        match moment {
            Moment::Offer => offer(ui, world, quest, &inventory),
            Moment::TurnIn => turn_in(ui, world, quest, &inventory),
            Moment::Decision => decision(ui, world, quest),
        }
    });
}

fn offer(ui: &mut Ui, world: &mut World, quest: QuestId, inventory: &Inventory) {
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let def = quest.get(&content);
    let progress = def.progress(
        &content,
        &parties,
        &ActiveQuest::fresh(&content, quest),
        Some(inventory),
    );
    head(ui, world, def);
    caption(
        ui,
        format!("{} · {}", def.kinds(), def.meta(&parties, quest)),
    );
    section(ui, "Objectives");
    for (index, (objective, progress)) in def.objectives().zip(&progress).enumerate() {
        ui.push_id(("objective", index), |ui| {
            objective_row(ui, world, objective, progress, color::TEXT);
        });
    }
    if def.reveals(ChoiceReveal::Gains) {
        section(ui, "Rewards");
        reward_chips(ui, world, def);
        if !def.pick_one().is_empty() {
            section(ui, "Pick one when you return");
            picks(ui, world, def);
        }
    }
    caption(ui, "Accept or decline in the conversation");
}

fn turn_in(ui: &mut Ui, world: &mut World, quest: QuestId, inventory: &Inventory) {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    head(ui, world, def);
    if def.reveals(ChoiceReveal::Costs) {
        for (index, &stack) in def.hand_in().iter().enumerate() {
            ui.push_id(("hand in", index), |ui| {
                hand_in_row(ui, world, stack, inventory.count(stack.item));
            });
        }
    }
    if !def.reveals(ChoiceReveal::Gains) {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        caption(ui, "Rewards");
        reward_chips(ui, world, def);
        if !def.pick_one().is_empty() {
            caption(ui, "+ your pick below");
        }
    });
}

fn decision(ui: &mut Ui, world: &World, quest: QuestId) {
    let def = quest.get(world.resource::<Content>());
    ui.label(RichText::new(def.title.as_str()).font(TextRole::Heading.font_id()));
    let paths = def.decision().unwrap_or_default();
    ui.columns(paths.len().max(1), |columns| {
        for (column, path) in columns.iter_mut().zip(paths) {
            column.label(RichText::new(path.choice.as_str()).family(family::medium()));
            for mean in &path.means {
                column.label(format!("• {mean}"));
            }
        }
    });
}

fn head(ui: &mut Ui, world: &mut World, def: &QuestDef) {
    let content = world.resource::<Content>().clone();
    ui.horizontal(|ui| {
        hud::icon(ui, world, def.icon(&content), PICK);
        ui.label(RichText::new(def.title.as_str()).font(TextRole::Heading.font_id()));
    });
}

fn hand_in_row(ui: &mut Ui, world: &mut World, stack: ItemStack, held: u32) {
    let content = world.resource::<Content>().clone();
    let held_color = if held >= stack.count {
        color::MUTED
    } else {
        color::DANGER
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{held} in your bag"))
                    .size(TextRole::Small.size())
                    .color(held_color),
            );
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                caption(ui, "Hand in");
                hud::icon(ui, world, &stack.item.get(&content).icon, ICON);
                ui.add(Label::new(counted(&content, stack)).wrap());
            });
        });
    });
}

pub(super) fn objective_row(
    ui: &mut Ui,
    world: &mut World,
    objective: &Objective,
    progress: &Progress,
    color: Color32,
) {
    let content = world.resource::<Content>().clone();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if progress.counted {
                ui.label(
                    RichText::new(format!("{} / {}", progress.have, progress.need))
                        .family(family::medium())
                        .color(color),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                hud::icon(ui, world, objective.icon(&content), ICON);
                ui.add(Label::new(RichText::new(progress.label.as_str()).color(color)).wrap());
            });
        });
    });
}

pub(super) fn reward_chips(ui: &mut Ui, world: &mut World, def: &QuestDef) {
    let content = world.resource::<Content>().clone();
    ui.horizontal_wrapped(|ui| {
        if def.xp > 0 {
            widgets::chip(ui, &format!("{} XP", def.xp));
        }
        for &stack in def.rewards() {
            let chip = item_chip(ui, world, &content, stack);
            item::card::inspect(world, chip, stack.item);
        }
    });
}

pub(super) fn picks(ui: &mut Ui, world: &mut World, def: &QuestDef) {
    let content = world.resource::<Content>().clone();
    ui.horizontal(|ui| {
        for (index, stack) in def.pick_one().iter().enumerate() {
            ui.push_id(("pick", index), |ui| {
                let look = SlotLook {
                    icon: Some(&stack.item.get(&content).icon),
                    count: stack.count,
                    outline: None,
                    faded: false,
                };
                let response = slot(ui, world, look, Sense::click());
                item::card::inspect(world, response, stack.item);
            });
        }
    });
}

pub(super) fn tag(ui: &mut Ui, label: &str, color: Color32) {
    widgets::pill(ui, label, color);
}

pub(super) fn section(ui: &mut Ui, label: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(label)
            .family(family::medium())
            .color(color::MUTED),
    );
}

pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .size(TextRole::Small.size())
            .color(color::MUTED),
    );
}

pub(super) fn counted(content: &Content, stack: ItemStack) -> String {
    let name = &stack.item.get(content).name;
    match stack.count {
        1 => name.to_owned(),
        count => format!("{name} ×{count}"),
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

pub(super) fn clock_now(world: &World) -> UnixMillis {
    world.resource::<ClientPlatform>().0.local_clock().now
}

pub(super) fn time_left(content: &Content, log: &QuestLog, quest: QuestId, now: UnixMillis) -> f32 {
    let left = log
        .active(quest)
        .and_then(|active| active.left(now))
        .map_or(0.0, |left| left.0);
    let limit = quest.get(content).time_limit().map_or(1.0, |limit| limit.0);
    (left / limit).clamp(0.0, 1.0)
}

fn item_chip(ui: &mut Ui, world: &mut World, content: &Content, stack: ItemStack) -> Response {
    egui::Frame::new()
        .fill(color::RAISED)
        .stroke(HAIRLINE)
        .corner_radius(CHIP_RADIUS)
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                hud::icon(ui, world, &stack.item.get(content).icon, 16.0);
                ui.label(RichText::new(counted(content, stack)).size(12.5));
            });
        })
        .response
        .interact(Sense::click())
}
