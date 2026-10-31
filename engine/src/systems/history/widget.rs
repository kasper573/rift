use bevy::prelude::*;
use strum::{IntoStaticStr, VariantArray};
use ui::egui;
use ui::egui::{Color32, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::ink::{self, Ground};
use ui::{panel, sheet};

use super::{
    HistoryEntry, HistoryMark, HistoryNews, HistoryRecord, HistoryTopic, KEPT, RecordTally,
};
use crate::core::text::{self, StyledLine, StyledSpan, TextLook};
use crate::core::time::LocalClock;
use crate::systems::hud::{self, HudWindow};
use crate::systems::input::map::InputAction;
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::player::ClientId;
use crate::systems::screen::ClientScreen;
use ui::form;
use ui::words;

const SIZE: Vec2 = Vec2::new(780.0, 460.0);
const TIME_WIDTH: f32 = 72.0;
const ICON: f32 = 16.0;
const TALK_INDENT: f32 = 12.0;
const PADDING: i8 = 8;

pub struct HistoryPlugin;

impl Plugin for HistoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HistoryBook>()
            .add_systems(Update, receive_news)
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
pub enum HistoryTab {
    #[default]
    All,
    Talk,
    Quests,
    Items,
    Notifications,
    Errors,
}

impl HistoryTab {
    pub fn title(self) -> &'static str {
        self.into()
    }

    fn shows(self, entry: &HistoryEntry) -> bool {
        let topic = match self {
            HistoryTab::All => return true,
            HistoryTab::Talk => HistoryTopic::Talk,
            HistoryTab::Quests => HistoryTopic::Quest,
            HistoryTab::Items => HistoryTopic::Item,
            HistoryTab::Notifications => HistoryTopic::Notification,
            HistoryTab::Errors => HistoryTopic::Error,
        };
        entry.topic == topic || entry.mark == Some(HistoryMark::Arrived)
    }
}

#[derive(Resource, Default)]
pub struct HistoryBook {
    subject: Option<ClientId>,
    records: Vec<HistoryRecord>,
    tab: HistoryTab,
}

impl HistoryBook {
    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }

    fn upsert(&mut self, record: HistoryRecord) {
        match self
            .records
            .binary_search_by_key(&record.nth, |kept| kept.nth)
        {
            Ok(index) => self.records[index] = record,
            Err(index) => self.records.insert(index, record),
        }
        let overflow = self.records.len().saturating_sub(KEPT);
        self.records.drain(..overflow);
    }
}

pub fn shown_tab(world: &World) -> Option<HistoryTab> {
    hud::window_open(world, HistoryWindow.title()).then(|| world.resource::<HistoryBook>().tab)
}

pub struct HistoryWindow;

impl HudWindow for HistoryWindow {
    fn title(&self) -> &'static str {
        "History"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleHistory
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::History
    }
    fn size(&self) -> Vec2 {
        SIZE
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let tabs: Vec<(HistoryTab, String)> = HistoryTab::VARIANTS
            .iter()
            .map(|&tab| (tab, tab.title().to_owned()))
            .collect();
        let before = world.resource::<HistoryBook>().tab;
        let mut tab = before;
        sheet::tabs(ui, &tabs, &mut tab);
        if tab != before {
            world.resource_mut::<HistoryBook>().tab = tab;
            InterfaceSound::UiPage.play(world);
        }
        let rows = rows(world, tab);
        let clock = LocalClock::of_this_machine();
        let area = egui::ScrollArea::vertical()
            .id_salt(("history", tab.title()))
            .stick_to_bottom(true)
            .auto_shrink(false);
        sheet::scroll(ui, area, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(0, PADDING))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for row in &rows {
                        let id = ui.id().with(row.key());
                        panel::in_view(ui, id, |ui| show_row(ui, world, row, clock));
                    }
                });
        });
    }
}

/// A record's line with its tally written in, inked to read on `ground`.
pub(crate) fn tallied(
    text: &mut StyledLine,
    tally: Option<RecordTally>,
    icon: bool,
    ground: Ground,
) {
    let (base, gained, earned) = match ground {
        Ground::Paper => (ink::INK, ink::VERDIGRIS, ink::OCHRE),
        Ground::Dusk => (ink::LIGHT, ink::SPRING, ink::HONEY),
    };
    match tally {
        Some(RecordTally::Change(change)) => {
            let color = match (change > 0, icon) {
                (true, true) => gained,
                (true, false) => earned,
                (false, _) => base.gamma_multiply(0.6),
            };
            prefix(
                text,
                format!("{} ", words::signed(f64::from(change))),
                color,
            );
        }
        Some(RecordTally::Progress { have, need }) => {
            text.pieces
                .push(StyledSpan::plain(format!(" {have}/{need}")).into());
        }
        None => {}
    }
}

#[derive(Clone, PartialEq)]
enum Row {
    Trimmed,
    Empty(HistoryTab),
    Record(HistoryRecord),
}

/// What tells a row from every other, so a history laid out lazily keeps each row's height.
#[derive(Debug, Hash)]
enum RowKey {
    Trimmed,
    Empty(HistoryTab),
    Record(u64),
}

