use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::RichText;
use ui::glyph::{Icon, phosphor};
use ui::ink;

use super::{
    ClientScreen, Notice, Retry, ScreenMessages, frame_as_before, frame_slowly, notice_screen,
};
use crate::core::content::sync::{
    ContentSource, SyncError, SyncPhase, SyncPolicy, SyncProgress, SyncStatus, SyncedContent, sync,
};
use crate::core::content::{Content, LoadPurpose, version};
use crate::core::gui::{self, Band};
use crate::core::math::Rng;
use crate::core::net::auth::Credentials;
use crate::core::os::{FetchFailure, Http, Profile, StartParams};
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::time;
use crate::core::units::Seconds;
use crate::core::update::{self, OtherBuildAnswer, ReleaseSource, UpdateFailure, UpdateStep};
use crate::systems::launch::ClientConfig;
use crate::systems::messages::{self, Messages};
use crate::systems::{install_content, load_content};
use futures_util::future::BoxFuture;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shown>()
            .add_systems(OnEnter(ClientScreen::Loading), begin)
            .add_systems(
                Update,
                (poll, read_words_early, show)
                    .chain()
                    .run_if(in_state(ClientScreen::Loading)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                loading_card
                    .in_set(Band::System)
                    .run_if(in_state(ClientScreen::Loading)),
            )
            .add_systems(OnExit(ClientScreen::Loading), forget);
    }
}

/// The sync under way; its task brings nothing when the content installed already is what the
/// server serves.
#[derive(Resource)]
struct Syncing {
    task: Task<Result<Option<Brought>, SyncError>>,
    status: SyncStatus,
    retry: Retry,
}

/// New content, synced and loaded.
struct Brought {
    synced: SyncedContent,
    content: Content,
}

/// The server's build being installed in place of this client's.
#[derive(Resource)]
struct Updating {
    task: Task<Result<PathBuf, UpdateFailure>>,
    step: Arc<Mutex<UpdateStep>>,
    to: String,
    retry: Retry,
}

/// The server's build is installed, and the client is starting again on it.
#[derive(Resource)]
struct Restarting {
    to: String,
}

/// What stopped the load. A failure whose retry is due tries again; one whose retry waits for
/// nothing can only be closed.
#[derive(Resource, Clone, PartialEq)]
struct LoadingFailure {
    cause: Cause,
    retry: Retry,
}

#[derive(Clone, PartialEq)]
enum Cause {
    DownloadFailed { reason: String },
    ServerRunsNewer { build: String },
    ServerRunsOther { build: String },
    UpdateDidNotRestart { build: String, reason: String },
    UpdateStopped { build: String, reason: String },
    UpdateUnsigned { build: String },
    UpdateGone { build: String },
    UpdateUnwritable { folder: String },
    UpdateMalformed { build: String, reason: String },
}

impl Cause {
    fn lines(&self, words: &ScreenMessages) -> Vec<String> {
        let line = match self {
            Cause::DownloadFailed { reason } => {
                return vec![words.download_failed().to_owned(), reason.clone()];
            }
            Cause::ServerRunsNewer { build } => words.server_runs_newer(build),
            Cause::ServerRunsOther { build } => words.server_runs_other(build),
            Cause::UpdateDidNotRestart { build, reason } => {
                words.update_did_not_restart(build, reason)
            }
            Cause::UpdateStopped { build, reason } => words.update_stopped(build, reason),
            Cause::UpdateUnsigned { build } => words.update_unsigned(build),
            Cause::UpdateGone { build } => words.update_gone(build),
            Cause::UpdateUnwritable { folder } => words.update_unwritable(folder),
            Cause::UpdateMalformed { build, reason } => words.update_malformed(build, reason),
        };
        vec![line]
    }
}

#[derive(Resource, Default)]
struct Shown {
    progress: SyncProgress,
    card: Option<CardModel>,
    early: EarlyWords,
}

/// The game's words before its content has loaded, read from its rows as soon as the sync has
/// them, so the card speaks while the media still downloads.
#[derive(Default)]
enum EarlyWords {
    #[default]
    Awaited,
    Read(Box<Messages>),
    Unreadable,
}

