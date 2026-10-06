pub mod alert;
pub mod bubble;
pub mod budget;
pub mod caption;
pub mod error;
pub mod feed;
pub mod intro;
pub mod milestone;
mod rows;

use std::borrow::Cow;

use bevy::ecs::entity::MapEntities;
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use bevy::scene::EntityScene;
use serde::{Deserialize, Serialize};

use crate::core::platform::ClientPlatform;
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::data;
use crate::data::npc::Id as NpcId;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic, RecordTally};
use crate::systems::npc::Npc;
use crate::systems::player::Players;
use crate::systems::rule::{Outcome, RuleContext};
use crate::systems::scene::Scene as GameScene;
use crate::systems::text::{self, LineText, Span, SpanText};
use crate::systems::visibility;

pub use crate::data::notification::Id as NotificationId;
pub use intro::IntroAppearance;
pub use rows::{NotificationRow, NotificationRows};

const READING_BASE: Seconds = Seconds(2.0);
const READING_PER_WORD: Seconds = Seconds(0.3);
const LATE_AFTER: Seconds = Seconds(2.0);
const SERVER_LABEL: &str = "Server";
const SERVER_LASTS: Seconds = Seconds(60.0);
const LONGEST_ROW_WORDS: usize = 30;
const LONGEST_NOTIFY_CHARS: usize = 160;

pub fn register(app: &mut bevy_app::App) {
    use bevy_replicon::prelude::*;
    app.add_mapped_server_message::<NotificationSent>(Channel::Ordered)
        .init_resource::<Broadcasts>();
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
    pub fn error() -> NotificationKind {
        NotificationKind::Error {
            sfx: Some(SfxId::UiRefuse),
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

pub fn notify_from(
    world: &mut World,
    player: Entity,
    speaker: Option<Entity>,
    notification: Notification,
) {
    record(world, player, &notification);
    let at = world.resource::<WallClock>().now;
    let speaker = speaker.filter(|&npc| notification.spoken_by(world, npc));
    let live = match notification.kind {
        NotificationKind::Feed { .. } | NotificationKind::Error { .. } => true,
        _ => budget::admit(world, player, &notification, speaker, at) == budget::Admit::Live,
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
    history::record(world, player, notification.history_entry());
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
        self.lasts.unwrap_or_else(|| {
            let words = self.text.word_count() as f32;
            Seconds(READING_BASE.0 + READING_PER_WORD.0 * words)
        })
    }

    fn spoken_by(&self, world: &World, npc: Entity) -> bool {
        match self.kind {
            NotificationKind::Speech { speaker } => {
                world.get::<Npc>(npc).is_some_and(|npc| npc.def == speaker)
            }
            _ => false,
        }
    }

    fn history_entry(&self) -> HistoryEntry {
        let entry = |topic: HistoryTopic, by: Option<&str>| {
            HistoryEntry::of(topic, self.text.clone()).by(by.map(str::to_owned))
        };
        match &self.kind {
            NotificationKind::Speech { speaker } => {
                entry(HistoryTopic::Notification, Some(speaker.get().display_name))
            }
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

pub struct NotificationDef {
    pub kind: NotificationKind,
    pub text: &'static [Span],
    pub lasts: Option<Seconds>,
}

impl NotificationDef {
    pub fn for_player(&self, world: &World, player: Entity) -> Notification {
        Notification {
            kind: self.kind.clone(),
            text: LineText::spoken(self.text, world, player),
            lasts: self.lasts,
        }
    }

    fn speaker(&self) -> Option<NpcId> {
        match self.kind {
            NotificationKind::Speech { speaker } => Some(speaker),
            _ => None,
        }
    }
}

pub struct Notify(pub NotificationId);

impl Outcome for Notify {
    fn apply(&self, ctx: &mut RuleContext) {
        let notification = self.0.get().for_player(ctx.world, ctx.player);
        notify_from(ctx.world, ctx.player, ctx.encounter.with, notification);
    }
}

pub fn speakers() -> impl Iterator<Item = NpcId> {
    data::notification::TABLE
        .iter()
        .filter_map(NotificationDef::speaker)
}

pub fn check() {
    for (id, def) in <NotificationId as strum::VariantArray>::VARIANTS
        .iter()
        .zip(data::notification::TABLE)
    {
        for fill in text::fills(def.text) {
            fill.check();
        }
        let words: usize = def
            .text
            .iter()
            .map(|span| match span {
                Span::Text { text, .. } => text.split_whitespace().count(),
                Span::Input(_) | Span::Fill { .. } => 1,
            })
            .sum();
        if words > LONGEST_ROW_WORDS {
            panic!("notification {id:?}: {words} words, more than {LONGEST_ROW_WORDS}");
        }
    }
}

#[derive(Resource, Default)]
pub struct Broadcasts(Vec<Notification>);

pub fn broadcast(world: &mut World, notification: Notification) {
    world.resource_mut::<Broadcasts>().0.push(notification);
}

pub fn relay(worlds: &mut [&mut World]) {
    let raised: Vec<Notification> = worlds
        .iter_mut()
        .flat_map(|world| std::mem::take(&mut world.resource_mut::<Broadcasts>().0))
        .collect();
    for notification in &raised {
        notify_everywhere(worlds, notification);
    }
}

pub fn notify_everywhere(worlds: &mut [&mut World], notification: &Notification) {
    for world in worlds.iter_mut() {
        let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
        for player in players {
            notify(world, player, notification.clone());
        }
    }
}

pub fn server_alert(message: impl Into<String>) -> Notification {
    Notification {
        kind: NotificationKind::Alert {
            label: Cow::Borrowed(SERVER_LABEL),
            sfx: Some(SfxId::UiChime),
        },
        text: LineText::plain(message),
        lasts: Some(SERVER_LASTS),
    }
}

/// Raise an alert for every player in every area.
#[bevy_terminal::command(name = "notify", access = crate::systems::account::role::is_admin)]
fn notify_command(
    world: &mut World,
    _ctx: &bevy_terminal::CommandCtx,
    message: bevy_terminal::RestOfLine,
) -> Result<String, String> {
    let length = message.0.chars().count();
    if length > LONGEST_NOTIFY_CHARS {
        return Err(format!(
            "{length} characters is too long, at most {LONGEST_NOTIFY_CHARS} fit"
        ));
    }
    broadcast(world, server_alert(message.0));
    Ok("notified every area".to_owned())
}

pub struct NotificationPlugin;

impl Plugin for NotificationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            bubble::BubblePlugin,
            caption::CaptionPlugin,
            alert::AlertPlugin,
            milestone::MilestonePlugin,
            intro::IntroPlugin,
            feed::FeedPlugin,
        ))
        .add_systems(OnEnter(GameScene::Area), spawn_places)
        .add_systems(
            OnExit(GameScene::Area),
            crate::systems::scene::despawn_all::<PlacesRoot>,
        )
        .add_systems(Update, deliver.run_if(in_state(GameScene::Area)));
    }
}

#[derive(Component, Default, Clone)]
struct PlacesRoot;

const CENTRE_BELOW: f32 = 15.0;

fn spawn_places(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        PlacesRoot
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: {UiRect::top(Val::Px(ui::tokens::spacing::XL))},
        }
        Pickable::IGNORE
        GlobalZIndex({ui::tokens::layer::NOTIFICATIONS})
        Children [
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px({ui::tokens::spacing::L}),
                    width: Val::Percent(100.0),
                }
                Pickable::IGNORE
                Children [
                    {EntityScene(ui::alert_bar())},
                    {EntityScene(ui::captions())},
                    (
                        {ui::toaster(ui::SonnerPosition::TopCenter)}
                        error::ErrorToaster
                        Node { position_type: PositionType::Relative, top: Val::Auto, left: Val::Auto, margin: UiRect::ZERO }
                    ),
                ]
            ),
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px({ui::tokens::spacing::XL}),
                    width: Val::Percent(100.0),
                    margin: {UiRect::top(Val::Vh(CENTRE_BELOW))},
                }
                Pickable::IGNORE
                Children [
                    ( intro::IntroHost Node { display: Display::Grid, justify_items: JustifyItems::Center } Pickable::IGNORE ),
                    {EntityScene(ui::milestones())},
                ]
            ),
        ]
    });
}

