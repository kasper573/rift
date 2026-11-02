pub mod bubble;
pub mod budget;
pub mod error;
pub mod feed;
pub mod intro;
pub mod pop;
mod rows;

pub use intro::IntroAppearance;
pub use pop::{pop, pop_as};
pub use rows::{NotificationRow, NotificationRows};

use std::borrow::Cow;

use bevy::ecs::entity::MapEntities;
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use bevy::window::WindowOccluded;
use bevy_egui::EguiPrimaryContextPass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::{PlaySfx, SfxId, SfxPlace};
use crate::core::content::ImageRef;
use crate::core::content::SchemaMark;
use crate::core::content::{Content, ContentFile, ContentId, Fixture, Loading, Nearby, RowCheck};
use crate::core::gui::{self, Band};
use crate::core::math::{Pos, Rgba};
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault, ValueType};
use crate::core::text::TextMotion;
use crate::core::time::WallClock;
use crate::core::units::{Meters, Seconds};
use crate::core::vfx::{VfxParams, VfxShaderId};
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic, RecordTally};
use crate::systems::interface::InterfaceSound;
use crate::systems::movement::position;
use crate::systems::npc::{Npc, NpcDef, NpcId};
use crate::systems::screen::ClientScreen;
use crate::systems::script::effect::{self as effects, Audience};
use crate::systems::script::{Condition, EffectCall, EffectReach, RiftScripting, condition};
use crate::systems::text::{self, LineText, Span};
use crate::systems::{GameSettings, visibility};

pub type NotificationId = ContentId<NotificationDef>;

const LONGEST_ROW_WORDS: usize = 30;
const COLUMN_TOP: f32 = 16.0;
const GROUP_GAP: f32 = 8.0;
const CENTRE_BELOW: f32 = 0.15;
const CENTRE_GAP: f32 = 16.0;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_mapped_server_message::<NotificationSent>()
        .add_server_message::<SoundPlayed>();
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
    Feed(FeedRow),
    Pop {
        shader: Option<VfxShaderId>,
        params: VfxParams,
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
            NotificationKind::Speech { .. } | NotificationKind::Pop { .. } => None,
            NotificationKind::Narration { sfx, .. }
            | NotificationKind::Error { sfx }
            | NotificationKind::Milestone { sfx, .. }
            | NotificationKind::Intro { sfx, .. }
            | NotificationKind::Feed(FeedRow { sfx, .. }) => *sfx,
        }
    }
}

/// What a row of the feed says beside its text: under which topic of history, with which icon,
/// tally and sound, and whether it tells of a failure.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FeedRow {
    pub topic: HistoryTopic,
    pub icon: Option<String>,
    pub tally: Option<RecordTally>,
    pub failure: bool,
    pub sfx: Option<SfxId>,
}

/// A row of the feed being worded, a notification once sent.
pub struct FeedLine {
    row: FeedRow,
    text: LineText,
}

impl FeedLine {
    pub fn icon(mut self, path: String) -> FeedLine {
        self.row.icon = Some(path);
        self
    }

    pub fn tally(mut self, counted: RecordTally) -> FeedLine {
        self.row.tally = Some(counted);
        self
    }

    pub fn failed(mut self) -> FeedLine {
        self.row.failure = true;
        self
    }

    pub fn sound(mut self, heard: SfxId) -> FeedLine {
        self.row.sfx = Some(heard);
        self
    }
}

