use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use engine::core::content::{Content, FilesystemFolder, LoadWarnings, write};
use engine::systems::actor::model::{ActorModel, ModelDef};
use engine::systems::actor::skeleton::{SkeletonDef, SkeletonId};
use engine::systems::actor::stance;
use engine::systems::item::ItemDef;
use engine::systems::{GameSettings, load_content, registry};
use walkdir::WalkDir;

/// Exits with an error when the game folder isn't whole: it must load as a game, and every file of
/// a kind the editor writes must read exactly as the editor writes it, so saving the game in the
/// editor changes only what was edited.
pub fn run(game: &Path) {
    let problems = problems(game);
    if problems.is_empty() {
        println!(
            "{}: loads whole, every file as the editor writes it",
            game.display()
        );
        return;
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    eprintln!("\n{}: {} problem(s)", game.display(), problems.len());
    std::process::exit(1);
}

/// Rewrites every file of the game that doesn't read as the editor writes it, as the editor writes
/// it: what each holds stays, only its layout changes.
pub fn format(game: &Path) {
    let content = load_content(Arc::new(FilesystemFolder(game.to_path_buf())), "format")
        .unwrap_or_else(|report| panic!("{report}"));
    let rewritten = as_the_editor_writes(game, content);
    for file in &rewritten.differing {
        fs::write(game.join(file), rewritten.editors(file)).expect("a rewritten file of the game");
    }
    println!(
        "{}: {} file(s) rewritten as the editor writes them",
        game.display(),
        rewritten.differing.len()
    );
}

fn problems(game: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let content = match load_content(Arc::new(FilesystemFolder(game.to_path_buf())), "check") {
        Ok(content) => content,
        Err(report) => {
            out.push(format!("does not load:\n{report}"));
            return out;
        }
    };
    check_bodies(&content, &mut out);
    let rewritten = as_the_editor_writes(game, content);
    for file in &rewritten.unwritten {
        out.push(format!("{file}: a file the editor never writes"));
    }
    for file in &rewritten.differing {
        let ours = fs::read(game.join(file)).expect("a file of the game");
        out.push(format!(
            "{file}: not as the editor writes it, {}",
            first_difference(
                &String::from_utf8_lossy(&ours),
                &String::from_utf8_lossy(&rewritten.editors(file))
            )
        ));
    }
    out
}

/// The game as the editor writes it, beside the game as it is.
struct Rewritten {
    scratch: tempfile::TempDir,
    /// Files of the kinds the editor writes that it doesn't write.
    unwritten: Vec<String>,
    /// Files the editor writes otherwise than the game has them.
    differing: Vec<String>,
}

impl Rewritten {
    fn editors(&self, file: &str) -> Vec<u8> {
        fs::read(self.scratch.path().join(file)).expect("a written file")
    }
}

fn as_the_editor_writes(game: &Path, content: Content) -> Rewritten {
    let scratch = tempfile::tempdir().expect("a scratch folder to write the game into");
    let writer =
        write(registry(), content, scratch.path()).expect("write the game to the scratch folder");
    let written: BTreeSet<String> = writer.written().iter().cloned().collect();
    let shipped = written_kinds(game);
    let differing = written
        .intersection(&shipped)
        .filter(|file| {
            fs::read(game.join(file)).expect("a file of the game")
                != fs::read(scratch.path().join(file)).expect("a written file")
        })
        .cloned()
        .collect();
    Rewritten {
        unwritten: shipped.difference(&written).cloned().collect(),
        differing,
        scratch,
    }
}

/// What a game's bodies must be beyond what loads: every skeleton worn by a model, and a set on the
/// player's skeleton for every stance an item names, so no weapon falls back to the base set. The
/// loader's warnings are printed, the designer's to accept.
fn check_bodies(content: &Content, out: &mut Vec<String>) {
    for warning in &content.built::<LoadWarnings>().0 {
        println!("warning: {warning}");
    }
    let worn: BTreeSet<SkeletonId> = content
        .table::<ModelDef>()
        .rows()
        .map(|model| model.skeleton)
        .collect();
    for skeleton in content.table::<SkeletonDef>().ids() {
        if !worn.contains(&skeleton) {
            out.push(format!(
                "{}: no model wears this skeleton",
                skeleton.path(content)
            ));
        }
    }
    let player = content.built::<GameSettings>().player.get(content);
    let body = ActorModel::get(content, player.class.get(content).model).skeleton();
    for item in content.table::<ItemDef>().ids() {
        let Some(stance) = stance::of_item(content, item) else {
            continue;
        };
        if body.def.set(stance).is_none() {
            out.push(format!(
                "{}: its stance {} has no set on the player's skeleton",
                item.path(content),
                stance.path(content)
            ));
        }
    }
}

/// Every file of the kinds the editor writes.
fn written_kinds(folder: &Path) -> BTreeSet<String> {
    WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry.path().strip_prefix(folder).unwrap_or(entry.path());
            relative.to_string_lossy().replace('\\', "/")
        })
        .filter(|file| registry().writes(file))
        .collect()
}

fn first_difference(ours: &str, editors: &str) -> String {
    let mut ours_lines = ours.lines();
    let mut editors_lines = editors.lines();
    for line in 1.. {
        match (ours_lines.next(), editors_lines.next()) {
            (Some(a), Some(b)) if a == b => continue,
            (None, None) => break,
            (a, b) => {
                return format!(
                    "from line {line}: it has `{}` where the editor writes `{}`",
                    a.unwrap_or("the end"),
                    b.unwrap_or("the end")
                );
            }
        }
    }
    "in its line endings".to_owned()
}
