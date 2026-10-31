use bevy::prelude::*;
use strum::IntoStaticStr;
use ui::controls::{self, Pill, RowLook};
use ui::egui;
use ui::egui::{Align, Label, Layout, Pos2, Rect, Sense, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::{self, Mark};
use ui::{card, ink, sheet};

use super::card::{
    Countdown, QuestCardLook, Standing, caption, inspectable, objective_row, picks, reward_chips,
    time_left,
};
use super::{
    ActiveQuest, FinishedQuest, Parties, QUEST_LOG_CAP, QuestId, QuestLog, QuestRequest,
    QuestResult,
};
use crate::core::content::Content;
use crate::core::time::UnixMillis;
use crate::systems::dialogue::ChoiceReveal;
use crate::systems::hud::{self, Confirm, HudWindow};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::widget::slot_note_source;
use crate::systems::item::{Inventory, ItemModuleKind, ItemStack};
use crate::systems::player::session;
use crate::systems::quest::QuestDef;
use crate::systems::screen::ClientScreen;
use ui::paint;
use ui::panel::{self, Edge};

const SIZE: Vec2 = Vec2::new(660.0, 440.0);
const QUESTS_PAGE_WIDTH: f32 = 320.0;
const LIST_WIDTH: f32 = 250.0;
const ROW_ICON: f32 = 20.0;
const DETAIL_PADDING: f32 = 10.0;
const FOOTER: f32 = 40.0;
const TIMER_BAR: Vec2 = Vec2::new(90.0, 4.0);

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
        sheet::tabs(ui, &tabs, &mut tab);
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
            now: super::card::clock_now(),
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
    let room = ui.available_rect_before_wrap();
    let list_rect = Rect::from_min_size(room.min, Vec2::new(LIST_WIDTH, room.height()));
    let detail_rect = Rect::from_min_max(
        Pos2::new(list_rect.max.x + DETAIL_PADDING, room.min.y),
        room.max,
    );
    let parted = Rect::from_min_max(
        room.min,
        Pos2::new(list_rect.max.x + DETAIL_PADDING / 2.0, room.max.y),
    );
    panel::divide(ui, parted, Edge::Right);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
        let area = egui::ScrollArea::vertical()
            .id_salt(("quest list", page.tab.title()))
            .auto_shrink(false);
        sheet::scroll(ui, area, |ui| list(ui, world, page, &listed, shown));
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(detail_rect), |ui| {
        if let Some(quest) = shown {
            detail(ui, world, page, quest);
        }
    });
    ui.allocate_rect(room, Sense::hover());
}

fn list(ui: &mut Ui, world: &mut World, page: &Page, listed: &[QuestId], shown: Option<QuestId>) {
    let content = world.resource::<Content>().clone();
    let (title, count) = match page.tab {
        LogTab::Active => (
            "Active quests",
            format!("{} of {QUEST_LOG_CAP} quests", page.log.active.len()),
        ),
        LogTab::Completed => (
            "Completed",
            match page.log.finished.len() {
                1 => "1 quest".to_owned(),
                finished => format!("{finished} quests"),
            },
        ),
    };
    sheet::section(ui, title);
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut category = None;
    for &quest in listed {
        let def = quest.get(&content);
        if category != Some(&def.category) {
            category = Some(&def.category);
            ui.add_space(6.0);
            sheet::label(ui, TextRole::Tab, &def.category.to_lowercase(), ink::INK3);
            ui.add_space(2.0);
        }
        ui.push_id(("quest row", quest.index()), |ui| {
            row(ui, world, page, quest, shown == Some(quest));
        });
    }
    ui.add_space(6.0);
    if listed.is_empty() {
        caption(ui, empty_note(page.tab));
    } else {
        caption(ui, count);
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
    let (status, tone) = status(&content, page, quest);
    let look = RowLook {
        selected,
        faint: tone == Tone::Done,
        failed: tone == Tone::Failed,
    };
    let shown = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match tone {
                    Tone::Notice => {
                        controls::pill(ui, &status, Pill::Azure);
                    }
                    Tone::Done => {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(44.0, 18.0), Sense::hover());
                        controls::stamp(ui.painter(), rect.center(), "done", ink::INK);
                    }
                    Tone::Failed => {
                        ui.label(TextRole::Small.rich(status.as_str()).color(ink::RUBRIC));
                    }
                    Tone::Plain => {
                        ui.label(TextRole::Small.rich(status.as_str()).color(ink::INK3));
                    }
                }
                if page.log.tracked.contains(&quest) {
                    glyph::show(ui, Mark::Flag, 12.0, ink::AZURE);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    hud::icon(ui, world, def.icon(&content), ROW_ICON);
                    ui.add(Label::new(face::bold(def.title.as_str())).truncate());
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            &def.title,
        )
    });
    if map::clicked(world, InputAction::Select, &response) {
        select(world, page.tab, quest);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Plain,
    Notice,
    Done,
    Failed,
}

