//! Connects the browser suite to the live stack: drives Keycloak (admin sign-in and user
//! creation), opens pages against the deployed website, and decodes captured WebSocket traffic
//! through a [`Mirror`] session.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::cdp::{Browser, Page};
use crate::flow;
use crate::keycloak::Keycloak;

pub const PASSWORD: &str = "e2e-password-1";

/// A read-only replica of what the browser's client received: the captured server frames played
/// into a [`world::MmoClient`] over a queue transport.
pub struct Mirror {
    pub client: world::MmoClient,
    queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl Mirror {
    pub fn new() -> Mirror {
        let queue = Rc::new(RefCell::new(VecDeque::new()));
        Mirror {
            client: world::MmoClient::with_transport(Box::new(Replay(Rc::clone(&queue)))),
            queue,
        }
    }

    pub fn feed(&mut self, frames: Vec<Vec<u8>>) {
        self.queue.borrow_mut().extend(frames);
        self.client.poll();
    }
}

struct Replay(Rc<RefCell<VecDeque<Vec<u8>>>>);

impl world::Transport for Replay {
    fn send(&mut self, _packet: &[u8]) {}

    fn poll(&mut self, sink: &mut dyn FnMut(&[u8])) {
        while let Some(frame) = self.0.borrow_mut().pop_front() {
            sink(&frame);
        }
    }

    fn status(&self) -> world::LinkStatus {
        world::LinkStatus::Open
    }
}

pub struct BrowserStage {
    browser: Browser,
    keycloak: Keycloak,
    site: String,
    run: u128,
    users: u32,
}

impl BrowserStage {
    pub fn connect() -> BrowserStage {
        let domain = required_env("RIFT_DOMAIN");
        let site = format!("https://{domain}");
        let auth = format!("https://auth.{domain}");
        crate::cdp::trace("keycloak admin sign-in");
        let keycloak = Keycloak::connect(
            &auth,
            &required_env("RIFT_AUTH__AUDIENCE"),
            &required_env("KC_BOOTSTRAP_ADMIN_USERNAME"),
            &required_env("KC_BOOTSTRAP_ADMIN_PASSWORD"),
        );
        crate::cdp::trace("keycloak ready");
        BrowserStage {
            browser: Browser::launch(),
            keycloak,
            site,
            run: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("epoch")
                .as_nanos(),
            users: 0,
        }
    }

    pub fn user(&mut self, roles: &[&str]) -> String {
        self.users += 1;
        let username = format!("e2e-{}-{}", self.run, self.users);
        crate::cdp::trace(&format!("create user {username} roles {roles:?}"));
        self.keycloak.create_user(&username, PASSWORD, roles);
        username
    }

    pub fn signed_in_page(&self, username: &str) -> Page {
        let page = self.site_page("/");
        flow::wait_for(&page, "document.querySelector('nav') !== null");
        flow::click_text(&page, "button", "Sign in");
        flow::wait_for(&page, "document.getElementById('username') !== null");
        flow::fill(&page, "username", username);
        flow::fill(&page, "password", PASSWORD);
        page.eval("document.getElementById('kc-login').click()");
        flow::wait_for(&page, "location.host.startsWith('rift.')");
        flow::wait_for(
            &page,
            &format!("document.body.innerText.includes('{username}')"),
        );
        page
    }

    pub fn site_page(&self, path: &str) -> Page {
        Page::open(&self.browser.cdp, &format!("{}{path}", self.site))
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("{name} must be set — run the browser suite via `cargo x e2e`"))
}
