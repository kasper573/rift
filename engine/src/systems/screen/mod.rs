pub mod area;
pub mod connection;
pub mod loading;
pub mod sign_in;

use std::time::Duration;
use ui::egui;
use ui::float;

use bevy::ecs::component::Mutable;
use bevy::prelude::*;
use bevy::tasks::Task;
use bevy::tasks::futures::check_ready;
use bevy::winit::{UpdateMode, WinitSettings};
use connection::PendingSession;
use ui::controls;
use ui::egui::{TextStyle, Ui};
use ui::face;

use crate::core::gui::Band;
use crate::core::math::Rng;
use crate::core::net::auth::Credentials;
use crate::core::net::transport::Connection;
use crate::core::time::Backoff;
use crate::core::units::Seconds;
use crate::systems::messages::messages;
use ui::dialog;

const NOTICE_WIDTH: f32 = 356.0;
// The most lines a load shows on its way, so the notice keeps one height as the screens and their
// lines come and go. Notes and a failure are rare, and grow it instead.
const NOTICE_LINES: usize = 4;
const LINE_GAP: f32 = 4.0;

const SERVER_RETRY: Backoff = Backoff {
    first: Seconds(1.0),
    longest: Seconds(30.0),
};

/// How many tries at the server have failed in a row, and when the next one is due. A due time of
/// `None` means the next try waits for the player.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Retry {
    failures: u32,
    due: Option<Seconds>,
}

impl Retry {
    pub fn failed(self, now: Seconds, rng: &mut Rng) -> Retry {
        let failures = self.failures + 1;
        Retry {
            failures,
            due: Some(now + SERVER_RETRY.after(failures, rng)),
        }
    }

    /// The same tries, the next waiting for the player.
    pub fn held(self) -> Retry {
        Retry { due: None, ..self }
    }

    pub fn due(self) -> Option<Seconds> {
        self.due
    }

    pub fn is_due(self, now: Seconds) -> bool {
        self.due.is_some_and(|due| now >= due)
    }
}

#[derive(States, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ClientScreen {
    Loading,
    SigningIn,
    Connecting,
    Lost,
    Area,
}

/// Loads the game, then joins signed in as `credentials`, or asks for a sign-in without them.
pub struct ClientScreenPlugin {
    pub credentials: Option<Credentials>,
}

impl Plugin for ClientScreenPlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(ClientScreen::Loading);
        if let Some(credentials) = &self.credentials {
            app.insert_resource(credentials.clone());
        }
        app.add_plugins((
            sign_in::SignInPlugin,
            loading::LoadingPlugin,
            connection::ConnectionPlugin,
            area::AreaScreenPlugin,
        ))
        .add_systems(
            Update,
            (
                connection::poll_session,
                drive,
                connection::join_once_welcomed,
            )
                .chain(),
        );
    }
}

fn drive(
    pending: Option<Res<PendingSession>>,
    connection: Option<Res<Connection>>,
    scene: Res<State<ClientScreen>>,
    mut next: ResMut<NextState<ClientScreen>>,
) {
    if matches!(scene.get(), ClientScreen::SigningIn | ClientScreen::Loading)
        || !matches!(*next, NextState::Unchanged)
    {
        return;
    }
    let connected = connection
        .as_ref()
        .is_some_and(|connection| connection.is_connected());
    let attempting = pending.is_some()
        || connection
            .as_ref()
            .is_some_and(|connection| connection.is_connecting());
    let target = if connected {
        ClientScreen::Area
    } else if attempting {
        ClientScreen::Connecting
    } else {
        ClientScreen::Lost
    };
    if *scene.get() != target {
        next.set(target);
    }
}

/// What a screen between screens says, always in one panel.
pub(crate) struct Notice<'a> {
    pub title: &'a str,
    pub lines: &'a [String],
    pub notes: &'a [String],
    /// How much of the work it waits on is done.
    pub progress: Option<f32>,
    /// The buttons at its foot, the first the one most players want.
    pub actions: &'a [egui::RichText],
}

/// Returns which of the notice's actions was taken.
pub(crate) fn notice_screen(
    gui: &egui::Context,
    id: &str,
    backdrop: Option<egui::Color32>,
    notice: Notice,
) -> Option<usize> {
    system_screen(gui, id, backdrop, |ui| notice_panel(ui, notice)).inner
}

