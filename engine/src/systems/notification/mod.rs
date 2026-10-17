pub mod bubble;
pub mod budget;
pub mod error;
pub mod feed;
pub mod intro;
mod rows;

pub use intro::IntroAppearance;
pub use rows::{NotificationRow, NotificationRows};

use std::borrow::Cow;

use bevy::ecs::entity::MapEntities;
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use bevy::window::WindowOccluded;
use bevy_egui::EguiPrimaryContextPass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::audio::playback::{PlaySfx, SfxId, SfxPlace};
use crate::core::content::{Content, ContentId, ContentRow, Fixture, Loading, RowCheck};
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::math::Rgba;
use crate::core::script::{Compiler, EffectDef, EffectStep, Name, ScriptFault, ValueType};
use crate::core::tiling::Tiles;
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic, RecordTally};
use crate::systems::interface::InterfaceSound;
use crate::systems::movement::position;
use crate::systems::npc::{Npc, NpcDef, NpcId};
use crate::systems::screen::ClientScreen;
use crate::systems::script::effect::{self as effects, Aim, Arrow, Audience, Link};
use crate::systems::script::{EffectCall, Reach, RiftScripting, condition};
use crate::systems::text::{self, LineText, Motion, Span};
use crate::systems::visibility;

pub type NotificationId = ContentId<NotificationDef>;

