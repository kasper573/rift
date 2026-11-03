use std::sync::Arc;

use editor::project::watch::FolderChange;
use editor::project::{
    ContentEdit, EditViewpoint, FileBytes, FileChange, Project, RowSelection, json_bytes,
};
use editor::schema::SchemaShapes;
use engine::core::content::SETTINGS_FILE;

use crate::support::game_folder;

#[test]
fn an_outside_change_restarts_the_saved_marker_at_the_disk_state() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");

    project
        .apply(rename(&project, "Lowtide"), viewpoint())
        .expect("applies");
    assert!(project.save().is_ok(), "the project saves");
    assert_eq!(project.history.saved(), Some(1));

    let mut outside = project
        .files
        .json_owned(SETTINGS_FILE)
        .expect("game.config.json");
    outside["name"] = "Outside".into();
    std::fs::write(folder.join(SETTINGS_FILE), &*json_bytes(&outside)).expect("writes outside");
    let changes = project.outside_changes(FolderChange::Unknown);
    assert!(changes.taken.iter().any(|file| file == SETTINGS_FILE));
    assert_eq!(project.history.saved(), Some(0));

    for name in ["First", "Second", "Third"] {
        project
            .apply(rename(&project, name), viewpoint())
            .expect("applies");
    }
    assert_eq!(project.history.steps().len(), 3);
    assert_eq!(project.history.saved(), Some(0));
}

#[test]
fn a_folder_made_then_filled_saves_as_the_disk_reads_it_back_and_undo_keeps_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");
    for (file, label) in [("props/", "New folder"), ("props/crate.md", "New note")] {
        let mut edit = ContentEdit::new(label);
        edit.write(file, FileBytes::from(b"".as_slice()));
        project.apply(edit, viewpoint()).expect("applies");
    }

    assert!(project.save().is_ok(), "the project saves");
    let changes = project.outside_changes(FolderChange::Unknown);

    assert!(changes.taken.is_empty(), "{:?}", changes.taken);
    assert_eq!(project.history.steps().len(), 2);
    project.undo();
    assert!(
        project.files.index().folders.contains("props"),
        "the emptied folder stays"
    );
}

#[test]
fn the_unsaved_files_follow_edits_saves_and_changes_on_disk() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");
    assert!(project.unsaved().is_empty());

    project
        .apply(rename(&project, "Lowtide"), viewpoint())
        .expect("applies");
    assert_eq!(&*project.unsaved(), [SETTINGS_FILE.to_owned()]);

    assert!(project.save().is_ok(), "the project saves");
    assert!(
        project.unsaved().is_empty(),
        "saving leaves nothing unsaved"
    );

    let mut outside = project
        .files
        .json_owned(SETTINGS_FILE)
        .expect("game.config.json");
    outside["name"] = "Outside".into();
    std::fs::write(folder.join(SETTINGS_FILE), &*json_bytes(&outside)).expect("writes outside");
    project.outside_changes(FolderChange::Files(vec![SETTINGS_FILE.to_owned()]));
    assert!(
        project.unsaved().is_empty(),
        "a change taken in from disk is saved"
    );

    project
        .apply(rename(&project, "Again"), viewpoint())
        .expect("applies");
    outside["name"] = "Outside again".into();
    std::fs::write(folder.join(SETTINGS_FILE), &*json_bytes(&outside)).expect("writes");
    project.outside_changes(FolderChange::Files(vec![SETTINGS_FILE.to_owned()]));
    assert_eq!(
        &*project.unsaved(),
        [SETTINGS_FILE.to_owned()],
        "an edit made here that disk now differs from stays unsaved"
    );
}

#[test]
fn the_saved_step_stays_marked_through_undo_until_a_new_edit_discards_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");
    for name in ["One", "Two"] {
        project
            .apply(rename(&project, name), viewpoint())
            .expect("applies");
    }
    assert!(project.save().is_ok(), "the project saves");

    project.undo();
    assert_eq!(
        project.history.saved(),
        Some(2),
        "undo keeps the saved step marked"
    );

    project
        .apply(rename(&project, "Three"), viewpoint())
        .expect("applies");
    assert_eq!(
        project.history.saved(),
        None,
        "a saved step a new edit discarded is gone"
    );
}

#[test]
fn undoing_a_merged_step_restores_every_file_it_came_to_touch() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = game_folder(scratch.path().join("game"));
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");
    let settings = project.files.get(SETTINGS_FILE);
    let note = "notes/plan.md";

    let mut first = rename(&project, "Lowtide");
    first.label = "paint".to_owned();
    project
        .apply(first.merging(), viewpoint())
        .expect("applies");
    let mut second = ContentEdit::new("paint");
    second.write(note, Arc::from(&b"later"[..]));
    project
        .apply(second.merging(), viewpoint())
        .expect("applies");
    assert_eq!(
        project.history.steps().len(),
        1,
        "the edits merge into one step"
    );

    project.undo();
    assert_eq!(project.files.get(SETTINGS_FILE), settings);
    assert!(
        project.files.get(note).is_none(),
        "the note the step wrote is gone"
    );
}

fn viewpoint() -> EditViewpoint {
    EditViewpoint {
        selection: RowSelection::game(),
        camera: None,
    }
}

fn rename(project: &Project, name: &str) -> ContentEdit {
    let mut game = project
        .files
        .json_owned(SETTINGS_FILE)
        .expect("game.config.json");
    game["name"] = name.into();
    ContentEdit {
        label: format!("rename to {name}"),
        changes: vec![FileChange {
            file: SETTINGS_FILE.to_owned(),
            bytes: Some(json_bytes(&game)),
        }],
        merge: false,
        amends: None,
    }
}
