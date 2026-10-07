use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use strum::VariantArray;
use ui::tokens::{palette, spacing, typography};
use ui::{RichSpan, RichText, component};

use super::{
    HistoryEntry, HistoryMark, HistoryNews, HistoryRecord, HistoryTopic, KEPT, RecordTally,
};
use crate::core::audio::playback::SfxId;
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::platform::ClientPlatform;
use crate::core::time::LocalClock;
use crate::systems::actor::Name;
use crate::systems::hud::{self, HudAudience, Window, reconcile_children};
use crate::systems::input::map::InputAction;
use crate::systems::player::ClientId;
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;
use crate::systems::text::LineText;

pub const HISTORY_WINDOW: &str = "History";
const SIZE: Vec2 = Vec2::new(780.0, 460.0);
const TIME_WIDTH: f32 = 72.0;
const ICON: f32 = 16.0;
const TALK_INDENT: f32 = 12.0;

pub struct HistoryPlugin;

impl Plugin for HistoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HistoryBook>()
            .add_systems(Update, receive_news)
            .add_systems(OnExit(GameScene::Area), forget)
            .add_observer(remember_tab);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, VariantArray)]
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
        match self {
            HistoryTab::All => "All",
            HistoryTab::Talk => "Talk",
            HistoryTab::Quests => "Quests",
            HistoryTab::Items => "Items",
            HistoryTab::Notifications => "Notifications",
            HistoryTab::Errors => "Errors",
        }
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

    fn index(self) -> usize {
        HistoryTab::VARIANTS
            .iter()
            .position(|&tab| tab == self)
            .unwrap_or(0)
    }
}

#[derive(Resource, Default)]
pub struct HistoryBook {
    subject: Option<ClientId>,
    records: Vec<HistoryRecord>,
    version: u64,
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

pub fn close(world: &mut World) -> bool {
    hud::close_window(world, HISTORY_WINDOW)
}

pub fn shown_tab(world: &World) -> Option<HistoryTab> {
    hud::window_open(world, HISTORY_WINDOW).then(|| world.resource::<HistoryBook>().tab)
}

pub struct HistoryWindow;

impl Window for HistoryWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Everyone
    }
    fn title(&self) -> &'static str {
        HISTORY_WINDOW
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleHistory
    }
    fn icon(&self) -> &'static str {
        "icons/misc/book_3.png"
    }
    fn order(&self) -> u32 {
        4
    }
    fn size(&self) -> Vec2 {
        SIZE
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        HistoryTab::VARIANTS
            .iter()
            .map(|&tab| ui::WindowContent {
                title: tab.title().into(),
                scene: Box::new(body(tab)),
            })
            .collect()
    }
    fn tab(&self, world: &World) -> usize {
        world.resource::<HistoryBook>().tab.index()
    }
    fn sync(&self, world: &mut World) {
        sync_bodies(world);
    }
}

#[derive(Component, Clone, Default)]
struct HistoryBody {
    tab: HistoryTab,
    built: Option<u64>,
}

#[derive(Component, Default, Clone)]
struct HistoryList;

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
        book.version += 1;
    }
}

fn forget(mut book: ResMut<HistoryBook>) {
    let tab = book.tab;
    *book = HistoryBook {
        tab,
        ..HistoryBook::default()
    };
}

fn remember_tab(
    changed: On<ui::WindowTabChanged>,
    bodies: Query<Entity, With<HistoryBody>>,
    parents: Query<&ChildOf>,
    mut book: ResMut<HistoryBook>,
    mut sounds: MessageWriter<PlaySfx>,
) {
    let ours = bodies
        .iter()
        .any(|body| parents.iter_ancestors(body).any(|up| up == changed.window));
    let Some(&tab) = HistoryTab::VARIANTS.get(changed.tab) else {
        return;
    };
    if ours && book.tab != tab {
        book.tab = tab;
        sounds.write(PlaySfx {
            id: SfxId::UiPage,
            place: SfxPlace::Interface,
        });
    }
}

fn body(tab: HistoryTab) -> impl Scene {
    bsn! {
        HistoryBody { tab: {tab}, built: None }
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Children [
            ( {ui::scroll_area()}
              Children [
                ( {ui::scroll_viewport()}
                  {ui::component(ui::PinToBottom::default())}
                  Children [
                    (
                        HistoryList
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px({spacing::S}),
                            width: Val::Percent(100.0),
                            padding: {UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L))},
                        }
                    )
                  ]
                ),
                ( {ui::scroll_bar()} Children [ {EntityScene(ui::scroll_thumb())} ] )
              ]
            )
        ]
    }
}

