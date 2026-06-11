//! Drives the client's real browser sign-in (`auth::sign_in`) end-to-end against the running
//! stack: it captures the authorize URL the client would open, completes the Keycloak login in a
//! headless browser, and asserts the loopback callback yields a session with the expected roles.

use e2e_support::keycloak::Keycloak;
use e2e_support::{
    PASSWORD, REALM, auth_base, caddy_ca, chrome, ensure_assets, flow, unique, wait_for_file,
};

#[test]
fn sso_loopback_flow_signs_in() {
    ensure_assets();
    let keycloak = Keycloak::connect(&auth_base(), REALM, "admin", "admin");
    let user = unique("e2e-sso");
    keycloak.create_user(&user, PASSWORD, &[world::SPECTATE_ROLE]);

    // `sign_in` opens the authorize URL via the browser; capture it (instead of opening one) by
    // pointing $BROWSER at a script that records its argument, then drive Keycloak ourselves.
    let dir = std::env::temp_dir().join(&user);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let captured = dir.join("authorize-url");
    let opener = dir.join("open.sh");
    std::fs::write(
        &opener,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > {}\n", captured.display()),
    )
    .expect("write opener");
    std::process::Command::new("chmod")
        .args(["+x", opener.to_str().expect("opener path")])
        .status()
        .expect("chmod opener");

    // SAFETY: this test runs single-threaded (`--test-threads=1`) and the vars are set before any
    // thread that reads them starts.
    unsafe {
        std::env::set_var("BROWSER", &opener);
        std::env::set_var(
            "RIFT_CLIENT_ISSUER",
            format!("{}/realms/{REALM}", auth_base()),
        );
        std::env::set_var("RIFT_CLIENT_EXTRA_CA", caddy_ca());
    }

    let signing_in = std::thread::spawn(client::auth::sign_in);

    let url = wait_for_file(&captured, 30.0).expect("captured authorize url");
    let browser = chrome::Browser::launch();
    let page = browser.open(&url);
    flow::wait_for(&page, "document.getElementById('username') !== null");
    flow::fill(&page, "username", &user);
    flow::fill(&page, "password", PASSWORD);
    page.eval("document.getElementById('kc-login').click()");

    let session = signing_in
        .join()
        .expect("sign-in thread")
        .expect("sign-in succeeds");
    assert!(
        !session.access_token.is_empty(),
        "an access token is returned"
    );
    assert!(
        session
            .roles
            .iter()
            .any(|role| role == world::SPECTATE_ROLE),
        "the spectate role is read from the token"
    );
}