fn status(content: &Content, page: &Page, quest: QuestId) -> (String, Tone) {
    let def = quest.get(content);
    if let Some(active) = page.active(quest) {
        if def.decision().is_some() {
            return ("choice".to_owned(), Tone::Notice);
        }
        if active.ready(content, &page.parties, Some(&page.inventory)) {
            return ("ready".to_owned(), Tone::Notice);
        }
        if active.deadline.is_some() {
            let left = Countdown::Left
                .label(&page.log, quest, page.now)
                .unwrap_or_default();
            return (left, Tone::Plain);
        }
        if def.daily() {
            return ("daily".to_owned(), Tone::Plain);
        }
        let counted = def
            .progress(content, &page.parties, active, Some(&page.inventory))
            .into_iter()
            .find(|progress| progress.counted && !progress.done())
            .map(|progress| format!("{}/{}", progress.have, progress.need))
            .unwrap_or_default();
        return (counted, Tone::Plain);
    }
    match page.finished(quest) {
        Some(finished) if finished.result == QuestResult::Failed => {
            ("failed".to_owned(), Tone::Failed)
        }
        Some(finished) if finished.resets_in(page.now).is_some() => (
            Countdown::Resets
                .label(&page.log, quest, page.now)
                .unwrap_or_default(),
            Tone::Plain,
        ),
        Some(_) => ("done".to_owned(), Tone::Done),
        None => (String::new(), Tone::Plain),
    }
}

fn detail(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let trackable = page.log.trackable(&content, quest);
    let footer = if trackable { FOOTER } else { 0.0 };
    let look = QuestCardLook {
        width: QUESTS_PAGE_WIDTH.min(ui.available_width()),
        max_height: (ui.available_height() - footer).max(card::KIND_BAND),
        moment: None,
    };
    ui.vertical_centered(|ui| {
        ui.set_width(look.width);
        super::card::show(ui, world, &content, quest, look, |ui, world| {
            plate(ui, world, page, quest)
        });
        if trackable {
            footer_row(ui, world, page, quest);
        }
    });
}

fn plate(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    let content = world.resource::<Content>().clone();
    let def = quest.get(&content);
    card::overview(ui, &def.meta(&page.parties, quest));
    if page
        .active(quest)
        .is_some_and(|active| active.deadline.is_some())
    {
        timer(ui, &content, page, quest);
    }
    card::flavour(ui, &def.blurb);
    card::heading(ui, "objectives");
    let fresh = ActiveQuest::fresh(&content, quest);
    let progress = def.progress(
        &content,
        &page.parties,
        page.active(quest).unwrap_or(&fresh),
        Some(&page.inventory),
    );
    let finished = page.finished(quest);
    for (index, (objective, mut progress)) in def.objectives().zip(progress).enumerate() {
        let standing = match finished.map(|finished| finished.result) {
            Some(QuestResult::Completed) => Standing::Done,
            Some(QuestResult::Failed) => Standing::Failed,
            None => Standing::Open,
        };
        progress.counted &= finished.is_none();
        ui.push_id(("objective", index), |ui| {
            objective_row(ui, world, &content, objective, &progress, standing);
        });
    }
    if let Some(paths) = def.decision() {
        card::heading(ui, "choices");
        for path in paths {
            card::fact(ui, &format!("{}: {}", path.choice, path.means.join("; ")));
        }
    }
    let handed_in = finished.is_some_and(|finished| finished.result == QuestResult::Completed);
    if def.reveals(ChoiceReveal::Gains) || handed_in {
        card::heading(ui, "rewards");
        let shown = reward_chips(ui, world, &content, def);
        inspectable(world, shown);
    }
    if def.reveals(ChoiceReveal::Gains) && !def.pick_one().is_empty() && finished.is_none() {
        card::heading(ui, "pick one when you return");
        let shown = picks(ui, world, &content, def);
        inspectable(world, shown);
    }
    if def.time_limit().is_some() {
        card::heading(ui, "if time runs out");
        card::fact(
            ui,
            &format!("The quest fails. {}.", def.retry_hint(&page.parties, quest)),
        );
    }
    if let Some(finished) = finished {
        let (outcome, color) = match finished.result {
            QuestResult::Completed if finished.resets_in(page.now).is_some() => (
                Countdown::ResetsDetail
                    .label(&page.log, quest, page.now)
                    .unwrap_or_default(),
                ink::VERDIGRIS,
            ),
            QuestResult::Completed => ("Completed".to_owned(), ink::VERDIGRIS),
            QuestResult::Failed if finished.offered_again => (
                format!("Failed · {}", def.retry_hint(&page.parties, quest)),
                ink::RUBRIC,
            ),
            QuestResult::Failed => ("Failed".to_owned(), ink::RUBRIC),
        };
        ui.add(
            Label::new(
                face::bold(&outcome)
                    .size(TextRole::CardBody.size())
                    .color(color),
            )
            .wrap(),
        );
    }
}

fn timer(ui: &mut Ui, content: &Content, page: &Page, quest: QuestId) {
    ui.horizontal(|ui| {
        let left = Countdown::LeftDetail
            .label(&page.log, quest, page.now)
            .unwrap_or_default();
        ui.label(
            face::bold(&left)
                .size(TextRole::CardBody.size())
                .color(ink::OCHRE),
        );
        let share = time_left(content, &page.log, quest, page.now);
        paint::progress_line(ui, TIMER_BAR, share, ink::OCHRE);
    });
}

fn footer_row(ui: &mut Ui, world: &mut World, page: &Page, quest: QuestId) {
    ui.add_space(8.0);
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
                if controls::danger(ui, "Abandon").clicked() {
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
            answer: Box::new(move |world, abandoned| {
                if abandoned {
                    world.write_message(QuestRequest::Abandon { quest });
                }
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
    if !stack.item.get(content).has(ItemModuleKind::QuestItem) {
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