pub(crate) fn system_screen<R>(
    gui: &egui::Context,
    id: &str,
    backdrop: Option<egui::Color32>,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let area = egui::Area::new(egui::Id::new(("screen", id)))
        .order(Band::System.order())
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO);
    let shown = float::anchored(gui, area, |ui| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 16.0;
            add(ui)
        })
        .inner
    });
    let content = shown.response.layer_id;
    match backdrop {
        Some(fill) => {
            let behind = egui::Area::new(egui::Id::new(("backdrop", id)))
                .order(Band::System.order())
                .fixed_pos(egui::Pos2::ZERO)
                .show(gui, |ui| {
                    let screen = ui.ctx().content_rect();
                    ui.painter().rect_filled(screen, 0.0, fill);
                    ui.allocate_rect(screen, egui::Sense::click());
                });
            gui.move_to_top(behind.response.layer_id);
            gui.set_sublayer(behind.response.layer_id, content);
        }
        None => gui.move_to_top(content),
    }
    shown
}

pub(crate) fn panel<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    dialog::note(ui, NOTICE_WIDTH, |ui| {
        ui.set_min_height(notice_height(ui));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = LINE_GAP;
            dialog::title(ui, title);
            add(ui)
        })
        .inner
    })
}

pub(crate) fn note(ui: &mut Ui, text: &str) {
    ui.label(face::small_faint(text));
}

/// A screen that waits on another thread or the network needs only a few frames a second, and
/// every frame beyond those is time the work it waits on can't have.
fn frame_slowly(world: &mut World) {
    if !world.contains_resource::<PacedFrom>() {
        let before = world.resource::<WinitSettings>().clone();
        world.insert_resource(PacedFrom(before));
    }
    world.insert_resource(WinitSettings {
        focused_mode: UpdateMode::reactive_low_power(WAITING_FRAME),
        unfocused_mode: UpdateMode::reactive_low_power(WAITING_FRAME),
    });
}

fn frame_as_before(world: &mut World) {
    if let Some(PacedFrom(before)) = world.remove_resource::<PacedFrom>() {
        world.insert_resource(before);
    }
}

#[derive(Resource)]
struct PacedFrom(WinitSettings);

const WAITING_FRAME: Duration = Duration::from_millis(100);

fn notice_panel(ui: &mut Ui, notice: Notice) -> Option<usize> {
    panel(ui, notice.title, |ui| {
        for line in notice.lines {
            ui.label(line);
        }
        if let Some(share) = notice.progress {
            let size = egui::Vec2::new(ui.available_width(), PROGRESS_HEIGHT);
            ui::paint::progress_line(ui, size, share, ui::ink::AZURE);
        }
        for text in notice.notes {
            note(ui, text);
        }
        let mut taken = None;
        let mut buttons = |ui: &mut Ui| {
            for (index, action) in notice.actions.iter().enumerate().rev() {
                let button = if index == 0 {
                    controls::primary(ui, action.clone())
                } else {
                    controls::button(ui, action.clone())
                };
                if button.clicked() {
                    taken = Some(index);
                }
            }
        };
        match notice.actions.len() {
            0 => {}
            1 => buttons(ui),
            _ => {
                let row = egui::Layout::right_to_left(egui::Align::Center);
                ui.with_layout(row, buttons);
            }
        }
        taken
    })
}

const PROGRESS_HEIGHT: f32 = 4.0;

fn notice_height(ui: &Ui) -> f32 {
    let line = ui.text_style_height(&TextStyle::Body) + LINE_GAP;
    ui.text_style_height(&TextStyle::Heading) + NOTICE_LINES as f32 * line
}

/// What the task the resource `R` holds came to, once it has, with `R` taken out; while it runs,
/// or without an `R`, nothing, and `R` stays.
fn finished<R: Resource<Mutability = Mutable>, T>(
    world: &mut World,
    task: impl FnOnce(&mut R) -> &mut Task<T>,
) -> Option<(R, T)> {
    let mut held = world.get_resource_mut::<R>()?;
    let outcome = check_ready(task(&mut held))?;
    Some((world.remove_resource::<R>()?, outcome))
}

