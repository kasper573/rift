use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::input::InputField;
use ui::{controls, ink};

use super::{ClientScreen, note, pace_as_before, pace_slowly, panel, system_screen};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::net::auth::Credentials;
use crate::core::os::{SignIn, StartParams, sign_in_in_browser};
use crate::systems::GameSettings;
use crate::systems::input::map::{self, InputAction};
use crate::systems::launch::fake_token;
use oidc::AuthorizationPage;

pub struct SignInPlugin;

impl Plugin for SignInPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AccountName>()
            .add_systems(OnEnter(ClientScreen::SigningIn), pace_slowly)
            .add_systems(OnExit(ClientScreen::SigningIn), (forget, pace_as_before))
            .add_systems(Update, poll.run_if(in_state(ClientScreen::SigningIn)))
            .add_systems(
                EguiPrimaryContextPass,
                sign_in_screen
                    .in_set(Band::System)
                    .run_if(in_state(ClientScreen::SigningIn)),
            );
    }
}

/// Why the player is asked to sign in again.
#[derive(Resource)]
pub struct SignInFailure(pub String);

#[derive(Resource, Default)]
struct AccountName(String);

#[derive(Resource)]
struct Waiting {
    task: Task<Result<String, String>>,
    page: AuthorizationPage,
}

enum Choice {
    Open(AuthorizationPage),
    Cancel,
    Account(String),
}

fn sign_in_screen(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let sign_in = world.resource::<StartParams>().sign_in.clone();
    let failure = world
        .get_resource::<SignInFailure>()
        .map(|failure| failure.0.clone());
    let waiting = world.get_resource::<Waiting>().map(|waiting| waiting.page);
    let game = world
        .resource::<Content>()
        .built::<GameSettings>()
        .name
        .clone();
    let mut name = std::mem::take(&mut world.resource_mut::<AccountName>().0);
    let submit = map::shortcut(world, InputAction::SubmitText);
    let choice = system_screen(&gui, "sign-in", Some(ink::VEIL), |ui| {
        panel(ui, &game, |ui| match (&sign_in, waiting) {
            (_, Some(page)) => browser_waits(ui, page),
            (SignIn::AnyAccount, None) => any_account(ui, &mut name, failure.as_deref(), submit),
            (SignIn::Browser(_), None) => provider(ui, failure.as_deref()),
            (SignIn::Token(_), None) => {
                if let Some(failure) = &failure {
                    ui.label(failure);
                }
                note(ui, "Sign in again where you started the game.");
                None
            }
        })
    })
    .inner;
    world.resource_mut::<AccountName>().0 = name;
    match choice {
        Some(Choice::Open(page)) => {
            if let SignIn::Browser(browser) = sign_in {
                let task = IoTaskPool::get().spawn(sign_in_in_browser(&browser, page));
                world.remove_resource::<SignInFailure>();
                world.insert_resource(Waiting { task, page });
            }
        }
        Some(Choice::Cancel) => {
            world.remove_resource::<Waiting>();
        }
        Some(Choice::Account(account)) => {
            signed_in(world, &fake_token(&account));
        }
        None => {}
    }
}

fn provider(ui: &mut egui::Ui, failure: Option<&str>) -> Option<Choice> {
    if let Some(failure) = failure {
        ui.label(failure);
    }
    if controls::primary(ui, "Sign in").clicked() {
        return Some(Choice::Open(AuthorizationPage::SignIn));
    }
    controls::button(ui, "Create account")
        .clicked()
        .then_some(Choice::Open(AuthorizationPage::SignUp))
}

fn browser_waits(ui: &mut egui::Ui, page: AuthorizationPage) -> Option<Choice> {
    ui.label(match page {
        AuthorizationPage::SignIn => "Continue in your browser to sign in.",
        AuthorizationPage::SignUp => "Continue in your browser to create your account.",
    });
    controls::button(ui, "Cancel")
        .clicked()
        .then_some(Choice::Cancel)
}

fn any_account(
    ui: &mut egui::Ui,
    name: &mut String,
    failure: Option<&str>,
    submit: Option<egui::KeyboardShortcut>,
) -> Option<Choice> {
    if let Some(failure) = failure {
        ui.label(failure);
    }
    let field = InputField::new(name)
        .hint("Account name")
        .return_key(submit)
        .show(ui);
    let entered = field.lost_focus()
        && submit.is_some_and(|submit| ui.input(|input| input.key_pressed(submit.logical_key)));
    if ui.memory(|memory| memory.focused().is_none()) {
        field.request_focus();
    }
    note(
        ui,
        "Any name signs in, and the same name again is the same account.",
    );
    let account = name.trim();
    let clicked = ui
        .add_enabled_ui(!account.is_empty(), |ui| controls::primary(ui, "Sign in"))
        .inner
        .clicked();
    ((clicked || entered) && !account.is_empty()).then(|| Choice::Account(account.to_owned()))
}

fn poll(world: &mut World) {
    let Some(mut waiting) = world.remove_resource::<Waiting>() else {
        return;
    };
    match block_on(future::poll_once(&mut waiting.task)) {
        Some(Ok(token)) => signed_in(world, &token),
        Some(Err(failure)) => world.insert_resource(SignInFailure(failure)),
        None => world.insert_resource(waiting),
    }
}

fn signed_in(world: &mut World, token: &str) {
    world.insert_resource(Credentials::from_access_token(token));
    world
        .resource_mut::<NextState<ClientScreen>>()
        .set(ClientScreen::Connecting);
}

fn forget(world: &mut World) {
    world.remove_resource::<Waiting>();
    world.remove_resource::<SignInFailure>();
}
