use std::sync::Arc;

use editor::app::Editor;
use editor::project::Project;
use editor::schema::SchemaShapes;
use editor::tabs::{DockRequest, EditorTab};
use editor::user::{EditorSettings, UserFolder};
use engine::core::content::ContentFile;
use engine::systems::area::AreaDef;
use ui::egui_dock::NodeIndex;

use crate::support::{area, game_folder};

#[test]
fn each_preview_takes_the_place_of_the_one_before_but_never_a_kept_tab() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);

    keep(&mut editor, item("a"));
    for key in ["b", "c", "d"] {
        preview(&mut editor, item(key));
    }

    assert_eq!(tabs(&editor), vec![item("a"), item("d")]);
}

#[test]
fn keeping_the_preview_keeps_it_where_it_is_even_once_closed_and_opened_again() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);

    keep(&mut editor, item("a"));
    preview(&mut editor, item("b"));
    keep(&mut editor, item("b"));
    preview(&mut editor, item("c"));
    assert_eq!(tabs(&editor), vec![item("a"), item("b"), item("c")]);

    let path = editor.dock.find_tab(&item("c")).expect("the preview");
    editor.dock.remove_tab(path);
    keep(&mut editor, item("c"));
    preview(&mut editor, item("d"));
    assert_eq!(
        tabs(&editor),
        vec![item("a"), item("b"), item("c"), item("d")]
    );
}

#[test]
fn previewing_an_open_tab_shows_it_and_spares_the_preview() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);

    keep(&mut editor, item("a"));
    preview(&mut editor, item("b"));
    preview(&mut editor, item("a"));
    assert_eq!(editor::tabs::active_tab(&editor), Some(item("a")));
    preview(&mut editor, item("c"));

    assert_eq!(tabs(&editor), vec![item("a"), item("c")]);
}

#[test]
fn a_renamed_preview_is_still_the_preview() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);

    preview(&mut editor, item("a"));
    editor.rename_tabs(|tab| (*tab == item("a")).then(|| item("z")));
    preview(&mut editor, item("b"));

    assert_eq!(tabs(&editor), vec![item("b")]);
}

#[test]
fn editing_what_a_preview_shows_keeps_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);
    let folder = game_folder(scratch.path().join("game"));
    editor.project = Some(Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens"));

    editor.preview_row(AreaDef::TABLE, &area());
    editor.place_tabs();
    let read = editor.read::<AreaDef>(&area()).expect("a project");
    let mut row = AreaDef::clone(&read.expect("the start area reads"));
    row.name = "Renamed field".to_owned();
    assert!(editor.write_content("Rename the area", &area(), &row));
    preview(&mut editor, EditorTab::Game);

    assert_eq!(
        tabs(&editor),
        vec![EditorTab::Area { key: area() }, EditorTab::Game]
    );
}

#[test]
fn a_tab_asking_for_focus_takes_it_from_the_other_side_of_a_split() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor(&scratch);
    keep(&mut editor, item("a"));
    let surface = editor.dock.main_surface_mut();
    let [left, _] = surface.split_right(NodeIndex::root(), 0.5, vec![item("b")]);
    surface.set_focused_node(left);
    assert_eq!(editor::tabs::active_tab(&editor), Some(item("a")));

    editor.dock_requests.push(DockRequest::Focus(item("b")));
    editor.serve_dock_requests();

    assert_eq!(editor::tabs::active_tab(&editor), Some(item("b")));
}

fn editor(scratch: &tempfile::TempDir) -> Editor {
    Editor::new(
        UserFolder::at(scratch.path().join("user")),
        EditorSettings::default(),
    )
}

fn item(key: &str) -> EditorTab {
    EditorTab::Row {
        table: "item".to_owned(),
        key: key.to_owned(),
    }
}

fn preview(editor: &mut Editor, tab: EditorTab) {
    editor.preview_tab(tab);
    editor.place_tabs();
}

fn keep(editor: &mut Editor, tab: EditorTab) {
    editor.open_tab(tab);
    editor.place_tabs();
}

fn tabs(editor: &Editor) -> Vec<EditorTab> {
    editor
        .dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .collect()
}
