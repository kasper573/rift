use std::sync::{Arc, Mutex, PoisonError};

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_egui::EguiPrimaryContextPass;
use ui::ink;

use super::{ClientScreen, Notice, Retry, frame_as_before, frame_slowly, notice_screen};
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
use crate::systems::{install_content, load_content};
use futures_util::future::BoxFuture;
use ui::words;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shown>()
            .add_systems(OnEnter(ClientScreen::Loading), begin)
            .add_systems(
                Update,
                (poll, show).chain().run_if(in_state(ClientScreen::Loading)),
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
    task: Task<Result<(), UpdateFailure>>,
    step: Arc<Mutex<UpdateStep>>,
    to: String,
    retry: Retry,
}

/// The server's build is installed, and the client is starting again on it.
#[derive(Resource)]
struct Restarting {
    to: String,
}

/// What stopped the load, said in lines. A failure whose retry is due tries again; one whose
/// retry waits for nothing can only be closed.
#[derive(Resource, Clone, PartialEq)]
struct LoadingFailure {
    lines: Vec<String>,
    retry: Retry,
}

#[derive(Resource, Default)]
struct Shown {
    progress: SyncProgress,
    card: Option<CardModel>,
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
                OtherBuildAnswer::Updated { .. } => stop(
                    world,
                    format!(
                        "The server now runs {server}, a build this update didn't install. \
                         Start the game again to update to it."
                    ),
                ),
                OtherBuildAnswer::ByHand => stop(
                    world,
                    format!(
                        "The server runs build {server}. Start a game client of that build to play."
                    ),
                ),
            }
        }
        Err(SyncError::Failed(reason)) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            fail(
                world,
                vec!["Download failed".to_owned(), reason],
                now,
                syncing.retry,
            );
        }
    }
}

fn fail(world: &mut World, lines: Vec<String>, now: Seconds, retry: Retry) {
    let retry = retry.failed(now, &mut world.resource_mut::<Rng>());
    world.insert_resource(LoadingFailure { lines, retry });
}

