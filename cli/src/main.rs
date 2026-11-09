mod ask;
mod bench;
mod binaries;
mod check;
mod folder;
mod loadtest;
mod profile;
mod render;
mod search;
mod server;

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
    /// desktop client's and the editor's frame rates, and whether the server and the client hold
    /// their memory level as a player plays it.
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
        #[arg(long, value_enum, default_value_t = render::VideoFilter::Highest)]
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
        /// The folder the renderer keeps what it works out once and reads again (the models'
        /// impostors) in, as a player's profile keeps it: a fresh one for this run when not
        /// given.
        #[arg(long)]
        profile: Option<PathBuf>,
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
            profile,
        }) => {
            if check_determinism {
                if let Err(failure) =
                    render::check_determinism(&game, &shot, &out, profile.as_deref())
                {
                    eprintln!("{failure}");
                    std::process::exit(1);
                }
            } else {
                render::area(&game, &shot, &out, profile.as_deref());
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
        Command::Profile { game, out } => profile::run(game.as_deref(), &out),
        Command::Schema(Schema::Probe) => println!("{}", engine::systems::probe::schema()),
        Command::Schema(Schema::Server) => println!("{}", engine::systems::launch::server_schema()),
        Command::Schema(Schema::Bench) => println!("{}", bench::schema()),
    }
}
