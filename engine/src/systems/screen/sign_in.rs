use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task};
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::input::InputField;
use ui::{controls, ink};

use super::{
    ClientScreen, ScreenMessages, frame_as_before, frame_slowly, note, panel, system_screen,
};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::net::auth::Credentials;
use crate::core::os::{SignIn, StartParams, sign_in_in_browser};
use crate::systems::GameSettings;
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::interface::InterfaceMessages;
use crate::systems::launch::fake_token;
use crate::systems::messages;
use oidc::AuthorizationPage;

pub struct SignInPlugin;

impl Plugin for SignInPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AccountName>()
            .add_systems(OnEnter(ClientScreen::SigningIn), frame_slowly)
            .add_systems(OnExit(ClientScreen::SigningIn), (forget, frame_as_before))
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
    let content = world.resource::<Content>().clone();
    let settings = content.built::<GameSettings>();
    let messages = messages::of(&content);
    let words = &messages.screen;
    let game = &settings.name;
    let mut name = std::mem::take(&mut world.resource_mut::<AccountName>().0);
    let submit = world
        .resource::<InputMap>()
        .shortcut(InputAction::SubmitText);
    let choice = system_screen(&gui, "sign-in", Some(ink::VEIL), |ui| {
        panel(ui, game, |ui| match (&sign_in, waiting) {
            (_, Some(page)) => browser_waits(ui, page, words, &messages.interface),
            (SignIn::AnyAccount, None) => {
                any_account(ui, &mut name, failure.as_deref(), submit, words)
            }
            (SignIn::Browser(_), None) => provider(ui, failure.as_deref(), words),
            (SignIn::Token(_), None) => {
                if let Some(failure) = &failure {
                    ui.label(failure);
                }
                note(ui, words.sign_in_elsewhere());
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

fn provider(ui: &mut egui::Ui, failure: Option<&str>, words: &ScreenMessages) -> Option<Choice> {
    if let Some(failure) = failure {
        ui.label(failure);
    }
    if controls::primary(ui, words.sign_in()).clicked() {
        return Some(Choice::Open(AuthorizationPage::SignIn));
    }
    controls::button(ui, words.create_account())
        .clicked()
        .then_some(Choice::Open(AuthorizationPage::SignUp))
}

fn browser_waits(
    ui: &mut egui::Ui,
    page: AuthorizationPage,
    words: &ScreenMessages,
    interface: &InterfaceMessages,
) -> Option<Choice> {
    ui.label(match page {
        AuthorizationPage::SignIn => words.continue_to_sign_in(),
        AuthorizationPage::SignUp => words.continue_to_sign_up(),
    });
    controls::button(ui, interface.cancel())
        .clicked()
        .then_some(Choice::Cancel)
}

fn any_account(
    ui: &mut egui::Ui,
    name: &mut String,
    failure: Option<&str>,
    submit: Option<egui::KeyboardShortcut>,
    words: &ScreenMessages,
) -> Option<Choice> {
    if let Some(failure) = failure {
        ui.label(failure);
    }
    let field = InputField::new(name)
        .hint(words.account_name())
        .return_key(submit)
        .show(ui);
    let entered = field.lost_focus()
        && submit.is_some_and(|submit| ui.input(|input| input.key_pressed(submit.logical_key)));
    if ui.memory(|memory| memory.focused().is_none()) {
        field.request_focus();
    }
    note(ui, words.any_account());
    let account = name.trim();
    let clicked = ui
        .add_enabled_ui(!account.is_empty(), |ui| {
            controls::primary(ui, words.sign_in())
        })
        .inner
        .clicked();
    ((clicked || entered) && !account.is_empty()).then(|| Choice::Account(account.to_owned()))
}

fn poll(world: &mut World) {
    let Some((_, outcome)) = super::finished::<Waiting, _>(world, |waiting| &mut waiting.task)
    else {
        return;
    };
    match outcome {
        Ok(token) => signed_in(world, &token),
        Err(failure) => world.insert_resource(SignInFailure(failure)),
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