/// Stops on `reason`, which no try can change.
fn stop(world: &mut World, reason: String) {
    world.insert_resource(LoadingFailure {
        lines: vec![reason],
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
    outcome: Result<(), UpdateFailure>,
    now: Seconds,
) {
    let to = &updating.to;
    let failure = match outcome {
        Ok(()) => match update::restart(&ClientConfig::updated_from_var(), version::build()) {
            Ok(()) => {
                world.write_message(AppExit::Success);
                world.insert_resource(Restarting { to: to.clone() });
                return;
            }
            Err(error) => format!(
                "The game client is updated to {to}, but didn't start again: {error}. \
                     Start the game again to play."
            ),
        },
        Err(UpdateFailure::Download(reason)) => {
            let lines = vec![format!("The download of {to} stopped: {reason}.")];
            fail(world, lines, now, updating.retry);
            return;
        }
        Err(UpdateFailure::Unsigned) => {
            format!("The download for {to} isn't signed by the release key. Nothing was installed.")
        }
        Err(UpdateFailure::Gone) => format!(
            "The server runs {to}, which is no longer published. Ask the game's hosts to move \
             to a newer release."
        ),
        Err(UpdateFailure::Unwritable { folder, .. }) => format!(
            "The game client can't update itself in {}. Move the game's folder somewhere you \
             can write to, or download the new client from the game's website.",
            folder.display()
        ),
        Err(UpdateFailure::Malformed(reason)) => {
            format!("The download for {to} holds no game client ({reason}). Nothing was installed.")
        }
    };
    stop(world, failure);
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
    let model = {
        let shown = world.resource::<Shown>();
        card_model(
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
        && window.title != model.title
    {
        window.title = model.title.clone();
    }
    world.resource_mut::<Shown>().card = Some(model);
}

fn card_model(
    progress: &SyncProgress,
    failure: Option<&LoadingFailure>,
    update: Option<&UpdateShown>,
    now: Seconds,
    veiled: bool,
    signed_in: bool,
) -> CardModel {
    let title = progress
        .name
        .clone()
        .unwrap_or_else(|| "Loading".to_owned());
    let mut lines = Vec::new();
    let mut notes = Vec::new();
    match &progress.phase {
        SyncPhase::Checking => lines.push("Checking content…".to_owned()),
        SyncPhase::Reading => {
            lines.push("Content checked".to_owned());
            lines.push("Read from this device".to_owned());
            lines.push(format!(
                "{} of {} MB",
                megabytes(progress.read.bytes),
                megabytes(progress.read.bytes_total)
            ));
        }
        SyncPhase::Downloading { updating } => {
            lines.push("Content checked".to_owned());
            let fetched = progress.fetched;
            if fetched.files_total == 0 && !progress.rows_changed {
                lines.push("Up to date".to_owned());
                lines.push(format!(
                    "Read from this device {} MB",
                    megabytes(progress.read.bytes_total)
                ));
            } else if *updating {
                lines.push("Downloading changes".to_owned());
                lines.push(format!(
                    "{} of {} MB",
                    megabytes(fetched.bytes),
                    megabytes(fetched.bytes_total)
                ));
                lines.push(format!(
                    "{} of {} files changed{}",
                    words::thousands(fetched.files_total),
                    words::thousands(progress.files_in_index),
                    if progress.rows_changed {
                        ", and the rows"
                    } else {
                        ""
                    }
                ));
            } else {
                lines.push("Downloading".to_owned());
                lines.push(format!(
                    "{} of {} MB",
                    megabytes(fetched.bytes),
                    megabytes(fetched.bytes_total)
                ));
                lines.push(format!(
                    "{} of {} files",
                    words::thousands(fetched.files),
                    words::thousands(fetched.files_total)
                ));
            }
        }
        SyncPhase::Done | SyncPhase::Building => {
            lines.push("Content checked".to_owned());
            lines.push(
                if progress.fetched.files_total == 0 && !progress.rows_changed {
                    "Up to date".to_owned()
                } else {
                    "Downloaded".to_owned()
                },
            );
            if progress.phase == SyncPhase::Building {
                lines.push("Building the game…".to_owned());
            } else if signed_in {
                lines.push("Joining".to_owned());
            }
        }
    }
    if progress.retrying > 0 {
        notes.push(format!(
            "Retrying {} {}",
            progress.retrying,
            if progress.retrying == 1 {
                "file"
            } else {
                "files"
            }
        ));
    }
    if progress.memory_only {
        notes.push("Files won't be kept on this device".to_owned());
    }
    let mut retry_in = None;
    let mut done = None;
    let mut actions = Vec::new();
    if let Some(update) = update {
        lines = vec![format!(
            "The server runs another build of the game client. Updating from {} to {}.",
            version::build(),
            update.to
        )];
        notes.clear();
        match update.step {
            UpdateStep::Downloading { got, of } => {
                done = Some(of.map_or(0.0, |of| got as f32 / of.max(1) as f32));
                lines.push(match of {
                    Some(of) => format!("Downloading · {} of {} MB", megabytes(got), megabytes(of)),
                    None => format!("Downloading · {} MB", megabytes(got)),
                });
                actions.push(CardAction::Cancel);
            }
            UpdateStep::Verifying => {
                lines.push("Downloaded".to_owned());
                lines.push("Checking the signature…".to_owned());
                actions.push(CardAction::Cancel);
            }
            UpdateStep::Replacing => {
                lines.push("Signature checked".to_owned());
                lines.push("Installing…".to_owned());
            }
            UpdateStep::Restarting => {
                lines.push("Installed".to_owned());
                lines.push("Restarting…".to_owned());
            }
        }
    }
    if let Some(failure) = failure {
        lines = failure.lines.clone();
        retry_in = failure
            .retry
            .due()
            .map(|at| format!("Retrying in {}", (at - now).in_words()));
        if retry_in.is_some() {
            actions.push(CardAction::Retry);
        }
        actions.push(CardAction::Close);
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
    let labels: Vec<&str> = model.actions.iter().map(|action| action.label()).collect();
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

impl CardAction {
    fn label(self) -> &'static str {
        match self {
            CardAction::Retry => "Retry now",
            CardAction::Cancel => "Cancel",
            CardAction::Close => "Close",
        }
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}
