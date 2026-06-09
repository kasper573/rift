use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio, exit};
use std::time::Duration;
use std::{env, fs, thread};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    env::set_current_dir(root).expect("enter workspace root");
    let args: Vec<String> = env::args().skip(1).collect();
    match args.split_first().map(|(cmd, rest)| (cmd.as_str(), rest)) {
        Some(("check", [])) => check(),
        Some(("wasm", [])) => wasm(),
        Some(("package", [])) => package(),
        Some(("images", [])) => images(),
        Some(("prewarm", [])) => prewarm(),
        Some(("dev", [])) => dev(),
        Some(("dev-serve", [])) => dev_serve(),
        Some(("e2e", args)) => e2e(args),
        Some(("up", [env_name, extra @ ..])) => up(env_name, extra),
        Some(("down", [env_name])) => compose(env_name, &["down".to_owned()]),
        Some(("compose", [env_name, rest @ ..])) => compose(env_name, rest),
        Some(("logs", [dir])) => logs(dir),
        _ => usage(),
    }
}

fn usage() {
    eprintln!(
        "usage: cargo x <command>

  check                    formatting and lints
  wasm                     the browser build of the game client, staged into target/game-client/
  package                  compile the release artifacts (wasm client + native binaries) and stage
                           them into docker/stage/ for the website and game-server images
  images                   build the docker images; with INFRA_TAG set, the rarely-changing
                           keycloak/reverse-proxy images are pulled by that tag (built+pushed on miss)
  prewarm                  pull the infra images and start keycloak early so the e2e suite does
                           not wait on its boot
  dev                      the dev environment: compose stack + website + game server,
                           rebuilding and restarting them as sources change
  e2e [--no-build] [filter]
                           the browser e2e game suite against the docker test stack (brings it up);
                           --no-build reuses already-built images instead of rebuilding
  up <env> [args...]       bring up the <env> compose stack (dev|test|prod)
  down <env>               tear down the <env> compose stack
  compose <env> <args...>  run docker compose with the <env> profile and env files
  logs <dir>               write each container's logs into <dir>, one file per container"
    );
    exit(1);
}

fn check() {
    run(Command::new("cargo").args(["fmt", "--check"]));
    // The client ships as wasm, so it is linted for that target; the rest of the workspace is
    // linted natively, which keeps the client's native-only deps (ALSA) out of CI.
    run(Command::new("cargo").args([
        "clippy",
        "--workspace",
        "--exclude",
        "client",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ]));
    run(Command::new("cargo").args([
        "clippy",
        "-p",
        "client",
        "--target",
        "wasm32-unknown-unknown",
        "--",
        "-D",
        "warnings",
    ]));
}

// The dev loop's wasm build gets its own target dir so it can run concurrently with the
// native build: cargo holds one lock per target dir.
const DEV_WASM_TARGET_DIR: &str = "target/wasm-dev";

fn wasm() {
    run(Command::new("cargo").arg("wasm"));
    stage_game_client("target/wasm32-unknown-unknown/release/client.wasm");
}

// Built for the host target (glibc) rather than cross-compiled to musl: the rust cache persists
// the host target's dependencies across CI runs reliably, the musl one it does not.
fn package() {
    wasm();
    run(Command::new("cargo").args(["build", "--release", "-p", "website", "-p", "server"]));
    let stage = Path::new("docker/stage");
    fs::create_dir_all(stage.join("game"))
        .unwrap_or_else(|error| die(&format!("{}: {error}", stage.display())));
    let release = "target/release";
    copy(format!("{release}/website"), &stage.join("website"));
    copy(format!("{release}/server"), &stage.join("game-server"));
    copy(
        "target/game-client/client.wasm",
        &stage.join("game/client.wasm"),
    );
    copy(
        "target/game-client/mq_js_bundle.js",
        &stage.join("game/mq_js_bundle.js"),
    );
}