fn deliver(world: &mut World, mut cursor: Local<MessageCursor<NotificationSent>>) {
    let paused = world.resource::<Time<Real>>().delta_secs() > LATE_AFTER.0;
    let now = world.resource::<ClientPlatform>().0.local_clock().now;
    let fresh: Vec<NotificationSent> = cursor
        .read(world.resource::<Messages<NotificationSent>>())
        .cloned()
        .collect();
    for notification_sent in fresh {
        if paused && !notification_sent.outlives_pause(now) {
            continue;
        }
        show(world, notification_sent);
    }
}

fn show(world: &mut World, notification_sent: NotificationSent) {
    if let Some(id) = notification_sent.notification.kind.sfx() {
        world.write_message(PlaySfx {
            id,
            place: SfxPlace::Interface,
        });
    }
    match notification_sent.notification.kind {
        NotificationKind::Speech { .. } => bubble::say(world, notification_sent),
        NotificationKind::Narration { .. } => caption::join(world, notification_sent),
        NotificationKind::Alert { .. } => alert::raise(world, notification_sent),
        NotificationKind::Error { .. } => error::toast(world, notification_sent),
        NotificationKind::Milestone { .. } => milestone::reach(world, notification_sent),
        NotificationKind::Intro { .. } => intro::show(world, notification_sent),
        NotificationKind::Feed { .. } => feed::show(world, notification_sent),
    }
}

impl NotificationSent {
    fn outlives_pause(&self, now: UnixMillis) -> bool {
        match (&self.notification.kind, self.notification.lasts) {
            (NotificationKind::Feed { .. }, _) => true,
            (_, Some(lasts)) => now < self.at.after(lasts),
            (_, None) => false,
        }
    }
}

pub(crate) fn plain_words(text: &LineText) -> String {
    text.0
        .iter()
        .map(|span| match span {
            SpanText::Text { text, .. } => text.as_str(),
            SpanText::Input(_) => "",
        })
        .collect()
}