enum Row {
    Watching { name: String, count: usize },
    Trimmed,
    Empty(HistoryTab),
    Record(HistoryRecord),
}

impl Row {
    fn key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        match self {
            Row::Watching { name, count } => ("watching", name, count).hash(&mut hasher),
            Row::Trimmed => "trimmed".hash(&mut hasher),
            Row::Empty(tab) => ("empty", tab.index()).hash(&mut hasher),
            Row::Record(record) => {
                (
                    "record",
                    record.nth,
                    format!("{:?}", record.entry.tally),
                    record.entry.repeats,
                )
                    .hash(&mut hasher);
            }
        }
        hasher.finish()
    }
}

fn sync_bodies(world: &mut World) {
    let bodies: Vec<(Entity, HistoryBody)> = world
        .query::<(Entity, &HistoryBody)>()
        .iter(world)
        .map(|(entity, body)| (entity, body.clone()))
        .collect();
    let version = world.resource::<HistoryBook>().version;
    let watched = watched_name(world);
    let mut built_key = DefaultHasher::new();
    (version, &watched).hash(&mut built_key);
    let built_key = built_key.finish();
    for (entity, body) in bodies {
        if body.built == Some(built_key) {
            continue;
        }
        let Some(list) = descendant_with::<HistoryList>(world, entity) else {
            continue;
        };
        if let Some(mut body) = world.get_mut::<HistoryBody>(entity) {
            body.built = Some(built_key);
        }
        let rows = rows(world, body.tab, watched.clone());
        let keys: Vec<u64> = rows.iter().map(Row::key).collect();
        let clock = world.resource::<ClientPlatform>().0.local_clock();
        reconcile_children(world, list, &keys, |world, index| {
            row(world.resource::<AssetServer>(), &rows[index], clock)
        });
    }
}

fn rows(world: &World, tab: HistoryTab, watched: Option<String>) -> Vec<Row> {
    let book = world.resource::<HistoryBook>();
    let mut rows = Vec::new();
    if let Some(name) = watched {
        rows.push(Row::Watching {
            name,
            count: book.records.len(),
        });
    }
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

fn watched_name(world: &World) -> Option<String> {
    if *world.resource::<Mode>() != Mode::Spectate {
        return None;
    }
    world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<Name>(seen))
        .map(|name| name.name.clone())
}

fn row(assets: &AssetServer, row: &Row, clock: LocalClock) -> Box<dyn Scene> {
    let ink = ui::theme::theme().surface_floating.on;
    match row {
        Row::Watching { name, count } => Box::new(note(
            format!("Watching {name} · {count} records"),
            ink,
            true,
        )),
        Row::Trimmed => Box::new(note(
            format!("Older records are gone. The last {KEPT} are kept."),
            ink.with_alpha(0.6),
            false,
        )),
        Row::Empty(tab) => Box::new(note(
            empty_text(*tab).to_owned(),
            ink.with_alpha(0.6),
            false,
        )),
        Row::Record(record) => record_row(assets, record, clock),
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

fn note(text: String, ink: Color, bar: bool) -> impl Scene {
    let surface = ui::theme::theme().surface_inset;
    let frame = ui::Style::new()
        .background(if bar { surface.base } else { Color::NONE })
        .node(|node| {
            node.width = Val::Percent(100.0);
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M));
            node.border_radius = BorderRadius::all(Val::Px(ui::tokens::radius::S));
        });
    bsn! {
        template_value(frame)
        Children [ {EntityScene(ui::styled_text(text, ink, typography::CAPTION))} ]
    }
}

