use std::collections::BTreeSet;
use std::path::Path;

use engine::core::content::write;
use engine::systems::registry;
use walkdir::WalkDir;

use crate::support::{TempFolder, load, repo};

#[test]
fn writing_the_shipped_folder_back_changes_no_bytes() {
    let folder = repo().join("example_game");
    let content = load(&folder).unwrap_or_else(|report| panic!("{report}"));
    let writer = write(&registry(), content, &folder).expect("written");

    assert_eq!(writer.written(), &[] as &[String], "rewritten files");
    assert_eq!(writer.kept(), rows_and_grids(&folder).len());
}

#[test]
fn a_folder_written_from_scratch_lists_every_row_and_grid() {
    let folder = repo().join("example_game");
    let content = load(&folder).unwrap_or_else(|report| panic!("{report}"));
    let fresh = TempFolder::of_template();
    std::fs::remove_dir_all(fresh.path().join("tables")).expect("the template's tables");
    std::fs::remove_file(fresh.path().join("game.json")).expect("the template's game.json");

    let writer = write(&registry(), content, fresh.path()).expect("written");

    let written: BTreeSet<String> = writer.written().iter().cloned().collect();
    assert_eq!(written, rows_and_grids(&folder));
    for file in &written {
        assert_eq!(
            std::fs::read(fresh.path().join(file)).expect("fresh"),
            std::fs::read(folder.join(file)).expect("shipped"),
            "{file} differs"
        );
    }
}

fn rows_and_grids(folder: &Path) -> BTreeSet<String> {
    let tables = WalkDir::new(folder.join("tables"))
        .follow_links(true)
        .into_iter()
        .map(|entry| entry.expect("an entry"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry.path().strip_prefix(folder).expect("under the folder");
            relative.to_string_lossy().replace('\\', "/")
        });
    std::iter::once("game.json".to_owned())
        .chain(tables)
        .collect()
}