fn dev() {
    up("dev", &[]);
    // watchexec owns the watch loop: each change re-runs serve, and cargo's own change
    // tracking decides what actually rebuilds. It honors .gitignore, so target/ is excluded.
    run(Command::new("watchexec").args([
        "--restart",
        "--watch",
        "app",
        "--watch",
        "lib",
        "--",
        "cargo",
        "x",
        "dev-serve",
    ]));
}

fn dev_serve() {
    let wasm = spawn(
        Command::new("cargo")
            .args(["build", "--profile", "dev-serve", "-p", "client"])
            .args(["--target", "wasm32-unknown-unknown"])
            .env("CARGO_TARGET_DIR", DEV_WASM_TARGET_DIR),
    );
    let native = spawn(Command::new("cargo").args([
        "build",
        "--profile",
        "dev-serve",
        "-p",
        "server",
        "-p",
        "website",
    ]));
    join(vec![wasm, native]);
    stage_game_client(&format!(
        "{DEV_WASM_TARGET_DIR}/wasm32-unknown-unknown/dev-serve/client.wasm"
    ));
    let vars = resolve_env("dev");
    let website = spawn(Command::new("target/dev-serve/website").envs(vars.clone()));
    let server = spawn(Command::new("target/dev-serve/server").envs(vars));
    wait_first(vec![website, server]);
}

fn join(children: Vec<Child>) {
    for mut child in children {
        let status = child
            .wait()
            .unwrap_or_else(|error| die(&format!("build: {error}")));
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
    }
}

const E2E_SERVICES: &[&str] = &[
    "reverse-proxy",
    "rift-website",
    "rift-game-server",
    "keycloak",
    "keycloak-healthcheck",
    "postgres",
    "postfix",
];

// A local run packages and rebuilds so it tests fresh code; CI passes --no-build to reuse the
// images its build stage already produced.
fn e2e(args: &[String]) {
    let (build, filter) = match args.split_first() {
        Some((flag, rest)) if flag == "--no-build" => (false, rest),
        _ => (true, args),
    };
    if build {
        package();
    }
    let build_flag = if build { "--build" } else { "--no-build" };
    // Re-pin the proxy to this env: compose does not recreate on env-file value changes.
    compose_str(
        "test",
        &["up", "-d", "--force-recreate", build_flag, "reverse-proxy"],
    );
    // The browser suite only exercises the app, auth, and proxy — bringing up the observability
    // stack would just cost CI a pile of image pulls and startups for nothing.
    let mut up_args = vec!["up", "-d", "--wait", build_flag];
    up_args.extend_from_slice(E2E_SERVICES);
    compose_str("test", &up_args);
    let mut test_args = vec!["--test", "browser"];
    test_args.extend(filter.iter().map(String::as_str));
    test_args.extend(["--", "--test-threads=1"]);
    stack_test(&test_args);
}

fn up(env_name: &str, extra: &[String]) {
    // Compose does not recreate on env-file value changes, so re-pin the proxy to this env.
    let mut recreate = vec!["up", "-d", "--force-recreate"];
    recreate.extend(extra.iter().map(String::as_str));
    recreate.push("reverse-proxy");
    compose_str(env_name, &recreate);
    let mut all = vec!["up", "-d", "--wait"];
    all.extend(extra.iter().map(String::as_str));
    compose_str(env_name, &all);
}

fn compose(env_name: &str, args: &[String]) {
    compose_str(
        env_name,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    );
}

fn images() {
    pull_infra();
    compose_str("prod", &["build", "rift-website", "rift-game-server"]);
}

// keycloak boots slowly (~30s); starting it now overlaps that with the rust build steps so the
// e2e suite finds it already healthy instead of waiting.
fn prewarm() {
    pull_infra();
    compose_str(
        "test",
        &[
            "up",
            "-d",
            "--no-build",
            "postgres",
            "keycloak",
            "keycloak-healthcheck",
        ],
    );
}