fn record_row(assets: &AssetServer, record: &HistoryRecord, clock: LocalClock) -> Box<dyn Scene> {
    let entry = &record.entry;
    let ink = ui::theme::theme().surface_floating.on;
    let time = clock.label(record.at);
    match entry.mark {
        Some(HistoryMark::Began) => {
            let who = entry
                .by
                .clone()
                .unwrap_or_else(|| "Conversation".to_owned());
            return Box::new(header(format!("{who} · {time}"), ink));
        }
        Some(HistoryMark::Arrived) => {
            return Box::new(divider(format!("{} · {time}", words(&entry.text)), ink));
        }
        _ => {}
    }
    let mut text = entry.text.rich();
    let prefix = |text: &mut RichText, label: String, color: Color| {
        text.pieces
            .insert(0, RichSpan::plain(label).color(color).into());
    };
    match entry.tally {
        Some(RecordTally::Change(change)) => {
            let (sign, color) = match change > 0 {
                true => ("+", palette::EMERALD_80),
                false => ("\u{2212}", ink.with_alpha(0.6)),
            };
            let color = match entry.icon.is_none() && change > 0 {
                true => palette::AMBER_80,
                false => color,
            };
            prefix(
                &mut text,
                format!("{sign}{} ", change.unsigned_abs()),
                color,
            );
        }
        Some(RecordTally::Progress { have, need }) => {
            text.pieces
                .push(RichSpan::plain(format!(" {have}/{need}")).into());
        }
        None => {}
    }
    if entry.repeats > 1 {
        text.pieces.push(
            RichSpan::plain(format!(" \u{d7}{}", entry.repeats))
                .color(ink.with_alpha(0.6))
                .into(),
        );
    }
    match (entry.topic, entry.mark) {
        (HistoryTopic::Talk, Some(HistoryMark::You)) => {
            prefix(&mut text, "You  ".to_owned(), palette::AZURE_80);
        }
        (HistoryTopic::Talk, _) => {
            if let Some(by) = &entry.by {
                prefix(&mut text, format!("{by}  "), palette::AMBER_80);
            }
        }
        (HistoryTopic::Notification | HistoryTopic::Quest, _) => {
            if let Some(by) = &entry.by {
                prefix(&mut text, format!("{by}  "), palette::VIOLET_80);
            }
        }
        _ => {}
    }
    let color = match (entry.topic, entry.mark) {
        (_, Some(HistoryMark::Failed)) | (HistoryTopic::Error, _) => palette::CRIMSON_80,
        _ => ink,
    };
    let indent = match entry.topic {
        HistoryTopic::Talk => TALK_INDENT,
        _ => 0.0,
    };
    let icon: Vec<Box<dyn Scene>> = entry
        .icon
        .clone()
        .map(|icon| -> Box<dyn Scene> { Box::new(icon_image(assets.load(icon))) })
        .into_iter()
        .collect();
    let line = ui::rich_text(
        RichText {
            size: typography::BODY.font_size,
            color,
            ..text
        },
        false,
    );
    Box::new(bsn! {
        Node { column_gap: Val::Px({spacing::M}), align_items: AlignItems::FlexStart, width: Val::Percent(100.0) }
        Children [
            (
                Node { width: Val::Px(TIME_WIDTH), flex_shrink: 0.0 }
                Children [ {EntityScene(ui::styled_text(time, ink.with_alpha(0.5), typography::CAPTION))} ]
            ),
            (
                Node { column_gap: Val::Px({spacing::M}), align_items: AlignItems::Center, flex_grow: 1.0, min_width: Val::Px(0.0), margin: {UiRect::left(Val::Px(indent))} }
                Children [ {icon}, {EntityScene(line)} ]
            ),
        ]
    })
}

fn header(text: String, ink: Color) -> impl Scene {
    bsn! {
        Node { margin: {UiRect::top(Val::Px(spacing::M))}, width: Val::Percent(100.0) }
        Children [ {EntityScene(ui::styled_text(text, ink.with_alpha(0.75), typography::LABEL))} ]
    }
}

fn divider(text: String, ink: Color) -> impl Scene {
    let line = || {
        ui::Style::new()
            .background(ink.with_alpha(0.25))
            .node(|node| {
                node.flex_grow = 1.0;
                node.height = Val::Px(1.0);
            })
    };
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, width: Val::Percent(100.0), margin: {UiRect::vertical(Val::Px(spacing::M))} }
        Children [
            ( template_value(line()) ),
            {EntityScene(ui::styled_text(text, ink.with_alpha(0.7), typography::CAPTION))},
            ( template_value(line()) ),
        ]
    }
}

fn icon_image(image: Handle<Image>) -> impl Scene {
    bsn! {
        Node { width: Val::Px(ICON), height: Val::Px(ICON), flex_shrink: 0.0 }
        component(ImageNode::new(image))
        Pickable::IGNORE
    }
}

fn words(text: &LineText) -> String {
    text.0
        .iter()
        .map(|span| match span {
            crate::systems::text::SpanText::Text { text, .. } => text.as_str(),
            crate::systems::text::SpanText::Input(_) => "",
        })
        .collect()
}

fn descendant_with<C: Component>(world: &World, root: Entity) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world.get::<C>(entity).is_some() {
            return Some(entity);
        }
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    None
}
