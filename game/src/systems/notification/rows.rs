use std::borrow::Cow;
use std::time::Duration;

use bevy::prelude::*;
use ui::{LabelledLine, RichPiece, RichText};

use super::NotificationSent;
use crate::core::time::Seconds;
use crate::systems::input::map::{self, InputAction};

const COUNT_LINGERS: Seconds = Seconds(5.0);

pub struct NotificationRows {
    pub rows: Vec<NotificationRow>,
    pub more: u32,
}

pub struct NotificationRow {
    pub label: String,
    pub text: String,
    pub replaced: u32,
}

pub(super) struct VoiceRows {
    limit: usize,
    replace_after: Duration,
    rows: Vec<VoiceRow>,
    turned_away: Option<TurnedAway>,
    clock: Duration,
    keys: u64,
}

struct VoiceRow {
    key: u64,
    voice: Cow<'static, str>,
    shown: NotificationSent,
    newest: Option<NotificationSent>,
    replaced: u32,
    changed: Duration,
    read_by: Duration,
}

struct TurnedAway {
    more: u32,
    until: Duration,
}

impl VoiceRows {
    pub(super) fn new(limit: usize, replace_after: Seconds) -> VoiceRows {
        VoiceRows {
            limit,
            replace_after: replace_after.into(),
            rows: Vec::new(),
            turned_away: None,
            clock: Duration::ZERO,
            keys: 0,
        }
    }

    pub(super) fn join(&mut self, voice: Cow<'static, str>, notification_sent: NotificationSent) {
        let now = self.clock;
        let stays: Duration = notification_sent.notification.stays().into();
        if let Some(row) = self.rows.iter_mut().find(|row| row.voice == voice) {
            if row.shown.notification == notification_sent.notification {
                row.read_by = row.read_by.max(now + stays);
                return;
            }
            row.replaced += 1;
            row.newest = Some(notification_sent);
            return;
        }
        if self.rows.len() >= self.limit {
            let turned_away = self.turned_away.get_or_insert(TurnedAway {
                more: 0,
                until: now,
            });
            turned_away.more += 1;
            turned_away.until = now + COUNT_LINGERS.into();
            return;
        }
        let after = self.rows.last().map_or(now, |last| last.read_by.max(now));
        self.keys += 1;
        self.rows.push(VoiceRow {
            key: self.keys,
            voice,
            shown: notification_sent,
            newest: None,
            replaced: 0,
            changed: now,
            read_by: after + stays,
        });
    }

    pub(super) fn tick(&mut self, delta: Duration, held: bool) {
        if held {
            return;
        }
        self.clock += delta;
        let now = self.clock;
        for row in &mut self.rows {
            if now >= row.changed + self.replace_after
                && let Some(newest) = row.newest.take()
            {
                let stays: Duration = newest.notification.stays().into();
                row.read_by = row.read_by.max(now + stays);
                row.shown = newest;
                row.changed = now;
            }
        }
        self.rows.retain(|row| row.read_by > now);
        self.turned_away = self
            .turned_away
            .take()
            .filter(|turned_away| turned_away.until > now);
    }

    pub(super) fn close(&mut self, key: u64) {
        self.rows.retain(|row| row.key != key);
    }

    pub(super) fn labelled_lines(&self) -> Vec<LabelledLine> {
        self.rows
            .iter()
            .map(|row| LabelledLine {
                key: row.key,
                label: row.voice.to_string(),
                replaced: row.replaced,
                text: row.shown.notification.text.rich(),
            })
            .collect()
    }

    pub(super) fn more(&self) -> Option<RichText> {
        self.turned_away
            .as_ref()
            .map(|turned_away| more_text(turned_away.more))
    }

    pub(super) fn shown(&self) -> NotificationRows {
        NotificationRows {
            rows: self
                .rows
                .iter()
                .map(|row| NotificationRow {
                    label: row.voice.to_string(),
                    text: super::plain_words(&row.shown.notification.text),
                    replaced: row.replaced,
                })
                .collect(),
            more: self
                .turned_away
                .as_ref()
                .map_or(0, |turned_away| turned_away.more),
        }
    }
}

pub(super) fn more_text(more: u32) -> RichText {
    RichText::new(vec![
        RichPiece::text(format!("+{more} more \u{b7} ")),
        map::input(InputAction::ToggleHistory),
        RichPiece::text(" history"),
    ])
}

pub(super) fn held<C: Component>(world: &mut World) -> bool {
    world
        .query_filtered::<&bevy::picking::hover::Hovered, (With<C>, Without<ui::Leaving>)>()
        .iter(world)
        .any(|hovered| hovered.get())
}
