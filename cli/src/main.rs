mod ask;
mod bake;
mod bench;
mod binaries;
mod check;
mod dialogue;
mod fit;
mod folder;
mod loadtest;
mod play;
mod profile;
mod records;
mod render;
mod search;
mod server;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use engine::core::units::Meters;

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
    /// Checks that a game folder loads whole, that every file reads as the editor writes it, and
    /// that the maps of its areas are baked from them as they are.
    Check { game: PathBuf },
    /// Rewrites every file of a game folder that doesn't read as the editor writes it, as the editor
    /// writes it.
    Format { game: PathBuf },
    /// Shapes the footprint of each PROP of GAME, named by its path in the game folder, round all
    /// of its model a walker would meet, as the editor fits it.
    Fit {
        game: PathBuf,
        #[arg(required = true)]
        props: Vec<String>,
        /// How tall the walker stands, in metres: the game's player unless given. A height below
        /// a tree's crown fits its trunk alone.
        #[arg(long)]
        tall: Option<f32>,
    },
    /// Answers the probe's questions about GAME itself, one JSON line each from stdin to stdout, as
    /// a game client playing it would; a snapshot only a client answers.
    Ask { game: PathBuf },
    /// Draws a piece of a game off screen.
    #[command(subcommand)]
    Render(Render),
    /// Works out what the designer bakes into a game, off screen, as the editor bakes it.
    #[command(subcommand)]
    Bake(Bake),
    /// The game server's ticks under a load of players playing, on a generated game, the crowd,
    /// or on GAME; with GAME, then the game client's and the editor's frame rates, and whether the
    /// server and the client hold their memory level as a player plays it.
    Bench {
        game: Option<PathBuf>,
        /// Spreads the server bench's players over the area rather than crowding them on its spawn.
        #[arg(long)]
        spread: bool,
        /// Measures the server at this many areas, `--repeat` times, rather than the bench's own
        /// load.
        #[arg(long, conflicts_with = "capacity")]
        areas: Option<usize>,
        /// Finds the most areas the server ticks within its budget, rather than measuring one
        /// load to compare builds by.
        #[arg(long)]
        capacity: bool,
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
        /// How many times each scenario runs, or the server is measured at `--areas`; the report
        /// keeps the median run and the spread.
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
    /// Plays characters of GAME on a running game server with no window: does what each JSON line
    /// of stdin tells one of them (sign in and join, send a request, say what it sees, leave),
    /// until stdin closes.
    Play {
        game: PathBuf,
        /// The game server's HTTP address.
        #[arg(long)]
        http: String,
    },
    /// Samples game-server-profiling playing the crowd, or GAME, under load, with perf, into OUT:
    /// CPU by function, CPU by system, and the heap's allocations.
    Profile {
        game: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// What a game server keeps.
    #[command(subcommand)]
    Records(records::Records),
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
    /// Draws the vfx of a game off screen.
    #[command(subcommand)]
    Vfx(RenderVfx),
    /// Draws the dialogue of GAME once for every style a line is set in and every state a choice
    /// shows, over grounds from the lightest to the darkest and the busiest, as the pictures of the
    /// folder OUT.
    Dialogue {
        game: PathBuf,
        out: PathBuf,
        /// Pictures to draw the dialogue over too, such as frames of the game.
        #[arg(long)]
        over: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RenderVfx {
    /// Films the coats SHEET lays out, a JSON file of tiles that each name a vfx of GAME and what
    /// it plays on, as the video OUT, through ffmpeg: page after page of tiles, each circled once
    /// by a camera of its own.
    Coat {
        game: PathBuf,
        sheet: PathBuf,
        out: PathBuf,
        /// The tiles to film, by patterns their labels match, such as "Blaze:*": every tile of
        /// the sheet when none is given.
        names: Vec<String>,
    },
}

#[derive(Subcommand)]
enum Bake {
    /// Bakes the maps of every area of GAME into its bundles: the squares whose ground, props or
    /// style changed since they were baked are drawn again, and the rest kept.
    Maps {
        game: PathBuf,
        /// The folder the renderer keeps what it works out once and reads again (the models'
        /// impostors) under, as a player's profile keeps it: a fresh one for this run when not
        /// given.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Schema {
    /// What a game client started with a probe address answers on it.
    Probe,
    /// What a game server prints once ready, and what its test routes take.
    Server,
    /// What `rift bench` writes as its report.
    Bench,
    /// What `rift play` takes on stdin, among it every request a game client sends, and what it
    /// prints.
    Play,
}

fn main() {
    engine::core::par::start();
    match Rift::parse().command {
        Command::Check { game } => check::run(&game),
        Command::Format { game } => check::format(&game),
        Command::Fit { game, props, tall } => fit::run(&game, tall.map(Meters), &props),
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
        Command::Render(Render::Vfx(RenderVfx::Coat {
            game,
            sheet,
            out,
            names,
        })) => render::coat::run(&game, &sheet, &out, &names),
        Command::Render(Render::Dialogue { game, out, over }) => dialogue::run(&game, &out, &over),
        Command::Bake(Bake::Maps { game, profile }) => bake::maps(&game, profile.as_deref()),
        Command::Bench {
            game,
            spread,
            areas,
            capacity,
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
                areas,
                capacity,
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
        Command::Play { game, http } => {
            if let Err(failure) = play::run(&game, &http) {
                eprintln!("{failure}");
                std::process::exit(1);
            }
        }
        Command::Profile { game, out } => profile::run(game.as_deref(), &out),
        Command::Records(records) => records::run(records),
        Command::Schema(Schema::Probe) => println!("{}", engine::systems::probe::schema()),
        Command::Schema(Schema::Server) => println!("{}", engine::systems::launch::server_schema()),
        Command::Schema(Schema::Bench) => println!("{}", bench::schema()),
        Command::Schema(Schema::Play) => {
            println!("{}", engine::systems::player::headless::schema())
        }
    }
}
