use bevy::prelude::*;
use design_tokens::{TextRole, color, family, widgets};
use egui::{Align, Color32, Label, Layout, RichText, Sense, Ui, Vec2};
use strum::IntoStaticStr;

use super::card::{
    Countdown, caption, objective_row, picks, reward_chips, section, tag, time_left,
};
use super::{
    ActiveQuest, FinishedQuest, Parties, QUEST_LOG_CAP, QuestId, QuestLog, QuestRequest,
    QuestResult,
};
use crate::core::content::{Content, Fixture};
use crate::core::time::UnixMillis;
use crate::systems::dialogue::ChoiceReveal;
use crate::systems::hud::{self, Confirm, HudWindow};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::widget::slot_note_source;
use crate::systems::item::{Inventory, ItemFlag, ItemStack};
use crate::systems::player::session;
use crate::systems::quest::QuestDef;
use crate::systems::screen::ClientScreen;

const SIZE: Vec2 = Vec2::new(660.0, 440.0);
const LIST_WIDTH: f32 = 250.0;
const ROW_ICON: f32 = 20.0;
const DETAIL_PADDING: f32 = 8.0;

pub struct QuestLogPlugin;

impl Plugin for QuestLogPlugin {
    fn build(&self, app: &mut App) {
        slot_note_source(app, quest_note);
        app.init_resource::<Browse>()
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub struct QuestLogWindow;

impl HudWindow for QuestLogWindow {
    fn title(&self) -> &'static str {
        "Quests"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleQuestLog
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::QuestLog
    }
    fn size(&self) -> Vec2 {
        SIZE
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let tabs = [LogTab::Active, LogTab::Completed].map(|tab| (tab, tab.title().to_owned()));
        let mut tab = world.resource::<Browse>().tab;
        widgets::tab_strip(ui, &tabs, &mut tab);
        world.resource_mut::<Browse>().tab = tab;
        let page = Page::of(world, tab);
        page_ui(ui, world, &page);
    }
}

pub fn select(world: &mut World, tab: LogTab, quest: QuestId) {
    let mut browse = world.resource_mut::<Browse>();
    match tab {
        LogTab::Active => browse.active = Some(quest),
        LogTab::Completed => browse.completed = Some(quest),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, IntoStaticStr)]
pub enum LogTab {
    #[default]
    Active,
    Completed,
}

impl LogTab {
    fn title(self) -> &'static str {
        self.into()
    }
}

#[derive(Resource, Default)]
struct Browse {
    tab: LogTab,
    active: Option<QuestId>,
    completed: Option<QuestId>,
}

impl Browse {
    fn of(&self, tab: LogTab) -> Option<QuestId> {
        match tab {
            LogTab::Active => self.active,
            LogTab::Completed => self.completed,
        }
    }
}

struct Page {
    parties: Parties,
    tab: LogTab,
    log: QuestLog,
    inventory: Inventory,
    now: UnixMillis,
    selected: Option<QuestId>,
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

impl Page {
    fn of(world: &World, tab: LogTab) -> Page {
        let seen = session::me(world);
        Page {
            parties: world.resource::<Parties>().clone(),
            tab,
            log: seen
                .and_then(|seen| world.get::<QuestLog>(seen))
                .cloned()
                .unwrap_or_default(),
            inventory: seen
                .and_then(|seen| world.get::<Inventory>(seen))
                .cloned()
                .unwrap_or_else(Inventory::empty),
            now: super::card::clock_now(world),
            selected: world.resource::<Browse>().of(tab),
        }
    }

    fn listed(&self, content: &Content) -> Vec<QuestId> {
        let mut quests: Vec<QuestId> = match self.tab {
            LogTab::Active => self.log.active.iter().map(|active| active.quest).collect(),
            LogTab::Completed => self
                .log
                .finished
                .iter()
                .map(|finished| finished.quest)
                .collect(),
        };
        let first_seen = |category: &str| {
            content
                .table::<QuestDef>()
                .rows()
                .position(|quest| quest.category == category)
        };
        quests.sort_by_key(|quest| (first_seen(&quest.get(content).category), quest.index()));
        quests
    }

    fn shown(&self, listed: &[QuestId]) -> Option<QuestId> {
        self.selected
            .filter(|selected| listed.contains(selected))
            .or_else(|| listed.first().copied())
    }

    fn active(&self, quest: QuestId) -> Option<&ActiveQuest> {
        self.log.active(quest)
    }

    fn finished(&self, quest: QuestId) -> Option<&FinishedQuest> {
        self.log.finished(quest)
    }
}

fn page_ui(ui: &mut Ui, world: &mut World, page: &Page) {
    let content = world.resource::<Content>().clone();
    let listed = page.listed(&content);
    let shown = page.shown(&listed);
    egui::Panel::left(egui::Id::new(("quest list", page.tab.title())))
        .resizable(false)
        .exact_size(LIST_WIDTH)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| list(ui, world, page, &listed, shown));
        });
    egui::ScrollArea::vertical()
        .id_salt(("quest detail", page.tab.title()))
        .auto_shrink(false)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(DETAIL_PADDING)
                .show(ui, |ui| {
                    if let Some(quest) = shown {
                        detail(ui, world, page, quest);
                    }
                });
        });
}

