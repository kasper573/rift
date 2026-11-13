use std::io::{BufRead, Write};
use std::path::Path;

use engine::core::content::LoadPurpose;
use engine::systems::probe;

use crate::folder;

/// Answers the probe's questions about the game itself, one JSON line each from stdin to stdout,
/// as a game client playing it would, until stdin closes.
pub fn run(game: &Path) {
    let content =
        folder::load(game, "ask", LoadPurpose::Play).unwrap_or_else(|report| panic!("{report}"));
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let question = probe::read_question(&content, &line)
            .unwrap_or_else(|error| panic!("asked {line}: {error}"));
        let answer = probe::answer(&content, &question);
        writeln!(out, "{answer}").expect("stdout takes answers");
        out.flush().expect("stdout takes answers");
    }
}