#[derive(Clone, PartialEq)]
struct CardModel {
    title: String,
    lines: Vec<String>,
    notes: Vec<String>,
    retry_in: Option<String>,
    done: Option<f32>,
    actions: Vec<CardAction>,
    veiled: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum CardAction {
    Retry,
    Cancel,
    Close,
}

/// What the card shows of an update under way.
struct UpdateShown {
    to: String,
    step: UpdateStep,
}

struct ServerContent {
    http: Http,
    base: String,
}

impl ContentSource for ServerContent {
    fn get(&self, path: &str) -> BoxFuture<'static, Result<Vec<u8>, FetchFailure>> {
        Box::pin(self.http.get(format!("{}{path}", self.base)))
    }
}

fn begin(world: &mut World) {
    world.remove_resource::<LoadingFailure>();
    start(world, Retry::default());
}

fn forget(world: &mut World) {
    world.remove_resource::<Syncing>();
    world.remove_resource::<Updating>();
    world.remove_resource::<Restarting>();
    world.remove_resource::<LoadingFailure>();
    frame_as_before(world);
    *world.resource_mut::<Shown>() = Shown::default();
}

fn start(world: &mut World, retry: Retry) {
    frame_slowly(world);
    let http = world.resource::<Http>().clone();
    let store = world.resource::<Profile>().content_store();
    let base = world.resource::<StartParams>().game_server_url.clone();
    let installed = world
        .get_resource::<SyncedContent>()
        .map(|synced| synced.manifest.clone());
    let status = SyncStatus::default();
    let task = AsyncComputeTaskPool::get().spawn({
        let status = status.clone();
        async move {
            let source = Arc::new(ServerContent { http, base });
            let synced = sync(
                source,
                &store,
                version::build(),
                SyncPolicy::default(),
                status.clone(),
            )
            .await?;
            if installed.is_some_and(|installed| installed.same_content(&synced.manifest)) {
                return Ok(None);
            }
            status.update(|progress| progress.phase = SyncPhase::Building);
            let content = load_content(
                Arc::new(synced.folder()),
                "the content did not load",
                LoadPurpose::Play,
            )
            .map_err(|report| SyncError::Failed(report.to_string()))?;
            Ok(Some(Brought { synced, content }))
        }
    });
    world.insert_resource(Syncing {
        task,
        status,
        retry,
    });
}

fn poll(world: &mut World) {
    let now = time::uptime(world);
    if let Some((updating, outcome)) =
        super::finished::<Updating, _>(world, |updating| &mut updating.task)
    {
        updated(world, updating, outcome, now);
        return;
    }
    if world.contains_resource::<Updating>() || world.contains_resource::<Restarting>() {
        return;
    }
    if !world.contains_resource::<Syncing>() {
        let due = world
            .get_resource::<LoadingFailure>()
            .is_some_and(|failure| failure.retry.is_due(now));
        if due && let Some(failure) = world.remove_resource::<LoadingFailure>() {
            start(world, failure.retry);
        }
        return;
    }
    let Some((syncing, outcome)) =
        super::finished::<Syncing, _>(world, |syncing| &mut syncing.task)
    else {
        return;
    };
    match outcome {
        Ok(brought) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            if let Some(Brought { synced, content }) = brought {
                install_content(world, synced, content);
            }
            let next = if world.contains_resource::<Credentials>() {
                ClientScreen::Connecting
            } else {
                ClientScreen::SigningIn
            };
            world.resource_mut::<NextState<ClientScreen>>().set(next);
        }
        Err(SyncError::OtherBuild { server }) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            let answer = world.resource::<StartParams>().other_build.clone();
            match answer {
                OtherBuildAnswer::Update(source) => {
                    begin_update(world, source, server, syncing.retry)
                }
                OtherBuildAnswer::Updated { .. } => {
                    stop(world, Cause::ServerRunsNewer { build: server })
                }
                OtherBuildAnswer::ByHand => stop(world, Cause::ServerRunsOther { build: server }),
            }
        }
        Err(SyncError::Failed(reason)) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            fail(world, Cause::DownloadFailed { reason }, now, syncing.retry);
        }
    }
}

fn fail(world: &mut World, cause: Cause, now: Seconds, retry: Retry) {
    let retry = retry.failed(now, &mut world.resource_mut::<Rng>());
    world.insert_resource(LoadingFailure { cause, retry });
}

