//! End-to-end smoke test of the real client: it builds and runs the actual `rift` binary (the
//! full Bevy app — windowing, rendering, netcode, HUD) against a freshly spawned server, and
//! asserts the client connects, spawns its player, renders it, and exits cleanly. A headless CI
//! runner provides a display via `xvfb-run`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

#[test]
fn the_client_connects_spawns_and_renders() {
    let client = env!("CARGO_BIN_EXE_rift");
    let server = binary(client, "server");
    // A unique port per test process keeps reruns from colliding on a lingering socket.
    let port = 30000 + (std::process::id() % 20000) as u16;
    let game_url = format!("http://127.0.0.1:{port}");

    let mut server = Command::new(&server)
        .env("RIFT_ASSETS", ASSETS)
        .env("RIFT_GAME_SERVER_AUTH_BYPASS", "true")
        .env("RIFT_GAME_SERVER_PORT", port.to_string())
        .env_remove("RIFT_AUTH_ISSUER")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");

    let healthy = wait_for_health(&game_url, Duration::from_secs(20));
    if !healthy {
        let _ = server.kill();
        panic!("server never became healthy");
    }

    let outcome = run_client(client, &game_url);
    let _ = server.kill();
    let _ = server.wait();
    assert!(
        outcome,
        "the real client must connect, join, spawn, and render its player (then exit 0)"
    );
}

/// Runs the real client in smoke mode, under `xvfb-run` when there is no display, and reports
/// whether it exited successfully within the deadline.
fn run_client(client: &str, game_url: &str) -> bool {
    let headless =
        std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none();
    let mut command = if headless {
        let mut command = Command::new("xvfb-run");
        command.args(["-a", client]);
        command
    } else {
        Command::new(client)
    };
    let mut child = command
        .env("RIFT_CLIENT_SMOKE", "1")
        .env("RIFT_CLIENT_GAME_URL", game_url)
        .env("RIFT_ASSETS", ASSETS)
        .env_remove("RIFT_CLIENT_EXTRA_CA")
        .env_remove("RIFT_CLIENT_ISSUER")
        .spawn()
        .expect("spawn client");

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll client") {
            Some(status) => return status.success(),
            None if Instant::now() > deadline => {
                let _ = child.kill();
                return false;
            }
            None => sleep(Duration::from_millis(200)),
        }
    }
}

/// The sibling binary `name` in the client binary's target directory, built if missing.
fn binary(client: &str, name: &str) -> PathBuf {
    let path = Path::new(client).parent().expect("target dir").join(name);
    if !path.exists() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", name])
            .status()
            .expect("build sibling binary");
        assert!(status.success(), "failed to build {name}");
    }
    path
}

fn wait_for_health(base: &str, timeout: Duration) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(mut response) = agent.get(format!("{base}/health")).call()
            && response
                .body_mut()
                .read_to_string()
                .is_ok_and(|body| body == "ok")
        {
            return true;
        }
        sleep(Duration::from_millis(300));
    }
    false
}
