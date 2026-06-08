//! Needs the docker test stack and Chrome: run with `cargo x e2e`, which derives the stack
//! settings from the docker env files. `CHROME` overrides the browser binary.

mod cdp;
mod flow;
mod keycloak;
mod stage;

use e2e::scenarios;
use stage::BrowserStage;
use world::SPECTATE_ROLE;
use world::core::math::{Pos, Tiles};

macro_rules! bind {
    ($group:ident: $($name:ident $(($($arg:expr),*))?),* $(,)?) => {$(
        #[test]
        fn $name() {
            scenarios::$name(&mut BrowserStage::connect() $($(, $arg)*)?);
        }
    )*};
}

e2e::for_each_scenario!(bind);

#[test]
fn a_visitor_can_register_sign_in_and_play() {
    let stage = BrowserStage::connect();
    let page = stage.site_page("/");
    flow::wait_for(&page, "document.querySelector('nav') !== null");

    flow::click_text(&page, "button", "Sign in");
    flow::wait_for(&page, "location.host.startsWith('auth.')");
    flow::wait_for(
        &page,
        "document.querySelector('a[href*=registration]') !== null",
    );
    page.eval("document.querySelector('a[href*=registration]').click()");
    flow::wait_for(&page, "document.getElementById('username') !== null");
    let user = format!(
        "e2e-ui-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_nanos()
    );
    flow::fill(&page, "username", &user);
    flow::fill(&page, "email", &format!("{user}@example.test"));
    flow::fill(&page, "firstName", "E2e");
    flow::fill(&page, "lastName", "Tester");
    flow::fill(&page, "password", stage::PASSWORD);
    flow::fill(&page, "password-confirm", stage::PASSWORD);
    page.eval("document.querySelector('form').submit()");

    flow::wait_for(&page, "location.host.startsWith('mp.')");
    flow::wait_for(
        &page,
        &format!("document.body.innerText.includes('{user}')"),
    );

    assert_eq!(
        page.eval("[...document.querySelectorAll('nav a')].map(a => a.textContent).join(',')")
            .as_str()
            .map(|links| links.contains("Spectate")),
        Some(false),
        "an ordinary user must not see the spectate link",
    );

    flow::click_text(&page, "nav a", "Play");
    flow::wait_for(&page, "document.getElementById('glcanvas') !== null");
    let mut mirror = rift::Client::new();
    let spawned = flow::wait(60.0, || {
        for frame in page.received_frames() {
            mirror.receive(&frame);
        }
        e2e::view_of(&mirror).me().is_some()
    });
    assert!(
        spawned,
        "the embedded client must connect and spawn a player"
    );

    let start = {
        let view = e2e::view_of(&mirror);
        let me = view.me().expect("player visible").clone();
        me.pos
    };
    let (x, y) = flow::canvas_click_point(&page, 40.0, 0.0);
    page.click(x, y);
    let moved = flow::wait(15.0, || {
        for frame in page.received_frames() {
            mirror.receive(&frame);
        }
        e2e::view_of(&mirror)
            .me()
            .is_some_and(|me| me.pos.distance(start) > 0.5)
    });
    assert!(moved, "clicking the canvas must move the player");
}

#[test]
fn a_spectator_can_sign_in_and_spectate_via_the_ui() {
    let mut stage = BrowserStage::connect();
    let user = stage.user(&[SPECTATE_ROLE]);
    let page = stage.signed_in_page(&user);

    flow::wait_for(
        &page,
        "[...document.querySelectorAll('nav a')].some(a => a.textContent === 'Spectate')",
    );
    flow::click_text(&page, "nav a", "Spectate");
    flow::wait_for(&page, "document.getElementById('glcanvas') !== null");

    let mut mirror = rift::Client::new();
    let spectating = flow::wait(60.0, || {
        for frame in page.received_frames() {
            mirror.receive(&frame);
        }
        e2e::view_of(&mirror).spectating
    });
    assert!(spectating, "the spectate page must boot a spectator client");
}
