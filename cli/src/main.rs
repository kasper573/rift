mod ask;
mod bench;
mod binaries;
mod check;
mod compare;
mod folder;
mod loadtest;
mod profile;
mod render;
mod search;
mod server;
mod snapshot;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

// The server's own allocator setup, so the bench's ticks allocate exactly as production does.
service::heap_profiling!();

/// Tools for a game made with rift.
#[derive(Parser)]
#[command(name = "rift")]
struct Rift {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Checks that a game folder loads whole, and that every file reads as the editor writes it.
    Check { game: PathBuf },
    /// Rewrites every file of a game folder that doesn't read as the editor writes it, as the editor
    /// writes it.
    Format { game: PathBuf },
    /// Answers the probe's questions about GAME itself, one JSON line each from stdin to stdout, as
    /// a desktop client playing it would; a snapshot only a client answers.
    Ask { game: PathBuf },
    /// Draws a piece of a game off screen.
    #[command(subcommand)]
    Render(Render),
    /// The game server's capacity, on a generated game, the crowd, or on GAME; with GAME, then the
    /// desktop client's frame rate and whether both hold their memory level as a player plays it.
    Bench {
        game: Option<PathBuf>,
        /// Spreads the server bench's players over the area rather than crowding them on its spawn.
        #[arg(long)]
        spread: bool,
        /// Runs only these parts of the bench, each named once.
        #[arg(long, value_enum)]
        only: Vec<bench::BenchPart>,
        /// Runs only the scenarios whose names match, `*` standing for anything.
        #[arg(long)]
        scenario: Option<String>,
        /// The game's scenarios file, naming what the scenarios play with; `bench/scenarios.json`
        /// in the game's folder unless given, and the content fills whatever neither names.
        #[arg(long)]
        scenarios: Option<PathBuf>,
        /// The video presets to run the scenarios at.
        #[arg(long, value_enum, default_value_t = render::VideoFilter::Both)]
        video: render::VideoFilter,
        /// How many times each scenario runs; the report keeps the median run and the spread.
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// Where the report is written.
        #[arg(long, default_value = "bench-report.json")]
        report: PathBuf,
        /// A report of an earlier run to compare against: fails when a scenario got worse.
        #[arg(long)]
        compare: Option<PathBuf>,
        /// The adapter the client must draw on; the bench refuses to run on another.
        #[arg(long)]
        adapter: Option<String>,
        /// One short scenario per target without criteria, proving the harness alone.
        #[arg(long)]
        smoke: bool,
        /// Takes pictures of what the client and the editor show instead of their frame rates,
        /// into DIR by adapter, a set each for `rift compare`.
        #[arg(long, value_name = "DIR")]
        snapshots: Option<PathBuf>,
    },
    /// Simulated players against a running game server that takes any account's sign-in:
    /// COUNT of them, or as many as it holds within its tick budget.
    Loadtest {
        /// The game server's HTTP address.
        #[arg(long)]
        http: String,
        count: Option<usize>,
    },
    /// Samples game-server-profiling playing the crowd, or GAME, under load, with perf, into OUT:
    /// CPU by function, CPU by system, and the heap's allocations.
    Profile {
        game: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Draws every shot of SHOTS, a folder of `*.shot.json` files, each at the video presets it
    /// asks for, into OUT by adapter and preset, with a manifest of what was drawn.
    Snapshot {
        game: PathBuf,
        shots: PathBuf,
        out: PathBuf,
        /// The video presets to draw at, of those each shot asks for.
        #[arg(long, value_enum, default_value_t = render::VideoFilter::Both)]
        video: render::VideoFilter,
        /// Draws only the shots whose names match, `*` standing for anything.
        #[arg(long)]
        only: Option<String>,
    },
    /// Compares the snapshots in CANDIDATE against those in BASELINE, shot by shot: exit 0 alike,
    /// 1 a shot strayed, 2 the sets don't compare, 3 a render crashed.
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
        /// Writes report.json and report.html, with the three pictures of each failing shot, here.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Takes the candidate's pictures over the baseline's for the shots whose names match.
        #[arg(long)]
        accept: Option<String>,
        /// Each shot's allowance, as `rift noise` writes them; the floor for every shot otherwise.
        #[arg(long)]
        thresholds: Option<PathBuf>,
    },
    /// Draws every shot of SHOTS RUNS times, each in a process of its own, and writes each shot's
    /// allowance, three times the noise between its runs, to OUT/thresholds.json.
    Noise {
        game: PathBuf,
        shots: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 5)]
        runs: usize,
    },
    /// Writes each shot's allowance, three times the noise between SETS already taken of the same
    /// shots (each the folder of one adapter's pictures, as `compare` takes them), to
    /// OUT/thresholds.json.
    NoiseBetween {
        out: PathBuf,
        #[arg(num_args = 2..)]
        sets: Vec<PathBuf>,
    },
    /// Prints the JSON Schema of a protocol the binaries speak.
    #[command(subcommand)]
    Schema(Schema),
}