// keycloak and reverse-proxy depend only on their own Dockerfiles, so CI tags them by a hash of
// those (INFRA_TAG) and reuses the pushed image across runs instead of rebuilding kc.sh/xcaddy.
fn pull_infra() {
    match env::var("INFRA_TAG").ok().filter(|tag| !tag.is_empty()) {
        Some(tag) => {
            let registry = env::var("DOCKER_REGISTRY_URL").unwrap_or_else(|_| "rift".to_owned());
            let version = env::var("DOCKER_IMAGE_VERSION").unwrap_or_else(|_| "latest".to_owned());
            for (service, image) in [
                ("keycloak", "rift-keycloak"),
                ("reverse-proxy", "rift-reverse-proxy"),
            ] {
                let cached = format!("{registry}/{image}:{tag}");
                let current = format!("{registry}/{image}:{version}");
                if try_run(Command::new("docker").args(["pull", cached.as_str()])) {
                    run(Command::new("docker").args(["tag", cached.as_str(), current.as_str()]));
                } else {
                    compose_str("prod", &["build", service]);
                    run(Command::new("docker").args(["tag", current.as_str(), cached.as_str()]));
                    run(Command::new("docker").args(["push", cached.as_str()]));
                }
            }
        }
        None => compose_str("prod", &["build", "keycloak", "reverse-proxy"]),
    }
}

fn logs(dir: &str) {
    let dir = Path::new(dir);
    fs::create_dir_all(dir).unwrap_or_else(|error| die(&format!("{}: {error}", dir.display())));
    let ids = capture(Command::new("docker").args(["ps", "-aq"]));
    for id in ids.split_whitespace() {
        let name = capture(Command::new("docker").args(["inspect", "--format", "{{.Name}}", id]));
        let path = dir.join(format!("{}.log", name.trim().trim_start_matches('/')));
        let out = fs::File::create(&path)
            .unwrap_or_else(|error| die(&format!("{}: {error}", path.display())));
        let err = out
            .try_clone()
            .unwrap_or_else(|error| die(&format!("{}: {error}", path.display())));
        let _ = Command::new("docker")
            .args(["logs", id])
            .stdout(out)
            .stderr(err)
            .status();
    }
}

fn compose_str(env_name: &str, args: &[&str]) {
    ensure_network();
    let mut command = Command::new("docker");
    command.args([
        "compose",
        "-f",
        "docker/docker-compose.yaml",
        "--profile",
        env_name,
    ]);
    if Path::new("docker/.env").exists() {
        command.args(["--env-file", "docker/.env"]);
    }
    command
        .arg("--env-file")
        .arg(format!("docker/.env.{env_name}"));
    command.args(["--env-file", "docker/.env.shared"]);
    command.args(args);
    command.env("COMPOSE_ENV", env_name);
    run(&mut command);
}

