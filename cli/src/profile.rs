use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use engine::core::time::UtcHour;
use engine::systems::launch::{AuthMode, ServerConfig};
use engine::systems::player::SpawnPolicy;

use crate::binaries::{self, GAME_SERVER_PROFILING};
use crate::loadtest;
use crate::server::{GameServer, LOCALHOST, absolute};

const SAMPLED: Duration = Duration::from_secs(20);
const SAMPLE_HZ: &str = "999";
const SEED: u64 = 1931;
const MAX_CLIENTS: usize = 50_000;
const LINE: usize = 160;

/// Writes into `out`: perf's samples, CPU by function and CPU by system, and the heap profile of
/// what the server's allocations hold, in pprof's format.
pub fn run(game: Option<&Path>, out: &Path) {
    let crowd = tempfile::tempdir().expect("a folder to write the crowd into");
    let game = match game {
        Some(game) => game.to_path_buf(),
        None => {
            testkit::crowd().write_to(crowd.path());
            crowd.path().to_path_buf()
        }
    };
    fs::create_dir_all(out).expect("a folder for the profile");
    let server = GameServer::start(
        &binaries::beside(GAME_SERVER_PROFILING),
        &ServerConfig {
            bind: LOCALHOST,
            port: 0,
            ws_port: 0,
            max_clients: MAX_CLIENTS,
            content_dir: absolute(&game),
            player_spawn_policy: SpawnPolicy::Dist,
            player_immortal: true,
            rng_seed: Some(SEED),
            daily_reset_utc_hour: UtcHour::MIDNIGHT,
            auth: AuthMode::Fake,
            test_characters: false,
            exit_on_stdin_eof: true,
        },
    );
    let mut load = loadtest::run_dyn(&server.http_url(), &server.ws_url());

    let samples = out.join("perf.data");
    println!("[profile] sampling the server for {SAMPLED:?}");
    let mut perf = Command::new("perf")
        .args([
            "record",
            "--quiet",
            "-F",
            SAMPLE_HZ,
            "--call-graph",
            "fp",
            "-o",
        ])
        .arg(&samples)
        .args(["-p", &server.id().to_string(), "--", "sleep"])
        .arg(SAMPLED.as_secs().to_string())
        .spawn()
        .expect("perf runs");
    load.hold(SAMPLED + Duration::from_secs(1));
    let recorded = perf.wait().expect("perf finishes");
    assert!(recorded.success(), "perf record failed: {recorded}");

    let by_function = report(&samples, &["--no-children", "--percent-limit", "1"], |_| {
        true
    });
    let by_system = report(&samples, &["--children", "--percent-limit", "2"], |line| {
        line.contains("engine::systems::") || line.contains("engine::core::")
    });
    write(out, "cpu-by-function.txt", "CPU by function", &by_function);
    write(out, "cpu-by-system.txt", "CPU by system", &by_system);

    let heap = ureq::get(format!("{}/debug/pprof/heap", server.http_url()))
        .call()
        .expect("the server's heap profile")
        .body_mut()
        .read_to_vec()
        .expect("the heap profile's bytes");
    let allocations = out.join("allocations.pb.gz");
    fs::write(&allocations, heap).expect("the heap profile written");
    println!("[profile] allocations: {}", allocations.display());
}

/// perf's report of the samples, sorted by symbol: the lines of its table that `keep` picks.
fn report(samples: &Path, sorting: &[&str], keep: impl Fn(&str) -> bool) -> Vec<String> {
    let output = Command::new("perf")
        .args(["report", "-i"])
        .arg(samples)
        .args(sorting)
        .args(["--sort", "symbol", "--stdio", "-g", "none"])
        .stderr(Stdio::null())
        .output()
        .expect("perf report runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .filter(|line| keep(line))
        .map(|line| line.trim_end().chars().take(LINE).collect())
        .collect()
}

fn write(out: &Path, file: &str, title: &str, lines: &[String]) {
    println!("[profile] {title}:");
    for line in lines {
        println!("{line}");
    }
    fs::write(out.join(file), lines.join("\n") + "\n").expect("a report written");
}
