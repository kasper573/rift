mod ask;
mod bench;
mod binaries;
mod check;
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
    /// desktop client's frame rate and whether both hold their memory level as a player plays it.
    Bench {
        game: Option<PathBuf>,
        /// Spreads the server bench's players over the area rather than crowding them on its spawn.
        #[arg(long)]
        spread: bool,
        /// Runs only these parts of the bench, each named once.
        #[arg(long, value_enum)]
        only: Vec<bench::BenchPart>,
    },
    /// Simulated players against a running game server that takes any account's sign-in:
    /// COUNT of them, or as many as it holds within its tick budget.
    Loadtest {
        /// The game server's HTTP address.
        #[arg(long)]
        http: String,
        /// The game server's WebSocket address.
        #[arg(long)]
        ws: String,
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
    },
}

#[derive(Subcommand)]
enum Schema {
    /// What a desktop client started with a probe address answers on it.
    Probe,
    /// What a game server prints once ready, and what its test routes take.
    Server,
}

fn main() {
    match Rift::parse().command {
        Command::Check { game } => check::run(&game),
        Command::Format { game } => check::format(&game),
        Command::Ask { game } => ask::run(&game),
        Command::Render(Render::Area { game, shot, out }) => render::area(&game, &shot, &out),
        Command::Bench { game, spread, only } => {
            if let Err(failure) = bench::run(game.as_deref(), spread, &only) {
                eprintln!("{failure}");
                std::process::exit(1);
            }
        }
        Command::Loadtest { http, ws, count } => loadtest::run(&http, &ws, count),
        Command::Profile { game, out } => profile::run(game.as_deref(), &out),
        Command::Schema(Schema::Probe) => println!("{}", engine::systems::probe::schema()),
        Command::Schema(Schema::Server) => println!("{}", engine::systems::launch::server_schema()),
    }
}
