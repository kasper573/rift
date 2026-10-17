pub mod widget;

use std::collections::VecDeque;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::systems::player::{ClientId, Owner, Players};
use crate::systems::text::LineText;
use crate::systems::visibility;

pub const KEPT: usize = 500;
const MERGE_WITHIN: Seconds = Seconds(10.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_server_message::<HistoryNews>(Channel::Ordered);
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryRecord {
    pub nth: u64,
    pub at: UnixMillis,
    pub entry: HistoryEntry,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub topic: HistoryTopic,
    pub by: Option<String>,
    pub icon: Option<String>,
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

#[derive(Component, Clone, Debug, Default)]
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

    pub fn icon(self, icon: Option<String>) -> HistoryEntry {
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
    pub fn line(self, text: &str) -> String {
        match self {
            RecordTally::Change(change) => format!("{} {text}", signed(change)),
            RecordTally::Progress { have, need } => format!("{text} {have}/{need}"),
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

    pub fn replayed(records: impl IntoIterator<Item = (UnixMillis, HistoryEntry)>) -> History {
        let mut history = History::default();
        for (at, entry) in records {
            history.write(entry, at);
        }
        history
    }

    fn write(&mut self, entry: HistoryEntry, now: UnixMillis) -> HistoryRecord {
        let recent = self
            .records
            .iter_mut()
            .rev()
            .take_while(|record| now.since(record.at).0 < MERGE_WITHIN.0);
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
        while self.records.len() > KEPT {
            self.records.pop_front();
        }
        record
    }
}

pub fn record(world: &mut World, player: Entity, entry: HistoryEntry) {
    let now = world.resource::<WallClock>().now;
    let Some(subject) = world.get::<Owner>(player).map(|owner| owner.client) else {
        return;
    };
    let Ok(mut entity) = world.get_entity_mut(player) else {
        return;
    };
    let record = entity
        .entry::<History>()
        .or_default()
        .into_mut()
        .write(entry, now);
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
        let Some(subject) = world.get::<Owner>(player).map(|owner| owner.client) else {
            continue;
        };
        let records = world
            .get::<History>(player)
            .map(|history| history.records().cloned().collect())
            .unwrap_or_default();
        visibility::send_to(world, conn, HistoryNews::Backfill { subject, records });
    }
}

pub fn signed(change: i32) -> String {
    if change > 0 {
        format!("+{change}")
    } else {
        format!("\u{2212}{}", change.unsigned_abs())
    }
}
