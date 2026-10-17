use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::widgets;

use super::connection::OVERLAY_BG;
use super::{ClientScreen, note, pace_as_before, pace_slowly, panel, system_screen};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::net::auth::Credentials;
use crate::core::platform::{ProviderPage, SignIn, StartParams};
use crate::systems::GameSettings;
use crate::systems::launch::fake_token;

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
    page: ProviderPage,
}

enum Choice {
    Open(ProviderPage),
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
    let choice = system_screen(&gui, "sign-in", Some(OVERLAY_BG), |ui| {
        panel(ui, &game, |ui| match (&sign_in, waiting) {
            (_, Some(page)) => browser_waits(ui, page),
            (SignIn::AnyAccount, None) => any_account(ui, &mut name, failure.as_deref()),
            (SignIn::Provider(_), None) => provider(ui, failure.as_deref()),
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
            if let SignIn::Provider(provider) = sign_in {
                let task = IoTaskPool::get().spawn_local(provider.sign_in(page));
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
    if widgets::primary(ui, "Sign in").clicked() {
        return Some(Choice::Open(ProviderPage::SignIn));
    }
    ui.button("Create account")
        .clicked()
        .then_some(Choice::Open(ProviderPage::CreateAccount))
}

fn browser_waits(ui: &mut egui::Ui, page: ProviderPage) -> Option<Choice> {
    ui.label(match page {
        ProviderPage::SignIn => "Continue in your browser to sign in.",
        ProviderPage::CreateAccount => "Continue in your browser to create your account.",
    });
    ui.button("Cancel").clicked().then_some(Choice::Cancel)
}

fn any_account(ui: &mut egui::Ui, name: &mut String, failure: Option<&str>) -> Option<Choice> {
    if let Some(failure) = failure {
        ui.label(failure);
    }
    let field = ui.add(egui::TextEdit::singleline(name).hint_text("Account name"));
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if ui.memory(|memory| memory.focused().is_none()) {
        field.request_focus();
    }
    note(
        ui,
        "Any name signs in, and the same name again is the same account.",
    );
    let account = name.trim();
    let clicked = ui
        .add_enabled_ui(!account.is_empty(), |ui| widgets::primary(ui, "Sign in"))
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
