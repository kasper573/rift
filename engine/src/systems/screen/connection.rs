use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task};
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::ink;

use super::sign_in::SignInFailure;
use super::{ClientScreen, Notice, Retry, notice_screen};
use crate::core::content::sync::SyncedContent;
use crate::core::content::version;
use crate::core::gui::{self, Band};
use crate::core::math::Rng;
use crate::core::net::SessionRequest;
use crate::core::net::auth::Credentials;
use crate::core::net::transport::{Connection, read_token};
use crate::core::os::{FetchFailure, Http, StartParams};
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::time::{self, Seconds};
use crate::systems::player::{JoinRequest, session};

pub struct ConnectionPlugin;

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Retry>()
            .init_resource::<Backdrop>()
            .add_systems(OnEnter(ClientScreen::Connecting), (veil, begin_session))
            .add_systems(OnEnter(ClientScreen::Lost), (veil, lost))
            .add_systems(OnEnter(ClientScreen::Area), settle)
            .add_systems(Update, retry_when_due.run_if(in_state(ClientScreen::Lost)))
            .add_systems(
                EguiPrimaryContextPass,
                (
                    connecting.run_if(in_state(ClientScreen::Connecting)),
                    connection_lost.run_if(in_state(ClientScreen::Lost)),
                )
                    .in_set(Band::System),
            );
    }
}

/// Set once the server welcomes the connection, until the client asks to join.
#[derive(Resource)]
pub(super) struct Announce;

pub(super) fn announce(world: &mut World) {
    if world.get_resource::<Announce>().is_none() || session::my_id(world).is_none() {
        return;
    }
    info!("connection welcomed; joining");
    world.write_message(JoinRequest);
    world.remove_resource::<Announce>();
}

#[derive(Resource)]
pub struct PendingSession {
    task: Task<Result<String, FetchFailure>>,
}

fn begin_session(world: &mut World) {
    let Some(credentials) = world.get_resource::<Credentials>().cloned() else {
        error!("no access token; cannot open a session");
        return;
    };
    let game_server_url = world.resource::<StartParams>().game_server_url.clone();
    let rows = world.resource::<SyncedContent>().manifest.rows.hash.clone();
    let body = serde_json::to_string(&SessionRequest {
        build: version::build().to_owned(),
        rows,
    })
    .expect("a session request serializes");
    let fetch = world.resource::<Http>().post(
        format!("{game_server_url}/session"),
        credentials.authorization,
        body,
    );
    let task = IoTaskPool::get().spawn(fetch);
    world.insert_resource(PendingSession { task });
}

pub(super) fn poll_session(world: &mut World) {
    let Some((_, outcome)) =
        super::finished::<PendingSession, _>(world, |pending| &mut pending.task)
    else {
        return;
    };
    match outcome {
        Ok(token) => match read_token(&token).map(Connection::open) {
            Some(Ok(connection)) => {
                world.insert_resource(connection);
                world.insert_resource(Announce);
                info!("connection opened");
            }
            Some(Err(error)) => error!("could not open a connection: {error}"),
            None => error!("the server handed out no connect token"),
        },
        Err(FetchFailure::Status { status: 401, body }) => {
            info!("the server refused the sign-in: {body}");
            world.remove_resource::<Credentials>();
            world.insert_resource(SignInFailure(
                "The server didn't take your sign-in.".to_owned(),
            ));
            world
                .resource_mut::<NextState<ClientScreen>>()
                .set(ClientScreen::SigningIn);
        }
        // Loading again finds out what changed, and catches the client up or relaunches it.
        Err(FetchFailure::Status { status: 409, body }) => {
            info!("the server refused the session: {body}");
            world
                .resource_mut::<NextState<ClientScreen>>()
                .set(ClientScreen::Loading);
        }
        Err(error) => error!("could not open a session: {error}"),
    }
}

/// Whether the screen hides the world, decided as it opens: a transition already under way
/// covers the world itself.
#[derive(Resource, Default)]
struct Backdrop(Option<egui::Color32>);

fn veil(transition: Res<ActiveTransition>, mut backdrop: ResMut<Backdrop>) {
    backdrop.0 = overlay(transition.phase());
}

fn connecting(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let notice = Notice {
        title: "Connecting…",
        lines: &[],
        notes: &[],
        action: None,
    };
    notice_screen(&gui, "connecting", world.resource::<Backdrop>().0, notice);
}

fn lost(world: &mut World) {
    let now = time::uptime(world);
    let retry = *world.resource::<Retry>();
    let next = retry.failed(now, &mut world.resource_mut::<Rng>());
    world.insert_resource(next);
}

fn connection_lost(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let notice = Notice {
        title: "Connection lost",
        lines: &["Reconnecting…".to_owned()],
        notes: &[],
        action: Some("Retry now"),
    };
    let retry = notice_screen(&gui, "lost", world.resource::<Backdrop>().0, notice);
    if retry {
        let held = world.resource::<Retry>().held();
        world.insert_resource(held);
        world
            .resource_mut::<NextState<ClientScreen>>()
            .set(ClientScreen::Loading);
    }
}

fn settle(mut retry: ResMut<Retry>) {
    *retry = Retry::default();
}

fn retry_when_due(
    time: Res<Time>,
    mut retry: ResMut<Retry>,
    mut next: ResMut<NextState<ClientScreen>>,
) {
    let now = Seconds(time.elapsed_secs());
    if retry.is_due(now) {
        *retry = retry.held();
        next.set(ClientScreen::Loading);
    }
}

fn overlay(phase: ScreenTransitionPhase) -> Option<egui::Color32> {
    (phase == ScreenTransitionPhase::Idle).then_some(ink::VEIL)
}