messages! {
    /// What the game says on the screens around play: signing in, loading, connecting.
    pub struct ScreenMessages {
        /// The title of the loading screen before the server has named the game.
        loading,
        /// What the loading screen says while it asks the server what the game's content is.
        checking,
        /// What the loading screen says once it knows what the game's content is.
        content_checked,
        /// What the loading screen says of content it reads from the player's device.
        read_from_device,
        /// What the loading screen says of how much of a download is done: `{done}` megabytes of `{total}`.
        megabytes_of(done: Number, total: Number),
        /// What the loading screen says when the content on the player's device is the server's.
        up_to_date,
        /// What the loading screen says of reading up to date content from the device: `{total}` megabytes.
        read_from_device_size(total: Number),
        /// What the loading screen says while it downloads what changed of the content on the device.
        downloading_changes,
        /// What the loading screen says of how many of the files `{total}` the server has are downloaded as changed, `{done}`.
        files_changed(done: Number, total: Number),
        /// What the loading screen says of how many of the files `{total}` the server has are downloaded as changed, `{done}`, when the rows changed too.
        files_and_rows_changed(done: Number, total: Number),
        /// What the loading screen says while it downloads the game's content.
        downloading,
        /// What the loading screen says of how many files `{done}` of all `{total}` are downloaded.
        files_of(done: Number, total: Number),
        /// What the loading screen says once the download is done.
        downloaded,
        /// What the loading screen says while it builds the game from its content.
        building,
        /// What the loading screen says while it joins the server once the game is built.
        joining,
        /// What the loading screen notes of the files, `{count}`, it tries again to download.
        retrying_files(count: Number),
        /// What the loading screen notes when the content cannot be kept on the player's device.
        not_kept,
        /// What the loading screen says of how long, `{time}`, until it tries again.
        retrying_in(time: Text),
        /// The button that tries again at once.
        retry_now,
        /// The first line of what the loading screen says when the download failed.
        download_failed,
        /// What the loading screen says while the game client updates itself from the build `{from}` to the build `{to}`.
        updating(from: Text, to: Text),
        /// What the loading screen says of how much of the new game client is downloaded: `{done}` megabytes of `{total}`.
        updating_of(done: Number, total: Number),
        /// What the loading screen says of how much of the new game client is downloaded when its size is not known: `{done}` megabytes.
        updating_of_unknown(done: Number),
        /// What the loading screen says while it checks the new game client's signature.
        checking_signature,
        /// What the loading screen says once the new game client's signature is checked.
        signature_checked,
        /// What the loading screen says while it installs the new game client.
        installing,
        /// What the loading screen says once the new game client is installed.
        installed,
        /// What the loading screen says while the game starts again as the new game client.
        restarting,
        /// What the loading screen says when the server runs the build `{build}`, which this update didn't install.
        server_runs_newer(build: Text),
        /// What the loading screen says when the server runs the build `{build}` and this client cannot update itself.
        server_runs_other(build: Text),
        /// What the loading screen says when the game client updated to `{build}` but didn't start again, for the reason `{reason}`.
        update_did_not_restart(build: Text, reason: Text),
        /// What the loading screen says when the download of the build `{build}` stopped, for the reason `{reason}`.
        update_stopped(build: Text, reason: Text),
        /// What the loading screen says when the download of the build `{build}` isn't signed by the release key.
        update_unsigned(build: Text),
        /// What the loading screen says when the build `{build}` the server runs is no longer published.
        update_gone(build: Text),
        /// What the loading screen says when the game client cannot update itself in the folder `{folder}`.
        update_unwritable(folder: Text),
        /// What the loading screen says when the download of the build `{build}` holds no game client, for the reason `{reason}`.
        update_malformed(build: Text, reason: Text),
        /// The title of the screen that shows while the game connects to the server.
        connecting,
        /// The title of the screen that shows when the connection to the server is lost.
        connection_lost,
        /// What the screen that shows when the connection is lost says while the game tries to connect again.
        reconnecting,
        /// The title of the screen that shows when the server cannot reach what it has saved.
        records_unreachable,
        /// What the screen that shows when the server cannot reach what it has saved reassures the player with.
        records_safe,
        /// The button that asks the server again once it could not reach what it has saved.
        try_again,
        /// What the sign-in screen says when the server didn't take the player's sign-in.
        sign_in_refused,
        /// What the sign-in screen says when the player is to sign in again where they started the game.
        sign_in_elsewhere,
        /// The button that signs the player in.
        sign_in,
        /// The button that takes the player to create an account.
        create_account,
        /// What the sign-in screen says while the player signs in in their browser.
        continue_to_sign_in,
        /// What the sign-in screen says while the player creates an account in their browser.
        continue_to_sign_up,
        /// The hint of the field where the player types the name of the account they sign in as.
        account_name,
        /// What the sign-in screen notes of the account name when any name signs in.
        any_account,
        /// What the screen says to a player who died, with the key `{key}` that respawns them.
        died_keyed[key: Line],
        /// What the screen says to a player who died while a respawn waits for the keys to come back, as when they type.
        died,
    }
}
