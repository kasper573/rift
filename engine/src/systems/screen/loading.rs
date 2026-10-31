use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_egui::EguiPrimaryContextPass;
use ui::ink;

use super::{
    ClientScreen, Notice, Retry, notice_screen, pace_as_before, pace_eagerly, pace_slowly,
};
use crate::core::content::sync::{
    ContentSource, Counted, SyncError, SyncPhase, SyncPolicy, SyncProgress, SyncStatus,
    SyncedContent, sync,
};
use crate::core::content::{Content, LoadPurpose, load_paced, version};
use crate::core::gui::{self, Band};
use crate::core::math::Rng;
use crate::core::net::auth::Credentials;
use crate::core::pace::Pace;
use crate::core::platform::{ClientPlatform, FetchFailure, LocalFuture, Platform, StartParams};
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::time::Seconds;
use crate::systems::{install_content, registry};
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
    building: bool,
}

/// New content, synced and loaded.
struct Brought {
    synced: SyncedContent,
    content: Content,
}

#[derive(Resource, Clone, PartialEq)]
struct LoadingFailure {
    reason: String,
    retry: Retry,
}

#[derive(Resource, Default)]
struct Shown {
    progress: SyncProgress,
    card: Option<CardModel>,
    announced: Option<String>,
}

#[derive(Clone, PartialEq)]
struct CardModel {
    title: String,
    lines: Vec<String>,
    notes: Vec<String>,
    // Shown but never announced: it counts down every second.
    retry_in: Option<String>,
    veiled: bool,
}

struct ServerContent {
    platform: Arc<dyn Platform>,
    base: String,
}

impl ContentSource for ServerContent {
    fn get(&self, path: &str) -> LocalFuture<Result<Vec<u8>, FetchFailure>> {
        self.platform.get(format!("{}{path}", self.base))
    }
}

fn begin(world: &mut World) {
    world.remove_resource::<LoadingFailure>();
    start(world, Retry::default());
}

fn forget(world: &mut World) {
    world.remove_resource::<Syncing>();
    world.remove_resource::<LoadingFailure>();
    pace_as_before(world);
    *world.resource_mut::<Shown>() = Shown::default();
}

fn start(world: &mut World, retry: Retry) {
    pace_slowly(world);
    let platform = world.resource::<ClientPlatform>().0.clone();
    let base = world.resource::<StartParams>().game_server_url.clone();
    let installed = world
        .get_resource::<SyncedContent>()
        .map(|synced| synced.manifest.clone());
    let pace = world.resource::<Pace>().clone();
    let status = SyncStatus::default();
    let task = IoTaskPool::get().spawn_local({
        let status = status.clone();
        async move {
            let source = ServerContent {
                platform: platform.clone(),
                base,
            };
            let store = platform.content_store();
            let synced = sync(
                &source,
                &*store,
                version::build(),
                SyncPolicy::default(),
                status.clone(),
            )
            .await?;
            if installed.is_some_and(|installed| installed.same_content(&synced.manifest)) {
                return Ok(None);
            }
            status.update(|progress| progress.phase = SyncPhase::Building);
            let content = load_paced(
                registry(),
                Arc::new(synced.folder()),
                version::build(),
                "the content did not load",
                LoadPurpose::Play,
                &pace,
            )
            .await
            .map_err(|report| SyncError::Failed(report.to_string()))?;
            Ok(Some(Brought { synced, content }))
        }
    });
    world.insert_resource(Syncing {
        task,
        status,
        retry,
        building: false,
    });
}