const READING_BASE: Seconds = Seconds(2.0);
const READING_PER_WORD: Seconds = Seconds(0.3);
const LONGEST_ROW_WORDS: usize = 30;
const COLUMN_TOP: f32 = 16.0;
const GROUP_GAP: f32 = 8.0;
const CENTRE_BELOW: f32 = 0.15;
const CENTRE_GAP: f32 = 16.0;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_mapped_server_message::<NotificationSent>(Channel::Ordered)
        .add_server_message::<SoundPlayed>(Channel::Ordered);
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Notification {
    pub kind: NotificationKind,
    pub text: LineText,
    pub lasts: Option<Seconds>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NotificationKind {
    Speech {
        speaker: NpcId,
    },
    Narration {
        label: Cow<'static, str>,
        sfx: Option<SfxId>,
    },
    Alert {
        label: Cow<'static, str>,
        sfx: Option<SfxId>,
    },
    Error {
        sfx: Option<SfxId>,
    },
    Milestone {
        label: Cow<'static, str>,
        topic: HistoryTopic,
        sfx: Option<SfxId>,
    },
    Intro {
        title: Cow<'static, str>,
        appearance: IntroAppearance,
        sfx: Option<SfxId>,
    },
    Feed {
        topic: HistoryTopic,
        icon: Option<String>,
        tally: Option<RecordTally>,
        failure: bool,
        sfx: Option<SfxId>,
    },
}

impl NotificationKind {
    pub fn error(content: &Content) -> NotificationKind {
        NotificationKind::Error {
            sfx: Some(InterfaceSound::UiRefuse.id(content)),
        }
    }

    fn sfx(&self) -> Option<SfxId> {
        match self {
            NotificationKind::Speech { .. } => None,
            NotificationKind::Narration { sfx, .. }
            | NotificationKind::Alert { sfx, .. }
            | NotificationKind::Error { sfx }
            | NotificationKind::Milestone { sfx, .. }
            | NotificationKind::Intro { sfx, .. }
            | NotificationKind::Feed { sfx, .. } => *sfx,
        }
    }
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct NotificationSent {
    pub notification: Notification,
    #[entities]
    pub speaker: Option<Entity>,
    pub at: UnixMillis,
}

pub fn notify(world: &mut World, player: Entity, notification: Notification) {
    notify_from(world, player, None, notification);
}

/// An error the client raises for itself, shown like one the server sent.
pub fn local_error(world: &mut World, text: impl Into<String>) {
    let kind = NotificationKind::error(world.resource::<Content>());
    show(
        world,
        NotificationSent {
            notification: Notification::new(kind, LineText::plain(text.into())),
            speaker: None,
            at: UnixMillis(0),
        },
    );
}

pub fn error(world: &mut World, player: Entity, text: impl Into<String>) {
    let kind = NotificationKind::error(world.resource::<Content>());
    notify(
        world,
        player,
        Notification::new(kind, LineText::plain(text.into())),
    );
}

pub fn notify_from(
    world: &mut World,
    player: Entity,
    speaker: Option<Entity>,
    notification: Notification,
) {
    record(world, player, &notification);
    let at = world.resource::<WallClock>().now;
    let speaker = speaker.filter(|&npc| {
        notification.spoken_by(world, npc) && visibility::present(world, npc, player)
    });
    let live = match notification.kind {
        NotificationKind::Feed { .. } | NotificationKind::Error { .. } => true,
        _ => budget::admit(world, player, &notification, speaker, at),
    };
    if live {
        visibility::send_private(
            world,
            player,
            NotificationSent {
                notification,
                speaker,
                at,
            },
        );
    }
}

pub fn record(world: &mut World, player: Entity, notification: &Notification) {
    let entry = notification.history_entry(world.resource::<Content>());
    history::record(world, player, entry);
}

pub fn captions(world: &World) -> NotificationRows {
    rows::shown::<rows::Captions>(world)
}

pub fn alerts(world: &World) -> NotificationRows {
    rows::shown::<rows::Alerts>(world)
}

pub fn milestones(world: &World) -> NotificationRows {
    rows::shown::<rows::Milestones>(world)
}

impl Notification {
    pub fn new(kind: NotificationKind, text: LineText) -> Notification {
        Notification {
            kind,
            text,
            lasts: None,
        }
    }

    pub fn stays(&self) -> Seconds {
        self.lasts
            .unwrap_or_else(|| reading(self.text.word_count()))
    }

    fn spoken_by(&self, world: &World, npc: Entity) -> bool {
        match self.kind {
            NotificationKind::Speech { speaker } => {
                world.get::<Npc>(npc).is_some_and(|npc| npc.def == speaker)
            }
            _ => false,
        }
    }

    fn history_entry(&self, content: &Content) -> HistoryEntry {
        let entry = |topic: HistoryTopic, by: Option<&str>| {
            HistoryEntry::of(topic, self.text.clone()).by(by.map(str::to_owned))
        };
        match &self.kind {
            NotificationKind::Speech { speaker } => entry(
                HistoryTopic::Notification,
                Some(speaker.get(content).name.as_str()),
            ),
            NotificationKind::Narration { label, .. } | NotificationKind::Alert { label, .. } => {
                entry(HistoryTopic::Notification, Some(label))
            }
            NotificationKind::Error { .. } => entry(HistoryTopic::Error, None),
            NotificationKind::Milestone { label, topic, .. } => entry(*topic, Some(label)),
            NotificationKind::Intro { title, .. } => entry(HistoryTopic::Notification, Some(title)),
            NotificationKind::Feed {
                topic,
                icon,
                tally,
                failure,
                ..
            } => entry(*topic, None)
                .icon(icon.clone())
                .tally(*tally)
                .mark(failure.then_some(HistoryMark::Failed)),
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct NotificationDef {
    pub kind: NotificationDefKind,
    pub text: Vec<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lasts: Option<Seconds>,
}

/// The kinds content may send; the others are raised by the game itself.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NotificationDefKind {
    Speech {
        speaker: NpcId,
    },
    Narration {
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sfx: Option<SfxId>,
    },
    Intro {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tint: Option<Rgba>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        motion: Option<Motion>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emblem: Option<AssetRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sfx: Option<SfxId>,
    },
}

impl ContentRow for NotificationDef {
    const TABLE: &'static str = "notification";
}

impl NotificationDef {
    pub fn notification(&self, text: LineText) -> Notification {
        Notification {
            kind: (&self.kind).into(),
            text,
            lasts: self.lasts,
        }
    }

    fn speaker(&self) -> Option<NpcId> {
        match self.kind {
            NotificationDefKind::Speech { speaker } => Some(speaker),
            _ => None,
        }
    }
}

impl From<&NotificationDefKind> for NotificationKind {
    fn from(kind: &NotificationDefKind) -> NotificationKind {
        match kind {
            NotificationDefKind::Speech { speaker } => {
                NotificationKind::Speech { speaker: *speaker }
            }
            NotificationDefKind::Narration { label, sfx } => NotificationKind::Narration {
                label: Cow::Owned(label.clone()),
                sfx: *sfx,
            },
            NotificationDefKind::Intro {
                title,
                tint,
                motion,
                emblem,
                sfx,
            } => NotificationKind::Intro {
                title: Cow::Owned(title.clone()),
                appearance: IntroAppearance {
                    tint: *tint,
                    motion: *motion,
                    emblem: emblem.clone(),
                },
                sfx: *sfx,
            },
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Notify {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub notification: NotificationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Notify {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![("next", self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let row = self.notification.get(cx.content);
        let values = text::values(&row.text)
            .into_iter()
            .map(|value| cx.expr(value, ValueType::Text))
            .collect::<Result<Vec<_>, _>>()?;
        let def = row.clone();
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                let spoken: Vec<_> = values.iter().map(|value| value(world, cx)).collect();
                let notification = def.notification(LineText::spoken(&def.text, &spoken));
                notify_from(world, player, cx.owner, notification);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Notifies(self.notification)]
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SoundPlayed {
    pub sfx: SfxId,
    pub at: Option<Pos<Tiles>>,
}

pub fn play_sound(
    world: &mut World,
    owner: Option<Entity>,
    subject: Option<Entity>,
    sfx: SfxId,
    audience: Audience,
) {
    let source = owner.or(subject);
    let at = source.and_then(|source| position(world, source));
    let played = SoundPlayed { sfx, at };
    match audience {
        Audience::Everyone => {
            let Some(source) = source else {
                return;
            };
            for client in visibility::seen_by(world, source) {
                visibility::send_to(world, client, played);
            }
        }
        Audience::Subject => {
            if let Some(subject) = subject {
                visibility::send_private(world, subject, played);
            }
        }
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<NotificationDef>(&content) {
        let mut check = RowCheck::default();
        for (index, span) in def.text.iter().enumerate() {
            if let Span::Value { value, .. } = span
                && let Err(fault) = condition::check_value(&content, value)
            {
                check.error(
                    format!("/text/{index}/value"),
                    fault.rule,
                    "the value",
                    fault.expected,
                );
            }
        }
        let words: usize = def
            .text
            .iter()
            .map(|span| match span {
                Span::Text { text, .. } => text.split_whitespace().count(),
                Span::Input(_) | Span::Value { .. } => 1,
            })
            .sum();
        if words > LONGEST_ROW_WORDS {
            check.error(
                "/text",
                "too many words",
                format!("{words} words"),
                format!("at most {LONGEST_ROW_WORDS} words, a value counting as one"),
            );
        }
        if let Some(speaker) = def.speaker()
            && content.table::<NpcDef>().get(speaker).is_some()
            && speaker.get(&content).voice().is_none()
        {
            check.error(
                "/kind/speaker",
                "needs a voice",
                speaker.key(&content),
                "an npc with a voice module",
            );
        }
        loading.row_errors(id, check);
    }
}

pub struct NotificationPlugin;

impl Plugin for NotificationPlugin {
    fn build(&self, app: &mut App) {
        rows::plugin::<rows::Captions>(app);
        rows::plugin::<rows::Alerts>(app);
        rows::plugin::<rows::Milestones>(app);
        error::plugin(app);
        app.add_plugins((bubble::BubblePlugin, intro::IntroPlugin, feed::FeedPlugin))
            .init_resource::<Occlusion>()
            .add_systems(Update, track_occlusion)
            .add_systems(
                Update,
                (deliver, deliver_sounds).run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                draw_column
                    .in_set(Band::Notices)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

fn draw_column(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    let alerts = rows::draw_alerts(world, &gui, screen.top() + COLUMN_TOP);
    let captions = rows::draw_captions(world, &gui, alerts + GROUP_GAP);
    let error = error::draw(world, &gui, captions + GROUP_GAP);
    let centre = error + screen.height() * CENTRE_BELOW;
    let intro = intro::draw(world, &gui, centre);
    rows::draw_milestones(world, &gui, intro + CENTRE_GAP);
}

fn deliver(world: &mut World, mut cursor: Local<MessageCursor<NotificationSent>>) {
    let fresh: Vec<NotificationSent> = cursor
        .read(world.resource::<Messages<NotificationSent>>())
        .cloned()
        .collect();
    for notification_sent in fresh {
        show(world, notification_sent);
    }
}

#[derive(Resource, Default)]
pub(super) struct Occlusion {
    occluded: bool,
}

fn track_occlusion(mut events: MessageReader<WindowOccluded>, mut occlusion: ResMut<Occlusion>) {
    for event in events.read() {
        occlusion.occluded = event.occluded;
    }
}

/// Notifications age only while the game window is on screen, so one that was raised before the
/// window was hidden still has its full time when it comes back.
pub(super) fn window_visible(occlusion: Res<Occlusion>) -> bool {
    !occlusion.occluded
}

fn deliver_sounds(mut played: MessageReader<SoundPlayed>, mut sfx: MessageWriter<PlaySfx>) {
    for sound in played.read() {
        sfx.write(PlaySfx {
            id: sound.sfx,
            place: sound.at.map_or(SfxPlace::Interface, SfxPlace::World),
        });
    }
}

fn show(world: &mut World, notification_sent: NotificationSent) {
    let intro = matches!(
        notification_sent.notification.kind,
        NotificationKind::Intro { .. }
    );
    if let Some(id) = notification_sent.notification.kind.sfx()
        && !intro
    {
        world.write_message(PlaySfx {
            id,
            place: SfxPlace::Interface,
        });
    }
    match &notification_sent.notification.kind {
        NotificationKind::Speech { .. } => bubble::say(world, notification_sent),
        NotificationKind::Narration { label, .. } => {
            rows::join::<rows::Captions>(world, label.clone(), notification_sent)
        }
        NotificationKind::Alert { label, .. } => {
            rows::join::<rows::Alerts>(world, label.clone(), notification_sent)
        }
        NotificationKind::Error { .. } => error::toast(world, notification_sent),
        NotificationKind::Milestone { label, .. } => {
            rows::join::<rows::Milestones>(world, label.clone(), notification_sent)
        }
        NotificationKind::Intro { .. } => intro::show(world, notification_sent),
        NotificationKind::Feed { .. } => feed::show(world, notification_sent),
    }
}

fn reading(words: usize) -> Seconds {
    Seconds(READING_BASE.0 + READING_PER_WORD.0 * words as f32)
}
