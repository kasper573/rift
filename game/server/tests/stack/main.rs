//! The end-to-end suite: a headless client app connects to the running game server over the
//! encrypted UDP netcode transport and exercises the session through `world`'s public API. Runs
//! against the docker test stack, or a locally run server via `RIFT_E2E_GAME_SERVER`.

mod app;
mod session;

use e2e_support::keycloak::Keycloak;
use e2e_support::{PASSWORD, REALM, auth_base, chrome, ensure_assets, flow, site_base, unique};

#[test]
fn a_player_can_join_and_move() {
    ensure_assets();
    let token = session::mint("Bypass e2e-mover");
    let mut app = app::connect(&token);

    let welcomed = app::wait(&mut app, 20.0, |app| {
        world::session::my_id(app.world()).is_some()
    });
    assert!(welcomed, "the client should connect and be welcomed");

    world::session::join(app.world_mut());
    let spawned = app::wait(&mut app, 15.0, |app| {
        world::session::my_position(app.world()).is_some()
    });
    assert!(spawned, "the player should spawn after joining");

    let start = world::session::my_position(app.world()).expect("spawned position");
    world::session::move_to(app.world_mut(), start.x + 4.0, start.y);
    let moved = app::wait(&mut app, 15.0, |app| {
        world::session::my_position(app.world()).is_some_and(|pos| pos.distance_to(start) > 0.5)
    });
    assert!(moved, "moving should change the player's position");
}

#[test]
fn spectate_without_the_role_is_denied() {
    ensure_assets();
    // A bypass player carries an identity with no roles, so the server refuses to seat them as a
    // spectator (an authenticated account needs the spectate role; see `spectate::allowed`).
    let token = session::mint("Bypass e2e-watcher");
    let mut app = app::connect(&token);
    let welcomed = app::wait(&mut app, 20.0, |app| {
        world::session::my_id(app.world()).is_some()
    });
    assert!(welcomed, "the client should connect and be welcomed");

    world::session::spectate(app.world_mut(), None);
    // Give the server ample time to (not) seat the spectator.
    app::wait(&mut app, 5.0, |_| false);
    assert!(
        !world::session::is_spectating(app.world()),
        "a roleless player must not be seated as a spectator"
    );
}

#[test]
fn spectate_with_the_role_is_allowed() {
    ensure_assets();
    let keycloak = Keycloak::connect(&auth_base(), REALM, "admin", "admin");
    let user = unique("e2e-spectator");
    keycloak.create_user(&user, PASSWORD, &[world::SPECTATE_ROLE]);
    let jwt = keycloak.password_token(&user, PASSWORD);

    let token = session::mint(&format!("Bearer {jwt}"));
    let mut app = app::connect(&token);
    let welcomed = app::wait(&mut app, 20.0, |app| {
        world::session::my_id(app.world()).is_some()
    });
    assert!(welcomed, "the spectator should connect and be welcomed");

    world::session::spectate(app.world_mut(), None);
    let spectating = app::wait(&mut app, 15.0, |app| {
        world::session::is_spectating(app.world())
    });
    assert!(
        spectating,
        "a player holding the spectate role should be seated as a spectator"
    );
}

#[test]
fn website_serves_sign_in_and_download() {
    let keycloak = Keycloak::connect(&auth_base(), REALM, "admin", "admin");
    let user = unique("e2e-site");
    keycloak.create_user(&user, PASSWORD, &[]);

    let browser = chrome::Browser::launch();
    let home = browser.open(&format!("{}/", site_base()));
    flow::wait_for(&home, "document.querySelector('nav') !== null");
    flow::click_text(&home, "button", "Sign in");
    flow::wait_for(&home, "document.getElementById('username') !== null");
    flow::fill(&home, "username", &user);
    flow::fill(&home, "password", PASSWORD);
    home.eval("document.getElementById('kc-login').click()");
    flow::wait_for(
        &home,
        &format!("document.body.innerText.includes('{user}')"),
    );

    let download = browser.open(&format!("{}/download", site_base()));
    flow::wait_for(
        &download,
        "document.body.innerText.includes('Download Rift')",
    );
    flow::wait_for(
        &download,
        "[...document.querySelectorAll('a')].some(a => a.href.includes('releases/latest/download'))",
    );
}
