use bevy::prelude::*;
use design_tokens::{TextRole, color, family, ink, widgets};
use egui::{Color32, Id, Rect, RichText, Ui, Vec2};
use strum::{IntoStaticStr, VariantArray};

use super::{
    HistoryEntry, HistoryMark, HistoryNews, HistoryRecord, HistoryTopic, KEPT, RecordTally, signed,
};
use crate::core::platform::ClientPlatform;
use crate::core::text::{self, StyledLine, StyledSpan, TextLook};
use crate::core::time::LocalClock;
use crate::systems::hud::{self, HudWindow};
use crate::systems::input::map::InputAction;
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::player::ClientId;
use crate::systems::screen::ClientScreen;

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
        widgets::tab_strip(ui, &tabs, &mut tab);
        if tab != before {
            world.resource_mut::<HistoryBook>().tab = tab;
            InterfaceSound::UiPage.play(world);
        }
        let rows = rows(world, tab);
        let clock = world.resource::<ClientPlatform>().0.local_clock();
        egui::ScrollArea::vertical()
            .id_salt(("history", tab.title()))
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(0, PADDING))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for (index, row) in rows.iter().enumerate() {
                            let id = ui.id().with(index);
                            lazily(ui, id, row, |ui| show_row(ui, world, row, clock));
                        }
                    });
            });
    }
}

pub(crate) fn tallied(text: &mut StyledLine, tally: Option<RecordTally>, icon: bool, ink: Color32) {
    match tally {
        Some(RecordTally::Change(change)) => {
            let color = match (change > 0, icon) {
                (true, true) => color::OK,
                (true, false) => color::WARN,
                (false, _) => ink.gamma_multiply(0.6),
            };
            prefix(text, format!("{} ", signed(change)), color);
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

// A history holds hundreds of rows, so only those in sight are laid out; the rest keep the height
// they had when last drawn, for as long as they and the width they wrap to stay the same.
#[derive(Clone)]
struct Measured {
    row: Row,
    width: f32,
    height: f32,
}

fn lazily(ui: &mut Ui, id: Id, row: &Row, add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let known = ui
        .data(|data| data.get_temp::<Measured>(id))
        .filter(|measured| measured.row == *row && measured.width == width);
    if let Some(measured) = known {
        let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(width, measured.height));
        if !ui.is_rect_visible(rect) {
            ui.allocate_space(rect.size());
            return;
        }
    }
    let height = ui.scope(add).response.rect.height();
    let measured = Measured {
        row: row.clone(),
        width,
        height,
    };
    ui.data_mut(|data| data.insert_temp(id, measured));
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
        Row::Trimmed => caption(
            ui,
            format!("Older records are gone. The last {KEPT} are kept."),
            color::MUTED,
        ),
        Row::Empty(tab) => caption(ui, empty_text(*tab).to_owned(), color::MUTED),
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

fn caption(ui: &mut Ui, text: String, color: Color32) {
    ui.label(
        RichText::new(text)
            .size(TextRole::Small.size())
            .color(color),
    );
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
            ui.label(
                RichText::new(format!("{who} · {time}"))
                    .family(family::medium())
                    .color(color::TEXT.gamma_multiply(0.75)),
            );
            return;
        }
        Some(HistoryMark::Arrived) => {
            divider(ui, format!("{} · {time}", entry.text.plain_words()));
            return;
        }
        _ => {}
    }
    let line = record_line(entry);
    let color = match (entry.topic, entry.mark) {
        (_, Some(HistoryMark::Failed)) | (HistoryTopic::Error, _) => color::DANGER,
        _ => color::TEXT,
    };
    let indent = match entry.topic {
        HistoryTopic::Talk => TALK_INDENT,
        _ => 0.0,
    };
    ui.horizontal_top(|ui| {
        ui.allocate_ui(Vec2::new(TIME_WIDTH, 0.0), |ui| {
            ui.set_min_width(TIME_WIDTH);
            caption(ui, time, color::FAINT);
        });
        ui.add_space(indent);
        if let Some(icon) = &entry.icon {
            hud::file_icon(ui, world, icon, ICON);
        }
        let look = TextLook {
            size: TextRole::Body.size(),
            color,
        };
        text::show(ui, world, &line, look, None);
    });
}

fn record_line(entry: &HistoryEntry) -> StyledLine {
    let mut line = entry.text.styled();
    tallied(&mut line, entry.tally, entry.icon.is_some(), color::TEXT);
    if entry.repeats > 1 {
        line.pieces.push(
            StyledSpan::plain(format!(" \u{d7}{}", entry.repeats))
                .ink(color::MUTED)
                .into(),
        );
    }
    match (entry.topic, entry.mark) {
        (HistoryTopic::Talk, Some(HistoryMark::You)) => {
            prefix(&mut line, "You  ".to_owned(), ink::PLACE);
        }
        (HistoryTopic::Talk, _) => {
            if let Some(by) = &entry.by {
                prefix(&mut line, format!("{by}  "), color::WARN);
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

fn divider(ui: &mut Ui, text: String) {
    let font = egui::FontId::proportional(TextRole::Small.size());
    let text_width = ui
        .painter()
        .layout_no_wrap(text.clone(), font, Color32::WHITE)
        .size()
        .x;
    let gap = ui.spacing().item_spacing.x;
    let lead = ((ui.available_width() - text_width) / 2.0 - gap).max(0.0);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        rule(ui, lead);
        caption(ui, text, color::TEXT.gamma_multiply(0.7));
        let rest = ui.available_width();
        rule(ui, rest);
    });
    ui.add_space(4.0);
}

fn rule(ui: &mut Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, color::TEXT.gamma_multiply(0.25));
}