fn poll(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let Some(mut syncing) = world.remove_resource::<Syncing>() else {
        let due = world
            .get_resource::<LoadingFailure>()
            .is_some_and(|failure| failure.retry.is_due(now));
        if due && let Some(failure) = world.remove_resource::<LoadingFailure>() {
            start(world, failure.retry);
        }
        return;
    };
    match block_on(future::poll_once(&mut syncing.task)) {
        None => {
            if !syncing.building && syncing.status.latest().phase == SyncPhase::Building {
                syncing.building = true;
                pace_eagerly(world);
            }
            world.insert_resource(syncing);
        }
        Some(Ok(brought)) => {
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
        Some(Err(SyncError::OtherBuild { server })) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            if let Err(reason) = world.resource::<ClientPlatform>().0.relaunch(&server) {
                world.insert_resource(LoadingFailure {
                    reason,
                    retry: Retry {
                        due: None,
                        ..syncing.retry
                    },
                });
            }
        }
        Some(Err(SyncError::Failed(reason))) => {
            world.resource_mut::<Shown>().progress = syncing.status.latest();
            fail(world, reason, now, syncing.retry);
        }
    }
}

fn fail(world: &mut World, reason: String, now: Seconds, retry: Retry) {
    let retry = retry.failed(now, &mut world.resource_mut::<Rng>());
    world.insert_resource(LoadingFailure { reason, retry });
}

fn show(world: &mut World) {
    if let Some(syncing) = world.get_resource::<Syncing>() {
        let latest = syncing.status.latest();
        world.resource_mut::<Shown>().progress = latest;
    }
    let failure = world.get_resource::<LoadingFailure>().cloned();
    let veiled = world.resource::<ActiveTransition>().phase() != ScreenTransitionPhase::Idle;
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let signed_in = world.contains_resource::<Credentials>();
    let model = {
        let shown = world.resource::<Shown>();
        card_model(&shown.progress, failure.as_ref(), now, veiled, signed_in)
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
    let status = {
        let shown = world.resource::<Shown>();
        announcement(&model, &shown.progress)
    };
    let mut shown = world.resource_mut::<Shown>();
    if shown.announced.as_ref() != Some(&status) {
        shown.announced = Some(status.clone());
        world.resource::<ClientPlatform>().0.announce(&status);
    }
    world.resource_mut::<Shown>().card = Some(model);
}

fn card_model(
    progress: &SyncProgress,
    failure: Option<&LoadingFailure>,
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
    if let Some(failure) = failure {
        lines = vec!["Download failed".to_owned(), failure.reason.clone()];
        retry_in = failure
            .retry
            .due
            .map(|at| format!("Retrying in {}", (at - now).in_words()));
    }
    CardModel {
        title,
        lines,
        notes,
        retry_in,
        veiled,
    }
}

/// What a screen reader hears: the card's words, with transfers rounded to quarters so the status
/// changes a few times, not every frame.
fn announcement(model: &CardModel, progress: &SyncProgress) -> String {
    let quarter = |counted: Counted| {
        let done = counted.bytes * 4 / counted.bytes_total.max(1);
        format!("{}%", done.min(4) * 25)
    };
    let mut parts = vec![model.title.clone()];
    if model.retry_in.is_some() {
        parts.extend(model.lines.iter().cloned());
    } else {
        match progress.phase {
            SyncPhase::Reading if progress.read.bytes_total > 0 => {
                parts.push(format!(
                    "reading from this device, {}",
                    quarter(progress.read)
                ));
            }
            SyncPhase::Downloading { .. } if progress.fetched.bytes_total > 0 => {
                parts.push(format!("downloading, {}", quarter(progress.fetched)));
            }
            _ => parts.extend(model.lines.iter().cloned()),
        }
    }
    parts.extend(model.notes.iter().cloned());
    parts.join(". ")
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
    let notice = Notice {
        title: &model.title,
        lines: &model.lines,
        notes: &notes,
        action: model.retry_in.is_some().then_some("Retry"),
    };
    let retry = notice_screen(&gui, "loading", backdrop, notice);
    if retry && world.get_resource::<Syncing>().is_none() {
        let retry = world
            .remove_resource::<LoadingFailure>()
            .map_or(Retry::default(), |failure| failure.retry);
        start(world, retry);
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}
