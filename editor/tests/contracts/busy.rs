use std::sync::mpsc;

use editor::app::Editor;
use editor::busy::SHOWN_AFTER;
use editor::project::{ContentEdit, FileBytes};
use editor::tree::RowToggle;
use editor::views::area::nodes::NodeFlag;
use editor::views::area::scene_tree;
use engine::systems::area::scene::scene_file;
use ui::egui::{self, Key, Modifiers};

use crate::support::{self, Frames};

#[test]
fn work_the_editor_waits_on_holds_its_input_and_says_what_it_waits_on_once_it_lasts() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game(), &scratch);
    editor::rows::new_file(&mut editor, "", "npc").expect("a new npc");
    support::files(&mut editor);
    let (finish, finished) = mpsc::channel::<()>();
    editor.run_busy(
        "Pondering",
        move |progress| {
            progress.doing("Thinking it over", None);
            let _ = finished.recv();
        },
        |editor, ()| editor.toast("Pondered"),
    );
    let mut frames = Frames::new(egui::vec2(1280.0, 800.0));
    let mut frame = |editor: &mut Editor, events: Vec<egui::Event>| {
        let drawn = frames.run(events, |ui| editor::shell::show(editor, ui.ctx()));
        editor.poll_busy();
        drawn
    };

    let at_once = frame(&mut editor, Vec::new());
    let saving = frame(&mut editor, support::key(Key::S, Modifiers::COMMAND));
    std::thread::sleep(SHOWN_AFTER);
    // A dialog takes its first frame to size itself, unseen.
    frame(&mut editor, Vec::new());
    let lasting = frame(&mut editor, Vec::new());
    finish.send(()).expect("the work waits");
    editor.finish_busy();
    let done = frame(&mut editor, Vec::new());

    assert!(
        !at_once.any(|text| text == "Pondering"),
        "a short wait shows nothing"
    );
    assert!(!saving.any(|text| text == "Pondering"));
    assert!(lasting.any(|text| text == "Pondering"), "{lasting:?}");
    assert!(
        lasting.any(|text| text == "Thinking it over"),
        "{lasting:?}"
    );
    assert!(!done.any(|text| text == "Pondering"));
    assert_eq!(
        editor.toast.as_ref().map(|(text, _)| text.as_str()),
        Some("Pondered")
    );
    assert!(
        editor.has_unsaved(),
        "the keys pressed meanwhile never reached the editor"
    );
}

#[test]
fn an_edit_never_waits_and_the_load_after_it_takes_back_one_the_folder_does_not_load_with() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::large_game(), &scratch);
    let scene = scene_file(&support::area());

    scene_tree::toggle(
        &mut editor,
        &support::area(),
        &["/0".to_owned()],
        RowToggle::Hide,
    );
    assert!(
        !editor.busy.is_busy(),
        "a scene's change applies on the frame"
    );
    assert!(NodeFlag::Hidden.of(&support::scene(&editor, &support::area())[0]));

    let before = support::settled(&mut editor).files.get(&scene);
    let mut broken = before.as_deref().expect("the scene file").to_vec();
    let at = broken
        .iter()
        .position(|byte| *byte == b'[')
        .expect("a list")
        + 1;
    broken.splice(at..at, br#"{ "type": "no such module" },"#.iter().copied());
    let mut edit = ContentEdit::new("Break the scene");
    edit.write(scene.clone(), FileBytes::from(broken));
    editor
        .try_apply(edit)
        .expect("a file too large to check on a frame applies unchecked");
    assert!(
        !editor.busy.is_busy(),
        "even one too large to check waits on nothing"
    );

    support::settled(&mut editor);
    let project = editor.project.as_mut().expect("a project");
    let refused = project.take_refused().expect("the load takes it back");
    assert_eq!(refused.label, "Break the scene");
    assert_eq!(project.files.get(&scene), before);
}
