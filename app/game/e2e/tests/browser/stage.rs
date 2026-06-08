//! No test hooks: every [`e2e::Player`] is the wasm client on the production website, signed in
//! through the real flow, driven by real input, and observed by decoding its captured WebSocket
//! traffic with the public `rift::Client`.

use std::time::Duration;

use client::render::{INV_GRID, INV_PAD, INV_SLOT, VIEW_TILES};
use e2e::{Player, Stage, View, view_of};
use world::SPECTATE_ROLE;
use world::core::area;
use world::core::math::{Pixels, Pos, Size, Tiles};

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

    fn enter_game(&mut self, roles: &[&str], link: &str) -> BrowserPlayer {
        let username = self.user(roles);
        let page = self.signed_in_page(&username);
        flow::wait_for(
            &page,
            &format!(
                "[...document.querySelectorAll('nav a')].some(a => a.textContent === '{link}')"
            ),
        );
        flow::click_text(&page, "nav a", link);
        let mut player = BrowserPlayer::new(page);
        assert!(
            flow::wait(60.0, || player.view().open),
            "the game client never connected"
        );
        player
            .page
            .eval("document.getElementById('glcanvas').focus()");
        player
    }
}

impl Stage for BrowserStage {
    fn player(&mut self) -> Box<dyn Player> {
        let mut player = self.enter_game(&[], "Play");
        assert!(
            flow::wait(30.0, || player.view().me().is_some()),
            "a joining player must spawn"
        );
        Box::new(player)
    }
    fn spectator(&mut self) -> Box<dyn Player> {
        let mut player = self.enter_game(&[SPECTATE_ROLE], "Spectate");
        assert!(
            flow::wait(30.0, || player.view().spectating),
            "an entitled spectator must be admitted"
        );
        Box::new(player)
    }
    fn unentitled_spectator(&mut self) -> Box<dyn Player> {
        let username = self.user(&[]);
        let page = self.signed_in_page(&username);
        page.eval("location.assign('/spectate')");
        flow::wait_for(&page, "document.body.innerText.includes('permission')");
        Box::new(BrowserPlayer::new(page))
    }
    fn step(&mut self, seconds: f32) {
        std::thread::sleep(Duration::from_secs_f32(seconds));
    }
}

pub struct BrowserPlayer {
    page: Page,
    mirror: rift::Client,
    canvas: Option<(f64, f64, f64, f64)>,
}

impl BrowserPlayer {
    fn new(page: Page) -> BrowserPlayer {
        BrowserPlayer {
            page,
            mirror: rift::Client::new(),
            canvas: None,
        }
    }

    fn pump(&mut self) {
        for frame in self.page.received_frames() {
            self.mirror.receive(&frame);
        }
    }

    fn canvas_rect(&mut self) -> (f64, f64, f64, f64) {
        *self
            .canvas
            .get_or_insert_with(|| flow::canvas_rect(&self.page))
    }

    // Aim at tile centers: the camera (following the moving player) runs a frame or two ahead
    // of the mirror, and a center hit stays on the intended tile despite the skew.
    fn click_world(&mut self, x: f32, y: f32) {
        let (x, y) = (x.floor() + 0.5, y.floor() + 0.5);
        let Some(me) = view_of(&self.mirror).me().cloned() else {
            return;
        };
        let area = me.area.unwrap_or(world::core::area::AreaId(0));
        let Some(camera) = client::render::camera_for(me.pos, area) else {
            return;
        };
        let (left, top, width, height) = self.canvas_rect();
        let (scale, offset_x, offset_y) = client::render::letterbox(width as f32, height as f32);
        let frame = client::render::to_frame_f(camera, Pos::new(Tiles(x), Tiles(y)));
        self.page.click(
            left + f64::from(offset_x + frame.x.0 * scale),
            top + f64::from(offset_y + frame.y.0 * scale),
        );
    }

    fn waypoint_toward(&mut self, target: Pos<Tiles>) -> Option<Pos<Tiles>> {
        let view = view_of(&self.mirror);
        let me = view.me()?;
        let half_w = VIEW_TILES.x.0 / 2.0 - 2.0;
        let half_h = VIEW_TILES.y.0 / 2.0 - 2.0;
        let area = me.area.unwrap_or(world::core::area::AreaId(0));
        let camera = client::render::camera_for(me.pos, area)?;
        let center = camera.center;
        let clamped = Pos::new(
            Tiles(target.x.0.clamp(center.x.0 - half_w, center.x.0 + half_w)),
            Tiles(target.y.0.clamp(center.y.0 - half_h, center.y.0 + half_h)),
        );
        if clamped == target {
            return Some(target);
        }
        // A waypoint resting inside a portal would carry an unintended portal intent.
        let area = me.area.unwrap_or(world::core::area::AreaId(0)).0 as usize;
        let portals = &area::areas().get(area)?.portals;
        let mut waypoint = clamped;
        if portals.iter().any(|portal| portal.rect.contains(waypoint)) {
            waypoint.y = Tiles((waypoint.y.0 - 3.0).max(center.y.0 - half_h));
        }
        Some(waypoint)
    }
}

impl Player for BrowserPlayer {
    fn view(&mut self) -> View {
        self.pump();
        view_of(&self.mirror)
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.pump();
        if let Some(waypoint) = self.waypoint_toward(Pos::new(Tiles(x), Tiles(y))) {
            self.click_world(waypoint.x.0, waypoint.y.0);
        }
    }

    fn attack(&mut self, entity: u32) {
        self.pump();
        let target = view_of(&self.mirror)
            .actors
            .iter()
            .find(|seen| seen.entity == entity)
            .map(|seen| seen.pos);
        let Some(at) = target else {
            return;
        };
        match self.waypoint_toward(at) {
            Some(waypoint) if waypoint == at => self.click_world(at.x.0, at.y.0),
            Some(waypoint) => self.click_world(waypoint.x.0, waypoint.y.0),
            None => {}
        }
    }

    fn respawn(&mut self) {
        self.page.press_key("KeyR", "r");
    }

    fn watch(&mut self, owner: u32) {
        // The client's spectate UI cycles watch targets with N; press until the wire confirms.
        for _ in 0..20 {
            self.pump();
            if view_of(&self.mirror).watching == Some(owner) {
                return;
            }
            self.page.press_key("KeyN", "n");
            std::thread::sleep(Duration::from_millis(300));
        }
        panic!("spectating never locked onto player {owner}");
    }

    // Double-click the slot's center in the inventory grid, whose layout the client derives
    // from the same client::render constants (slot in the first, unscrolled viewport).
    fn use_item(&mut self, slot: u32) {
        let (left, top, width, _) = self.canvas_rect();
        let (row, col) = (slot / INV_GRID.x, slot % INV_GRID.x);
        let grid_w = INV_GRID.x as f32 * INV_SLOT;
        // The grid is anchored top-right of the canvas; `center` is the slot center in canvas pixels.
        let origin = Pos::new(Pixels(width as f32 - INV_PAD - grid_w), Pixels(INV_PAD));
        let center = origin
            + Size::new(col, row).convert::<Pixels>(|slots| slots * INV_SLOT)
            + Size::splat(Pixels(INV_SLOT / 2.0));
        let (x, y) = (left + f64::from(center.x.0), top + f64::from(center.y.0));
        self.page.click(x, y);
        std::thread::sleep(Duration::from_millis(120));
        self.page.click(x, y);
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("{name} must be set — run the browser suite via `cargo x e2e`"))
}