fn ensure_network() {
    let exists = Command::new("docker")
        .args(["network", "inspect", "rift"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !exists {
        run(Command::new("docker").args(["network", "create", "rift"]));
    }
}

fn stack_test(args: &[&str]) {
    let vars = resolve_env("test");
    run(Command::new("cargo")
        .args(["test", "-p", "e2e", "--features", "stack"])
        .args(args)
        .envs(vars));
}

// The miniquad JS loader ships inside the macroquad crate version pinned by Cargo.lock.
fn stage_game_client(wasm_artifact: &str) {
    let lock = fs::read_to_string("Cargo.lock").expect("Cargo.lock");
    let mut lines = lock.lines();
    let version = loop {
        match lines.next() {
            Some("name = \"macroquad\"") => {
                let line = lines.next().unwrap_or_default();
                break line.split('"').nth(1).unwrap_or_default().to_owned();
            }
            Some(_) => {}
            None => die("macroquad not found in Cargo.lock"),
        }
    };
    let cargo_home = env::var_os("CARGO_HOME").map_or_else(
        || PathBuf::from(env::var_os("HOME").expect("HOME")).join(".cargo"),
        PathBuf::from,
    );
    let sources = cargo_home.join("registry/src");
    let bundle = fs::read_dir(&sources)
        .unwrap_or_else(|error| die(&format!("{}: {error}", sources.display())))
        .filter_map(Result::ok)
        .map(|entry| {
            entry
                .path()
                .join(format!("macroquad-{version}/js/mq_js_bundle.js"))
        })
        .find(|path| path.exists())
        .unwrap_or_else(|| {
            die(&format!(
                "mq_js_bundle.js for macroquad {version} not in the cargo registry"
            ))
        });
    fs::create_dir_all("target/game-client").expect("create target/game-client");
    copy(wasm_artifact, Path::new("target/game-client/client.wasm"));
    // The bundle's own net plugin is broken (minification stripped its helper globals), so the
    // client's working replacement rides along in the same file.
    let mut runtime = fs::read_to_string(&bundle)
        .unwrap_or_else(|error| die(&format!("{}: {error}", bundle.display())));
    runtime.push('\n');
    runtime.push_str(
        &fs::read_to_string("app/game/client/js/rift_ws.js").expect("the rift_ws plugin"),
    );
    runtime.push('\n');
    runtime.push_str(
        &fs::read_to_string("app/game/client/js/rift_audio.js").expect("the rift_audio plugin"),
    );
    fs::write("target/game-client/mq_js_bundle.js", runtime).expect("stage mq_js_bundle.js");
}

// Mirrors the compose env-file layering: later files override, and a ${KEY:-default}
// self-reference resolves to the earlier layer's value, or its default.
fn resolve_env(env_name: &str) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = Vec::new();
    for (file, required) in [
        (".env".to_owned(), false),
        (format!(".env.{env_name}"), true),
        (".env.shared".to_owned(), true),
    ] {
        let path = Path::new("docker").join(&file);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) if !required => continue,
            Err(error) => die(&format!("{}: {error}", path.display())),
        };
        for line in text.lines() {
            let Some((key, raw)) = line.trim().split_once('=') else {
                continue;
            };
            if key.is_empty() || key.starts_with('#') {
                continue;
            }
            let value = interpolate(unquote(raw), &vars);
            match vars.iter_mut().find(|(name, _)| name == key) {
                Some(entry) => entry.1 = value,
                None => vars.push((key.to_owned(), value)),
            }
        }
    }
    vars
}

fn unquote(raw: &str) -> &str {
    let raw = match raw.split_once(" #") {
        Some((value, _)) => value,
        None => raw,
    }
    .trim();
    raw.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(raw)
}

fn interpolate(raw: &str, vars: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let reference = &rest[start + 2..start + end];
        let (name, fallback) = match reference.split_once(":-") {
            Some((name, fallback)) => (name, fallback),
            None => (reference, ""),
        };
        let value = vars
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback);
        out.push_str(value);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|error| die(&format!("{command:?}: {error}")));
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn try_run(command: &mut Command) -> bool {
    command
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn capture(command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| die(&format!("{command:?}: {error}")));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn spawn(command: &mut Command) -> Child {
    command
        .spawn()
        .unwrap_or_else(|error| die(&format!("{command:?}: {error}")))
}

// When one child exits, signal our whole process group so every descendant
// (cargo, the website, the server) stops with us.
fn wait_first(mut children: Vec<Child>) {
    loop {
        for child in &mut children {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!("a dev process exited ({status}); stopping the rest");
                let _ = Command::new("kill").args(["-TERM", "0"]).status();
                exit(status.code().unwrap_or(1));
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn copy(from: impl AsRef<Path>, to: &Path) {
    fs::copy(from.as_ref(), to).unwrap_or_else(|error| {
        die(&format!(
            "copy {} -> {}: {error}",
            from.as_ref().display(),
            to.display()
        ))
    });
}

fn die(message: &str) -> ! {
    eprintln!("{message}");
    exit(1);
}