fn list(ui: &mut Ui, world: &mut World, page: &Page, listed: &[QuestId], shown: Option<QuestId>) {
    let content = world.resource::<Content>().clone();
    let (title, aside) = match page.tab {
        LogTab::Active => (
            "Active quests",
            format!("{} / {QUEST_LOG_CAP}", page.log.active.len()),
        ),
        LogTab::Completed => ("Completed", page.log.finished.len().to_string()),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).family(family::medium()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(aside).color(color::MUTED));
        });
    });
    let mut category = None;
    for &quest in listed {
        let def = quest.get(&content);
        if category != Some(&def.category) {
            category = Some(&def.category);
            ui.add_space(6.0);
            ui.label(
                RichText::new(def.category.to_uppercase())
                    .size(TextRole::Small.size())
                    .family(family::medium())
                    .color(color::FAINT),
            );
        }
        ui.push_id(("quest row", quest.index()), |ui| {
            row(ui, world, page, quest, shown == Some(quest));
        });
    }
    if listed.is_empty() {
        ui.add_space(8.0);
        caption(ui, empty_note(page.tab));
    }
}

fn empty_note(tab: LogTab) -> &'static str {
    match tab {
        LogTab::Active => "No quests yet. Look for a ! over someone's head.",
        LogTab::Completed => "Nothing finished yet",
    }
}

fn row(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId, selected: bool) {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    let fill = if selected {
        color::ACCENT_SOFT
    } else {
        Color32::TRANSPARENT
    };
    let response = egui::Frame::new()
        .fill(fill)
        .corner_radius(design_tokens::WIDGET_RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if page.log.tracked.contains(&quest) {
                        hud::icon(ui, world, &InterfaceIcon::Tracked.get(&content).image, 14.0);
                    }
                    let (status, color) = status(&content, page, quest);
                    ui.label(
                        RichText::new(status)
                            .size(TextRole::Small.size())
                            .color(color),
                    );
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        hud::icon(ui, world, def.icon(&content), ROW_ICON);
                        ui.add(
                            Label::new(RichText::new(def.title.as_str()).family(family::medium()))
                                .truncate(),
                        );
                    });
                });
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            &def.title,
        )
    });
    if response.hovered() && !selected {
        ui.painter().rect_filled(
            response.rect,
            design_tokens::WIDGET_RADIUS,
            Color32::from_white_alpha(8),
        );
    }
    if map::clicked(world, InputAction::Select, &response) {
        select(world, page.tab, quest);
    }
}

fn status(content: &Content, page: &Page, quest: QuestId) -> (String, Color32) {
    let ink = color::TEXT.gamma_multiply(0.7);
    let def = quest.get(content);
    if let Some(active) = page.active(quest) {
        if def.decision().is_some() {
            return ("choice".to_owned(), color::WARN);
        }
        if active.ready(content, &page.parties, Some(&page.inventory)) {
            return ("ready".to_owned(), color::WARN);
        }
        if active.deadline.is_some() {
            let left = Countdown::Left
                .label(&page.log, quest, page.now)
                .unwrap_or_default();
            return (left, color::WARN);
        }
        let counted = def
            .progress(content, &page.parties, active, Some(&page.inventory))
            .into_iter()
            .find(|progress| progress.counted && !progress.done())
            .map(|progress| format!("{}/{}", progress.have, progress.need))
            .unwrap_or_default();
        return (counted, ink);
    }
    match page.finished(quest) {
        Some(finished) if finished.result == QuestResult::Failed => {
            ("failed".to_owned(), color::DANGER)
        }
        Some(finished) if finished.resets_in(page.now).is_some() => (
            Countdown::Resets
                .label(&page.log, quest, page.now)
                .unwrap_or_default(),
            ink,
        ),
        Some(_) => ("done".to_owned(), ink),
        None => (String::new(), ink),
    }
}

