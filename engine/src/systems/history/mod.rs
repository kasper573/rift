pub mod widget;

use std::collections::VecDeque;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::common_conditions::resource_removed;
use bevy_replication::ServerLink;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ImageRef};
use crate::core::text::LineText;
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::messages::{Direction, messages};
use crate::systems::player::{ClientId, PlayedBy, Players};
use crate::systems::visibility;

/// How a character's history is kept.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySettings {
    /// How soon after each other like entries of a character's history count as one, a run of loot
    /// or kills read as a tally.
    pub merges_within: Seconds,
    /// How many records a character's history keeps. The oldest go first, and the history says it
    /// is trimmed.
    pub kept: u32,
}

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_server_message::<HistoryNews>();
}

/// Keeps a client's [`HistoryBook`] as the server tells it of its character's history, until
/// the session ends.
pub struct HistoryBookPlugin;

impl Plugin for HistoryBookPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HistoryBook>().add_systems(
            Update,
            (
                read_news.run_if(resource_exists::<Content>),
                forget.run_if(resource_removed::<ServerLink>),
            ),
        );
    }
}

/// The history of the character a client plays, as far as the game keeps it.
#[derive(Resource, Default)]
pub struct HistoryBook {
    subject: Option<ClientId>,
    records: Vec<HistoryRecord>,
}

