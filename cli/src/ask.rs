use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use engine::core::content::FilesystemFolder;
use engine::systems::{load_content, probe};

/// Answers the probe's questions about the game itself, one JSON line each from stdin to stdout,
/// as a desktop client playing it would, until stdin closes.
pub fn run(game: &Path) {
    let content = load_content(Arc::new(FilesystemFolder(game.to_path_buf())), "ask")
        .unwrap_or_else(|report| panic!("{report}"));
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let question = probe::read_question(&content, &line)
            .unwrap_or_else(|error| panic!("asked {line}: {error}"));
        let answer = probe::answer(&content, &question)
            .unwrap_or_else(|| panic!("only a desktop client answers {line}, from a frame"));
        writeln!(out, "{answer}").expect("stdout takes answers");
        out.flush().expect("stdout takes answers");
    }
}