#[derive(Subcommand)]
enum Render {
    /// Draws SHOT, a JSON file naming an area of GAME, a lens and the bodies standing in it, through
    /// the world's own camera as the picture OUT.
    Area {
        game: PathBuf,
        shot: PathBuf,
        out: PathBuf,
        /// Draws the shot a second time in a process of its own and prints how far the two
        /// pictures differ, failing when they differ by more than the eye can tell.
        #[arg(long)]
        check_determinism: bool,
    },
}

#[derive(Subcommand)]
enum Schema {
    /// What a desktop client started with a probe address answers on it.
    Probe,
    /// What a game server prints once ready, and what its test routes take.
    Server,
    /// What `rift bench` writes as its report.
    Bench,
}

fn exit_on(result: Result<(), String>) {
    if let Err(failure) = result {
        eprintln!("{failure}");
        std::process::exit(2);
    }
}

fn main() {
    engine::core::par::start();
    match Rift::parse().command {
        Command::Check { game } => check::run(&game),
        Command::Format { game } => check::format(&game),
        Command::Ask { game } => ask::run(&game),
        Command::Render(Render::Area {
            game,
            shot,
            out,
            check_determinism,
        }) => {
            if check_determinism {
                if let Err(failure) = render::check_determinism(&game, &shot, &out) {
                    eprintln!("{failure}");
                    std::process::exit(1);
                }
            } else {
                render::area(&game, &shot, &out);
            }
        }
        Command::Bench {
            game,
            spread,
            only,
            scenario,
            scenarios,
            video,
            repeat,
            report,
            compare,
            adapter,
            smoke,
            snapshots,
        } => {
            let args = bench::BenchArgs {
                game,
                spread,
                only,
                scenario,
                scenarios,
                video,
                repeat,
                report,
                compare,
                adapter,
                smoke,
                snapshots,
            };
            match bench::run(&args) {
                Ok(code) => std::process::exit(code),
                Err(failure) => {
                    eprintln!("{failure}");
                    std::process::exit(2);
                }
            }
        }
        Command::Loadtest { http, count } => {
            if let Err(failure) = loadtest::run(&http, count) {
                eprintln!("{failure}");
                std::process::exit(1);
            }
        }
        Command::Snapshot {
            game,
            shots,
            out,
            video,
            only,
        } => exit_on(snapshot::run(&game, &shots, &out, video, only.as_deref())),
        Command::Compare {
            baseline,
            candidate,
            report,
            accept,
            thresholds,
        } => match compare::run(
            &baseline,
            &candidate,
            report.as_deref(),
            accept.as_deref(),
            thresholds.as_deref(),
        ) {
            Ok(outcome) => std::process::exit(outcome.exit_code()),
            Err(failure) => {
                eprintln!("{failure}");
                std::process::exit(2);
            }
        },
        Command::Noise {
            game,
            shots,
            out,
            runs,
        } => exit_on(snapshot::noise(&game, &shots, &out, runs)),
        Command::NoiseBetween { out, sets } => exit_on(snapshot::noise_between(&sets, &out)),
        Command::Profile { game, out } => profile::run(game.as_deref(), &out),
        Command::Schema(Schema::Probe) => println!("{}", engine::systems::probe::schema()),
        Command::Schema(Schema::Server) => println!("{}", engine::systems::launch::server_schema()),
        Command::Schema(Schema::Bench) => println!("{}", bench::schema()),
    }
}
