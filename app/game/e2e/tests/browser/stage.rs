//! Connects the browser suite to the live stack: drives Keycloak (admin sign-in and user
//! creation) and opens pages against the deployed website. Each test owns its own `rift::Client`
//! mirror to decode the captured WebSocket traffic.

use crate::cdp::{Browser, Page};
use crate::flow;
use crate::keycloak::Keycloak;

pub const PASSWORD: &str = "e2e-password-1";

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