/// Stops on `cause`, which no try can change.
fn stop(world: &mut World, cause: Cause) {
    world.insert_resource(LoadingFailure {
        cause,
        retry: Retry::default().held(),
    });
}

fn begin_update(world: &mut World, source: ReleaseSource, to: String, retry: Retry) {
    let http = world.resource::<Http>().clone();
    let step = Arc::new(Mutex::new(UpdateStep::Downloading { got: 0, of: None }));
    let task = AsyncComputeTaskPool::get().spawn({
        let step = step.clone();
        let to = to.clone();
        blocking::unblock(move || {
            let tell = |now: UpdateStep| {
                *step.lock().unwrap_or_else(PoisonError::into_inner) = now;
            };
            let client = source.fetch(&http, &to, &tell)?;
            tell(UpdateStep::Replacing);
            update::replace_running(&client)
        })
    });
    world.insert_resource(Updating {
        task,
        step,
        to,
        retry,
    });
}

fn updated(
    world: &mut World,
    updating: Updating,
    outcome: Result<PathBuf, UpdateFailure>,
    now: Seconds,
) {
    let to = updating.to.clone();
    let failure = match outcome {
        Ok(installed) => match update::restart(
            &installed,
            &ClientConfig::updated_from_var(),
            version::build(),
        ) {
            Ok(()) => {
                world.write_message(AppExit::Success);
                world.insert_resource(Restarting { to });
                return;
            }
            Err(error) => Cause::UpdateDidNotRestart {
                build: to,
                reason: error.to_string(),
            },
        },
        Err(UpdateFailure::Download(reason)) => {
            let cause = Cause::UpdateStopped { build: to, reason };
            fail(world, cause, now, updating.retry);
            return;
        }
        Err(UpdateFailure::Unsigned) => Cause::UpdateUnsigned { build: to },
        Err(UpdateFailure::Gone) => Cause::UpdateGone { build: to },
        Err(UpdateFailure::Unwritable { folder, .. }) => Cause::UpdateUnwritable {
            folder: folder.display().to_string(),
        },
        Err(UpdateFailure::Malformed(reason)) => Cause::UpdateMalformed { build: to, reason },
    };
    stop(world, failure);
}

fn read_words_early(world: &mut World) {
    let shown = world.resource::<Shown>();
    let loaded = world
        .get_resource::<Content>()
        .is_some_and(|content| content.try_built::<Messages>().is_some());
    if loaded || !matches!(shown.early, EarlyWords::Awaited) {
        return;
    }
    let Some(rows) = world
        .get_resource::<Syncing>()
        .and_then(|syncing| syncing.status.rows())
    else {
        return;
    };
    world.resource_mut::<Shown>().early = match messages::read_alone(rows) {
        Some(read) => EarlyWords::Read(Box::new(read)),
        None => EarlyWords::Unreadable,
    };
}

fn show(world: &mut World) {
    if let Some(syncing) = world.get_resource::<Syncing>() {
        let latest = syncing.status.latest();
        world.resource_mut::<Shown>().progress = latest;
    }
    let failure = world.get_resource::<LoadingFailure>().cloned();
    let veiled = world.resource::<ActiveTransition>().phase() != ScreenTransitionPhase::Idle;
    let now = time::uptime(world);
    let signed_in = world.contains_resource::<Credentials>();
    let update = match (
        world.get_resource::<Updating>(),
        world.get_resource::<Restarting>(),
    ) {
        (Some(updating), _) => Some(UpdateShown {
            to: updating.to.clone(),
            step: *updating.step.lock().unwrap_or_else(PoisonError::into_inner),
        }),
        (None, Some(restarting)) => Some(UpdateShown {
            to: restarting.to.clone(),
            step: UpdateStep::Restarting,
        }),
        (None, None) => None,
    };
    let content = world.get_resource::<Content>().cloned();
    let model = {
        let shown = world.resource::<Shown>();
        card_model(
            words(content.as_ref(), shown),
            &shown.progress,
            failure.as_ref(),
            update.as_ref(),
            now,
            veiled,
            signed_in,
        )
    };
    if world.resource::<Shown>().card.as_ref() == Some(&model) {
        return;
    }
    if let Ok(mut window) = world
        .query_filtered::<&mut Window, With<bevy::window::PrimaryWindow>>()
        .single_mut(world)
        && !model.title.is_empty()
        && window.title != model.title
    {
        window.title = model.title.clone();
    }
    world.resource_mut::<Shown>().card = Some(model);
}