impl Row {
    fn key(&self) -> RowKey {
        match self {
            Row::Trimmed => RowKey::Trimmed,
            Row::Empty(tab) => RowKey::Empty(*tab),
            Row::Record(record) => RowKey::Record(record.nth),
        }
    }
}

fn receive_news(mut news: MessageReader<HistoryNews>, mut book: ResMut<HistoryBook>) {
    for news in news.read() {
        match news.clone() {
            HistoryNews::Backfill { subject, records } => {
                book.subject = Some(subject);
                book.records = records;
            }
            HistoryNews::Recorded { subject, record } => {
                if *book.subject.get_or_insert(subject) != subject {
                    continue;
                }
                book.upsert(record);
            }
        }
    }
}

fn forget(mut book: ResMut<HistoryBook>) {
    let tab = book.tab;
    *book = HistoryBook {
        tab,
        ..HistoryBook::default()
    };
}

fn rows(world: &World, tab: HistoryTab) -> Vec<Row> {
    let book = world.resource::<HistoryBook>();
    let mut rows = Vec::new();
    if book.records.first().is_some_and(|first| first.nth > 1) {
        rows.push(Row::Trimmed);
    }
    let shown: Vec<Row> = book
        .records
        .iter()
        .filter(|record| tab.shows(&record.entry))
        .cloned()
        .map(Row::Record)
        .collect();
    if shown.is_empty() {
        rows.push(Row::Empty(tab));
    }
    rows.extend(shown);
    rows
}

fn show_row(ui: &mut Ui, world: &mut World, row: &Row, clock: LocalClock) {
    match row {
        Row::Trimmed => form::caption(
            ui,
            format!("Older records are gone. The last {KEPT} are kept."),
            ink::INK2,
        ),
        Row::Empty(tab) => form::caption(ui, empty_text(*tab).to_owned(), ink::INK2),
        Row::Record(record) => record_row(ui, world, record, clock),
    }
}

fn empty_text(tab: HistoryTab) -> &'static str {
    match tab {
        HistoryTab::All => "Nothing yet. What you say, see and get is kept here.",
        HistoryTab::Talk => "Nothing said yet.",
        HistoryTab::Quests => "No quest news yet.",
        HistoryTab::Items => "Nothing gained or lost yet.",
        HistoryTab::Notifications => "No notifications yet.",
        HistoryTab::Errors => "No errors yet.",
    }
}

fn record_row(ui: &mut Ui, world: &mut World, record: &HistoryRecord, clock: LocalClock) {
    let entry = &record.entry;
    let time = clock.label(record.at);
    match entry.mark {
        Some(HistoryMark::Began) => {
            let who = entry
                .by
                .clone()
                .unwrap_or_else(|| "Conversation".to_owned());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(face::bold(who).color(ink::VERDIGRIS));
                ui.label(face::faint(format!("· {time}")));
            });
            return;
        }
        Some(HistoryMark::Arrived) => {
            form::divider(ui, format!("{} · {time}", entry.text.plain_words()));
            return;
        }
        _ => {}
    }
    let line = record_line(entry);
    let color = match (entry.topic, entry.mark) {
        (_, Some(HistoryMark::Failed)) | (HistoryTopic::Error, _) => ink::RUBRIC,
        _ => ink::INK,
    };
    let indent = match entry.topic {
        HistoryTopic::Talk => TALK_INDENT,
        _ => 0.0,
    };
    ui.horizontal_top(|ui| {
        ui.allocate_ui(Vec2::new(TIME_WIDTH, 0.0), |ui| {
            ui.set_min_width(TIME_WIDTH);
            form::caption(ui, time, ink::INK3);
        });
        ui.add_space(indent);
        if let Some(icon) = &entry.icon {
            hud::file_icon(ui, world, icon, ICON);
        }
        let look = TextLook::new(TextRole::Body.size(), color);
        let look = match entry.topic {
            HistoryTopic::Talk => look.quill(),
            _ => look,
        };
        text::show(ui, world, &line, look, None);
    });
}

fn record_line(entry: &HistoryEntry) -> StyledLine {
    let mut line = entry.text.styled();
    tallied(&mut line, entry.tally, entry.icon.is_some(), Ground::Paper);
    if entry.repeats > 1 {
        line.pieces.push(
            StyledSpan::plain(format!(" \u{d7}{}", entry.repeats))
                .ink(ink::INK3)
                .into(),
        );
    }
    match (entry.topic, entry.mark) {
        (HistoryTopic::Talk, Some(HistoryMark::You)) => {
            prefix(&mut line, "You  ".to_owned(), ink::VERDIGRIS);
        }
        (HistoryTopic::Talk, _) => {
            if let Some(by) = &entry.by {
                prefix(&mut line, format!("{by}  "), ink::VERDIGRIS);
            }
        }
        (HistoryTopic::Notification | HistoryTopic::Quest, _) => {
            if let Some(by) = &entry.by {
                prefix(&mut line, format!("{by}  "), ink::MAGIC);
            }
        }
        _ => {}
    }
    line
}

fn prefix(text: &mut StyledLine, label: String, color: Color32) {
    text.pieces
        .insert(0, StyledSpan::plain(label).ink(color).into());
}