fn detail(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(def.title.as_str()).font(TextRole::Heading.font_id()));
        tag(ui, &def.kinds(), color::WARN);
    });
    caption(ui, def.meta(&page.parties, quest));
    if page
        .active(quest)
        .is_some_and(|active| active.deadline.is_some())
    {
        timer(ui, world, page, quest);
    }
    ui.label(
        RichText::new(format!("\u{201c}{}\u{201d}", def.blurb))
            .color(color::TEXT.gamma_multiply(0.85)),
    );
    section(ui, "Objectives");
    let fresh = ActiveQuest::fresh(&content, quest);
    let progress = def.progress(
        &content,
        &page.parties,
        page.active(quest).unwrap_or(&fresh),
        Some(&page.inventory),
    );
    let finished = page.finished(quest);
    for (index, (objective, mut progress)) in def.objectives().zip(progress).enumerate() {
        let color = match finished.map(|finished| finished.result) {
            Some(QuestResult::Completed) => color::OK,
            Some(QuestResult::Failed) => color::MUTED,
            None if progress.done() => color::OK,
            None => color::TEXT,
        };
        progress.counted &= finished.is_none();
        ui.push_id(("objective", index), |ui| {
            objective_row(ui, world, objective, &progress, color);
        });
    }
    if let Some(paths) = def.decision() {
        section(ui, "Choices");
        for path in paths {
            ui.label(format!("{}: {}", path.choice, path.means.join("; ")));
        }
    }
    let handed_in = finished.is_some_and(|finished| finished.result == QuestResult::Completed);
    if def.reveals(ChoiceReveal::Gains) || handed_in {
        section(ui, "Rewards");
        reward_chips(ui, world, def);
    }
    if def.reveals(ChoiceReveal::Gains) && !def.pick_one().is_empty() && finished.is_none() {
        section(ui, "Pick one when you return");
        picks(ui, world, def);
    }
    if def.time_limit().is_some() {
        section(ui, "If time runs out");
        ui.label(format!(
            "The quest fails. {}.",
            def.retry_hint(&page.parties, quest)
        ));
    }
    if let Some(finished) = finished {
        let (outcome, color) = match finished.result {
            QuestResult::Completed if finished.resets_in(page.now).is_some() => (
                Countdown::ResetsDetail
                    .label(&page.log, quest, page.now)
                    .unwrap_or_default(),
                color::OK,
            ),
            QuestResult::Completed => ("Completed".to_owned(), color::OK),
            QuestResult::Failed if finished.offered_again => (
                format!("Failed · {}", def.retry_hint(&page.parties, quest)),
                color::DANGER,
            ),
            QuestResult::Failed => ("Failed".to_owned(), color::DANGER),
        };
        ui.label(RichText::new(outcome).family(family::medium()).color(color));
    }
    if page.log.trackable(&content, quest) {
        footer(ui, world, page, quest);
    }
}

fn timer(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    ui.horizontal(|ui| {
        hud::icon(ui, world, &InterfaceIcon::Timed.get(&content).image, 16.0);
        let left = Countdown::LeftDetail
            .label(&page.log, quest, page.now)
            .unwrap_or_default();
        ui.label(
            RichText::new(left)
                .family(family::medium())
                .color(color::WARN),
        );
        ui.add(
            egui::ProgressBar::new(time_left(&content, &page.log, quest, page.now))
                .desired_height(6.0)
                .fill(color::WARN),
        );
    });
}

fn footer(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let tracked = page.log.tracked.contains(&quest);
        let mut track = tracked;
        if ui.checkbox(&mut track, "Track on HUD").changed() {
            world.write_message(QuestRequest::Track {
                quest,
                tracked: track,
            });
        }
        if page.active(quest).is_some() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::danger(ui, "Abandon").clicked() {
                    confirm_abandon(world, quest);
                }
            });
        }
    });
}

fn confirm_abandon(world: &mut World, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    let taken = def
        .grants()
        .iter()
        .map(|stack| format!("{} is taken from your bag.", stack.item.get(&content).name));
    let again = match world.resource::<Parties>().giver(quest) {
        Some(party) if party.item.is_some() => {
            format!("Use the {} to start it again.", party.name)
        }
        Some(party) => format!("{} will offer the quest again.", party.name),
        None => "Nobody offers it again.".to_owned(),
    };
    let body = taken
        .chain(["Your progress is lost.".to_owned(), again])
        .collect();
    hud::confirm(
        world,
        Confirm {
            title: format!("Abandon {}?", def.title),
            body,
            confirm: "Abandon quest".to_owned(),
            cancel: "Keep quest".to_owned(),
            on_confirm: Box::new(move |world| {
                world.write_message(QuestRequest::Abandon { quest });
            }),
        },
    );
}

fn quest_note(world: &World, stack: ItemStack) -> Option<String> {
    let content = world.resource::<Content>();
    let parties = world.resource::<Parties>();
    if content.table::<QuestDef>().ids().any(|quest| {
        parties
            .giver(quest)
            .is_some_and(|party| party.item == Some(stack.item))
    }) {
        return Some("begins a quest · use to read it".to_owned());
    }
    if !stack.item.get(content).has(ItemFlag::Quest) {
        return None;
    }
    let granted_by = session::me(world)
        .and_then(|seen| world.get::<QuestLog>(seen))
        .and_then(|log| {
            log.active.iter().find(|active| {
                active
                    .quest
                    .get(content)
                    .grants()
                    .iter()
                    .any(|grant| grant.item == stack.item)
            })
        })
        .map(|active| active.quest.get(content).title.as_str());
    Some(match granted_by {
        Some(title) => {
            format!("quest item · can't be sold or dropped · taken back if you abandon {title}")
        }
        None => "quest item · can't be sold or dropped".to_owned(),
    })
}