fn card_model(
    messages: Option<&Messages>,
    progress: &SyncProgress,
    failure: Option<&LoadingFailure>,
    update: Option<&UpdateShown>,
    now: Seconds,
    veiled: bool,
    signed_in: bool,
) -> CardModel {
    let (actions, update_done) = card_actions(failure, update);
    let done = update_done.or_else(|| failure.is_none().then(|| sync_share(progress)).flatten());
    let Some(messages) = messages else {
        return CardModel {
            title: progress.name.clone().unwrap_or_default(),
            lines: Vec::new(),
            notes: Vec::new(),
            retry_in: None,
            done,
            actions,
            veiled,
        };
    };
    let screen = &messages.screen;
    let title = progress
        .name
        .clone()
        .unwrap_or_else(|| screen.loading().to_owned());
    let mut lines = Vec::new();
    let mut notes = Vec::new();
    match &progress.phase {
        SyncPhase::Checking => lines.push(screen.checking().to_owned()),
        SyncPhase::Reading => {
            lines.push(screen.content_checked().to_owned());
            lines.push(screen.read_from_device().to_owned());
            lines.push(megabytes_of(
                screen,
                progress.read.bytes,
                progress.read.bytes_total,
            ));
        }
        SyncPhase::Downloading { updating } => {
            lines.push(screen.content_checked().to_owned());
            let fetched = progress.fetched;
            if fetched.files_total == 0 && !progress.rows_changed {
                lines.push(screen.up_to_date().to_owned());
                lines.push(screen.read_from_device_size(megabytes(progress.read.bytes_total)));
            } else if *updating {
                lines.push(screen.downloading_changes().to_owned());
                lines.push(megabytes_of(screen, fetched.bytes, fetched.bytes_total));
                let (done, total) = (fetched.files_total, progress.files_in_index);
                lines.push(if progress.rows_changed {
                    screen.files_and_rows_changed(done, total)
                } else {
                    screen.files_changed(done, total)
                });
            } else {
                lines.push(screen.downloading().to_owned());
                lines.push(megabytes_of(screen, fetched.bytes, fetched.bytes_total));
                lines.push(screen.files_of(fetched.files, fetched.files_total));
            }
        }
        SyncPhase::Done | SyncPhase::Building => {
            lines.push(screen.content_checked().to_owned());
            lines.push(
                if progress.fetched.files_total == 0 && !progress.rows_changed {
                    screen.up_to_date().to_owned()
                } else {
                    screen.downloaded().to_owned()
                },
            );
            if progress.phase == SyncPhase::Building {
                lines.push(screen.building().to_owned());
            } else if signed_in {
                lines.push(screen.joining().to_owned());
            }
        }
    }
    if progress.retrying > 0 {
        notes.push(screen.retrying_files(progress.retrying));
    }
    if progress.memory_only {
        notes.push(screen.not_kept().to_owned());
    }
    let mut retry_in = None;
    if let Some(update) = update {
        lines = vec![screen.updating(version::build(), &update.to)];
        notes.clear();
        match update.step {
            UpdateStep::Downloading { got, of } => {
                lines.push(match of {
                    Some(of) => screen.updating_of(megabytes(got), megabytes(of)),
                    None => screen.updating_of_unknown(megabytes(got)),
                });
            }
            UpdateStep::Verifying => {
                lines.push(screen.downloaded().to_owned());
                lines.push(screen.checking_signature().to_owned());
            }
            UpdateStep::Replacing => {
                lines.push(screen.signature_checked().to_owned());
                lines.push(screen.installing().to_owned());
            }
            UpdateStep::Restarting => {
                lines.push(screen.installed().to_owned());
                lines.push(screen.restarting().to_owned());
            }
        }
    }
    if let Some(failure) = failure {
        lines = failure.cause.lines(screen);
        retry_in = failure
            .retry
            .due()
            .map(|at| screen.retrying_in(messages.time.rough(at - now)));
    }
    CardModel {
        title,
        lines,
        notes,
        retry_in,
        done,
        actions,
        veiled,
    }
}