impl HistoryBook {
    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }

    fn upsert(&mut self, record: HistoryRecord, kept: usize) {
        match self
            .records
            .binary_search_by_key(&record.nth, |kept| kept.nth)
        {
            Ok(index) => self.records[index] = record,
            Err(index) => self.records.insert(index, record),
        }
        let overflow = self.records.len().saturating_sub(kept);
        self.records.drain(..overflow);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryRecord {
    pub nth: u64,
    pub at: UnixMillis,
    pub entry: HistoryEntry,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub topic: HistoryTopic,
    pub by: Option<String>,
    pub icon: Option<ImageRef>,
    pub text: LineText,
    pub tally: Option<RecordTally>,
    pub mark: Option<HistoryMark>,
    pub repeats: u32,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryTopic {
    Talk,
    Quest,
    Item,
    Notification,
    Error,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMark {
    You,
    Began,
    Arrived,
    Failed,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordTally {
    Change(i32),
    Progress { have: u32, need: u32 },
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct History {
    counted: u64,
    records: VecDeque<HistoryRecord>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum HistoryNews {
    Backfill {
        subject: ClientId,
        records: Vec<HistoryRecord>,
    },
    Recorded {
        subject: ClientId,
        record: HistoryRecord,
    },
}

impl HistoryEntry {
    pub fn of(topic: HistoryTopic, text: LineText) -> HistoryEntry {
        HistoryEntry {
            topic,
            by: None,
            icon: None,
            text,
            tally: None,
            mark: None,
            repeats: 1,
        }
    }

    pub fn by(self, by: Option<String>) -> HistoryEntry {
        HistoryEntry { by, ..self }
    }

    pub fn icon(self, icon: Option<ImageRef>) -> HistoryEntry {
        HistoryEntry { icon, ..self }
    }

    pub fn tally(self, tally: Option<RecordTally>) -> HistoryEntry {
        HistoryEntry { tally, ..self }
    }

    pub fn mark(self, mark: Option<HistoryMark>) -> HistoryEntry {
        HistoryEntry { mark, ..self }
    }

    pub fn merged(&self, newer: &HistoryEntry) -> Option<HistoryEntry> {
        let same = self.topic == newer.topic
            && self.by == newer.by
            && self.icon == newer.icon
            && self.text == newer.text
            && self.mark == newer.mark;
        if !same {
            return None;
        }
        match (self.tally, newer.tally) {
            (Some(old), Some(new)) => Some(HistoryEntry {
                tally: Some(old.merged(new)?),
                ..newer.clone()
            }),
            (None, None) => Some(HistoryEntry {
                repeats: self.repeats + newer.repeats,
                ..newer.clone()
            }),
            _ => None,
        }
    }
}

impl RecordTally {
    /// `text` with the tally beside it, as plain words.
    pub fn line(self, text: &str, messages: &HistoryMessages) -> String {
        match self {
            RecordTally::Change(change) => {
                messages.change_line(change.unsigned_abs(), Direction::of(change), text)
            }
            RecordTally::Progress { have, need } => messages.progress_line(have, need, text),
        }
    }

    pub fn merged(self, newer: RecordTally) -> Option<RecordTally> {
        match (self, newer) {
            (RecordTally::Change(old), RecordTally::Change(new))
                if old.signum() == new.signum() =>
            {
                Some(RecordTally::Change(old.saturating_add(new)))
            }
            (RecordTally::Progress { .. }, RecordTally::Progress { .. }) => Some(newer),
            _ => None,
        }
    }
}

impl History {
    pub fn records(&self) -> impl Iterator<Item = &HistoryRecord> {
        self.records.iter()
    }

    /// The history `records` make, like entries within `settings.merges_within` of each other
    /// counted as one.
    pub fn replayed(
        records: impl IntoIterator<Item = (UnixMillis, HistoryEntry)>,
        settings: HistorySettings,
    ) -> History {
        let mut history = History::default();
        for (at, entry) in records {
            history.write(entry, at, settings);
        }
        history
    }

    fn write(
        &mut self,
        entry: HistoryEntry,
        now: UnixMillis,
        settings: HistorySettings,
    ) -> HistoryRecord {
        let recent = self
            .records
            .iter_mut()
            .rev()
            .take_while(|record| now.since(record.at) < settings.merges_within);
        for record in recent {
            if let Some(merged) = record.entry.merged(&entry) {
                record.entry = merged;
                return record.clone();
            }
        }
        self.counted += 1;
        let record = HistoryRecord {
            nth: self.counted,
            at: now,
            entry,
        };
        self.records.push_back(record.clone());
        while self.records.len() > settings.kept as usize {
            self.records.pop_front();
        }
        record
    }
}

pub fn record(world: &mut World, player: Entity, entry: HistoryEntry) {
    let now = world.resource::<WallClock>().now;
    let settings = world.resource::<GameSettings>().history;
    let Some(subject) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
        return;
    };
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return;
    };
    let record = entity
        .entry::<History>()
        .or_default()
        .into_mut()
        .write(entry, now, settings);
    visibility::send_private(world, player, HistoryNews::Recorded { subject, record });
}

pub fn backfill(
    world: &mut World,
    sights: &mut QueryState<(Entity, &ClientId), Added<ClientId>>,
    arrivals: &mut QueryState<Entity, Added<History>>,
) {
    let mut sends: Vec<(Entity, Entity)> = sights
        .iter(world)
        .filter_map(|(conn, sight)| {
            let player = world.resource::<Players>().0.get(sight).copied()?;
            Some((conn, player))
        })
        .collect();
    let arrived: Vec<Entity> = arrivals.iter(world).collect();
    for player in arrived {
        sends.extend(
            visibility::private_viewers(world, player)
                .into_iter()
                .map(|conn| (conn, player)),
        );
    }
    for (conn, player) in sends {
        let Some(subject) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
            continue;
        };
        let records = world
            .get::<History>(player)
            .map(|history| history.records().cloned().collect())
            .unwrap_or_default();
        visibility::send_to(world, conn, HistoryNews::Backfill { subject, records });
    }
}

messages! {
    /// What the game says of what has happened to the player.
    pub struct HistoryMessages {
        /// The title of the history's tab of everything that happened.
        tab_all,
        /// The title of the history's tab of what was said.
        tab_talk,
        /// The title of the history's tab of quest news.
        tab_quests,
        /// The title of the history's tab of items gained and lost.
        tab_items,
        /// The title of the history's tab of notifications.
        tab_notifications,
        /// The title of the history's tab of refusals and errors.
        tab_errors,
        /// What the tab of everything says while nothing has happened.
        empty_all,
        /// What the tab of talk says while nothing was said.
        empty_talk,
        /// What the tab of quest news says while there is none.
        empty_quests,
        /// What the tab of items says while none was gained or lost.
        empty_items,
        /// What the tab of notifications says while there are none.
        empty_notifications,
        /// What the tab of errors says while there are none.
        empty_errors,
        /// What the top of a full history says of the records dropped, the number kept being `{count}`.
        trimmed(count: Number),
        /// What a conversation with no named speaker is headed.
        conversation,
        /// What the player's own line in a conversation is led by.
        you,
        /// A change of `{change}`, up or down as `{direction}` says, in front of the line it is of,
        /// as the tally of a record.
        change_mark(change: Number, direction: Text),
        /// How far along something is, `{have}` out of `{need}`, after the line it is of, as the tally of a record.
        progress_mark(have: Number, need: Number),
        /// A line, `{text}`, with a change of `{change}`, up or down as `{direction}` says, in front
        /// of it, as plain words.
        change_line(change: Number, direction: Text, text: Text),
        /// A line, `{text}`, with how far along something is, `{have}` out of `{need}`, after it, as plain words.
        progress_line(have: Number, need: Number, text: Text),
        /// What follows a line in the history that happened `{count}` times in a row.
        repeated(count: Number),
        /// What another's line in a conversation or a notification is led by: the speaker `{by}`.
        speaker(by: Text),
    }
}

fn read_news(
    mut news: MessageReader<HistoryNews>,
    mut book: ResMut<HistoryBook>,
    content: Res<Content>,
) {
    let kept = content.built::<GameSettings>().history.kept as usize;
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
                book.upsert(record, kept);
            }
        }
    }
}

fn forget(mut book: ResMut<HistoryBook>) {
    *book = HistoryBook::default();
}