impl From<FeedLine> for Notification {
    fn from(line: FeedLine) -> Notification {
        Notification::new(NotificationKind::Feed(line.row), line.text)
    }
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct NotificationSent {
    pub notification: Notification,
    /// The actor the notification comes from, when it has one in the world.
    #[entities]
    pub actor: Option<Entity>,
    /// How long it stays to be read.
    pub stays: Seconds,
}

impl NotificationSent {
    /// `notification` from `actor`, staying as long as it says or as the game's players take to
    /// read it.
    pub fn new(
        world: &World,
        notification: Notification,
        actor: Option<Entity>,
    ) -> NotificationSent {
        let reading = world.resource::<GameSettings>().reading;
        NotificationSent {
            stays: notification
                .lasts
                .unwrap_or_else(|| reading.of(notification.text.word_count())),
            notification,
            actor,
        }
    }
}

/// An error the client raises for itself, shown like one the server sent.
pub fn local_error(world: &mut World, text: impl Into<String>) {
    let kind = NotificationKind::error(world.resource::<Content>());
    let notification = Notification::new(kind, LineText::plain(text.into()));
    let sent = NotificationSent::new(world, notification, None);
    show(world, sent);
}

pub fn error(world: &mut World, player: Entity, text: impl Into<String>) {
    let kind = NotificationKind::error(world.resource::<Content>());
    let notification = Notification::new(kind, LineText::plain(text.into()));
    notify(world, player, None, notification);
}

/// Tells `player` `notification`, from `speaker` where it comes from one they see.
pub fn notify(
    world: &mut World,
    player: Entity,
    speaker: Option<Entity>,
    notification: impl Into<Notification>,
) {
    let notification = notification.into();
    record(world, player, &notification);
    let at = world.resource::<WallClock>().now;
    let speaker = speaker.filter(|&npc| {
        notification.spoken_by(world, npc) && visibility::present(world, npc, player)
    });
    let sent = NotificationSent::new(world, notification, speaker);
    let live = match sent.notification.kind {
        NotificationKind::Feed(_) | NotificationKind::Error { .. } => true,
        _ => budget::admit(world, player, &sent, at),
    };
    if live {
        visibility::send_private(world, player, sent);
    }
}

fn record(world: &mut World, player: Entity, notification: &Notification) {
    if let Some(entry) = notification.history_entry(world.resource::<Content>()) {
        history::record(world, player, entry);
    }
}

pub fn captions(world: &World) -> NotificationRows {
    rows::shown::<rows::Captions>(world)
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

    /// A row of the feed under a topic, bare until given an icon, a tally, a sound or a failure.
    pub fn feed(topic: HistoryTopic, text: impl Into<String>) -> FeedLine {
        FeedLine {
            row: FeedRow {
                topic,
                icon: None,
                tally: None,
                failure: false,
                sfx: None,
            },
            text: LineText::plain(text),
        }
    }

    fn spoken_by(&self, world: &World, npc: Entity) -> bool {
        match self.kind {
            NotificationKind::Speech { speaker } => {
                world.get::<Npc>(npc).is_some_and(|npc| npc.def == speaker)
            }
            _ => false,
        }
    }

    fn history_entry(&self, content: &Content) -> Option<HistoryEntry> {
        let entry = |topic: HistoryTopic, by: Option<&str>| {
            HistoryEntry::of(topic, self.text.clone()).by(by.map(str::to_owned))
        };
        Some(match &self.kind {
            NotificationKind::Speech { speaker } => entry(
                HistoryTopic::Notification,
                Some(speaker.get(content).name.as_str()),
            ),
            NotificationKind::Narration { label, .. } => {
                entry(HistoryTopic::Notification, Some(label))
            }
            NotificationKind::Error { .. } => entry(HistoryTopic::Error, None),
            NotificationKind::Milestone { label, topic, .. } => entry(*topic, Some(label)),
            NotificationKind::Intro { title, .. } => entry(HistoryTopic::Notification, Some(title)),
            NotificationKind::Feed(FeedRow {
                topic,
                icon,
                tally,
                failure,
                ..
            }) => entry(*topic, None)
                .icon(icon.clone())
                .tally(*tally)
                .mark(failure.then_some(HistoryMark::Failed)),
            NotificationKind::Pop { .. } => return None,
        })
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
        motion: Option<TextMotion>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emblem: Option<ImageRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sfx: Option<SfxId>,
    },
}

impl ContentFile for NotificationDef {
    const TABLE: &'static str = "notification";

    fn fresh(near: &Nearby) -> Result<NotificationDef, String> {
        Ok(NotificationDef {
            kind: NotificationDefKind::Narration {
                label: near.name(),
                sfx: None,
            },
            text: Vec::new(),
            lasts: None,
        })
    }
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

/// Shows a notification.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("notification"))]
pub struct Notify {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub notification: NotificationId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Notify {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
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
                notify(world, player, cx.owner, notification);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Notifies(self.notification)]
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SoundPlayed {
    pub sfx: SfxId,
    pub at: Option<Pos<Meters>>,
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
                speaker.path(&content),
                "an npc with a voice module",
            );
        }
        loading.row_errors(id, check);
    }
}

pub struct NotificationPlugin;

impl Plugin for NotificationPlugin {
    fn build(&self, app: &mut App) {
        rows::place::<rows::PlacedRows<rows::Captions>>(app);
        rows::place::<rows::PlacedRows<rows::Milestones>>(app);
        error::plugin(app);
        app.add_plugins((
            bubble::BubblePlugin,
            intro::IntroPlugin,
            feed::FeedPlugin,
            pop::PopPlugin,
        ))
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
    let captions = rows::draw_captions(world, &gui, screen.top() + COLUMN_TOP);
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
        NotificationKind::Error { .. } => error::toast(world, notification_sent),
        NotificationKind::Milestone { label, .. } => {
            rows::join::<rows::Milestones>(world, label.clone(), notification_sent)
        }
        NotificationKind::Intro { .. } => intro::show(world, notification_sent),
        NotificationKind::Feed(_) => feed::show(world, notification_sent),
        NotificationKind::Pop { .. } => pop::show(world, notification_sent),
    }
}

/// Forgets what a kind of notice shows once the area it was shown in is left; what it spawned
/// goes with the area by itself.
fn forget<R: Resource + Default>(world: &mut World) {
    world.insert_resource(R::default());
}