fn loading_card(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(model) = world.resource::<Shown>().card.clone() else {
        return;
    };
    let backdrop = (!model.veiled).then_some(ink::VEIL);
    let notes: Vec<String> = model.notes.iter().chain(&model.retry_in).cloned().collect();
    let content = world.get_resource::<Content>().cloned();
    let messages = words(content.as_ref(), world.resource::<Shown>());
    let labels: Vec<RichText> = model
        .actions
        .iter()
        .map(|action| action.label(messages))
        .collect();
    let notice = Notice {
        title: &model.title,
        lines: &model.lines,
        notes: &notes,
        progress: model.done,
        actions: &labels,
    };
    let taken = notice_screen(&gui, "loading", backdrop, notice)
        .and_then(|index| model.actions.get(index).copied());
    match taken {
        Some(CardAction::Retry)
            if world.get_resource::<Syncing>().is_none()
                && world.get_resource::<Updating>().is_none() =>
        {
            let retry = world
                .remove_resource::<LoadingFailure>()
                .map_or(Retry::default(), |failure| failure.retry);
            start(world, retry);
        }
        Some(CardAction::Cancel | CardAction::Close) => {
            world.write_message(AppExit::Success);
        }
        Some(CardAction::Retry) | None => {}
    }
}

/// What the card lets the player do, and how far along an update is: the same whether or not the
/// game's words have loaded yet.
fn card_actions(
    failure: Option<&LoadingFailure>,
    update: Option<&UpdateShown>,
) -> (Vec<CardAction>, Option<f32>) {
    let mut actions = Vec::new();
    let mut done = None;
    match update.map(|update| update.step) {
        Some(UpdateStep::Downloading { got, of }) => {
            done = Some(of.map_or(0.0, |of| got as f32 / of.max(1) as f32));
            actions.push(CardAction::Cancel);
        }
        Some(UpdateStep::Verifying) => actions.push(CardAction::Cancel),
        Some(UpdateStep::Replacing | UpdateStep::Restarting) | None => {}
    }
    if let Some(failure) = failure {
        if failure.retry.due().is_some() {
            actions.push(CardAction::Retry);
        }
        actions.push(CardAction::Close);
    }
    (actions, done)
}

impl CardAction {
    /// What the action's button says: its words, or a picture of it before the game's words have
    /// loaded.
    fn label(self, messages: Option<&Messages>) -> RichText {
        match (self, messages) {
            (CardAction::Retry, Some(messages)) => RichText::new(messages.screen.retry_now()),
            (CardAction::Cancel, Some(messages)) => RichText::new(messages.interface.cancel()),
            (CardAction::Close, Some(messages)) => RichText::new(messages.interface.close()),
            (CardAction::Retry, None) => Icon::regular(phosphor::ARROW_CLOCKWISE).text(ICON_SIZE),
            (CardAction::Cancel | CardAction::Close, None) => {
                Icon::regular(phosphor::X).text(ICON_SIZE)
            }
        }
    }
}

/// The game's words: the loaded content's, or before it has loaded, those read early.
fn words<'a>(content: Option<&'a Content>, shown: &'a Shown) -> Option<&'a Messages> {
    content
        .and_then(|content| content.try_built::<Messages>())
        .or(match &shown.early {
            EarlyWords::Read(messages) => Some(messages),
            EarlyWords::Awaited | EarlyWords::Unreadable => None,
        })
}

/// How much of the content the sync has read or downloaded, while it reads or downloads some.
fn sync_share(progress: &SyncProgress) -> Option<f32> {
    let tally = match progress.phase {
        SyncPhase::Reading => progress.read,
        SyncPhase::Downloading { .. } => progress.fetched,
        SyncPhase::Checking | SyncPhase::Done | SyncPhase::Building => return None,
    };
    (tally.bytes_total > 0).then(|| tally.bytes as f32 / tally.bytes_total as f32)
}

fn megabytes_of(messages: &ScreenMessages, done: u64, total: u64) -> String {
    messages.megabytes_of(megabytes(done), megabytes(total))
}

fn megabytes(bytes: u64) -> f64 {
    (bytes as f64 / 100_000.0).round() / 10.0
}

const ICON_SIZE: f32 = 18.0;
