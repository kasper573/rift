use std::path::{Component, Path, PathBuf};

use editor::app::Editor;
use editor::user::{EditorSettings, UserFolder};

use crate::support::game_folder;

#[test]
fn a_folder_opened_by_a_relative_path_is_known_by_its_absolute_one() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut editor = Editor::new(
        UserFolder::at(scratch.path().join("user")),
        EditorSettings::default(),
    );

    editor.open_folder(&relative_from_here(&folder));
    editor.finish_busy();

    let root = &editor.project.as_ref().expect("the game opens").root;
    assert!(root.is_absolute(), "{} is relative", root.display());
    assert_eq!(
        std::fs::canonicalize(root).expect("the root exists"),
        std::fs::canonicalize(&folder).expect("the folder exists")
    );
}

fn relative_from_here(path: &Path) -> PathBuf {
    let here = std::env::current_dir().expect("a working directory");
    let up = here
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .map(|_| Component::ParentDir);
    let down = path
        .components()
        .filter(|component| matches!(component, Component::Normal(_)));
    up.chain(down).collect()
}
