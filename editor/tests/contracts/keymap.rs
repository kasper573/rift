use editor::keymap::{EditorKey, KeyScope};
use strum::VariantArray;
use ui::egui;
use ui::egui::{Context, Event, Key, Modifiers, RawInput};

#[test]
fn a_chord_with_more_modifiers_held_claims_its_key_alone() {
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    assert_eq!(pressed_by(Key::Z, ctrl_shift), vec![EditorKey::Redo]);
    assert_eq!(
        pressed_by(Key::Z, Modifiers::COMMAND),
        vec![EditorKey::Undo]
    );
    assert_eq!(
        pressed_by(Key::F5, Modifiers::SHIFT),
        vec![EditorKey::StopPlay]
    );
    assert_eq!(
        pressed_by(Key::P, Modifiers::SHIFT),
        vec![EditorKey::FootprintsOverlay]
    );
}

#[test]
fn a_modifier_held_for_a_finer_step_keeps_the_key_it_steps_with() {
    let pressed = pressed_by(Key::ArrowLeft, Modifiers::SHIFT);
    assert_eq!(pressed, vec![EditorKey::Move]);
}

#[test]
fn every_key_is_named_and_listed_in_the_sheet_once() {
    let listed: Vec<EditorKey> = KeyScope::VARIANTS
        .iter()
        .flat_map(|scope| scope.keys())
        .collect();
    assert_eq!(listed.len(), EditorKey::VARIANTS.len());
    for key in EditorKey::VARIANTS {
        assert!(listed.contains(key), "{key:?} is not on the sheet");
        assert!(!key.name().is_empty(), "{key:?} has no name");
        assert!(!key.about().is_empty(), "{key:?} says nothing");
    }
}

fn pressed_by(key: Key, modifiers: Modifiers) -> Vec<EditorKey> {
    let ctx = Context::default();
    let input = RawInput {
        events: vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
        ..RawInput::default()
    };
    let mut pressed = Vec::new();
    let mut output = ctx.run_ui(input, |ui| {
        pressed = EditorKey::VARIANTS
            .iter()
            .copied()
            .filter(|editor_key| ui.input(|i| editor_key.pressed(i)))
            .collect();
    });
    output.textures_delta.clear();
    pressed
}

#[test]
fn saving_reaches_the_editor_while_a_field_is_typed_into_and_undo_stays_with_the_field() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = crate::support::open(crate::support::game(), &scratch);
    let made = editor::rows::new_file(&mut editor, "", "npc").expect("a new npc");
    let mut frames = crate::support::Frames::new(egui::vec2(800.0, 600.0));
    let mut typed = String::new();
    let mut frame = |editor: &mut editor::app::Editor, events: Vec<Event>| {
        frames.run(events, |ui| {
            ui.text_edit_singleline(&mut typed).request_focus();
            editor::shell::shortcuts(editor, ui.ctx());
        });
    };

    frame(&mut editor, Vec::new());
    frame(&mut editor, crate::support::key(Key::Z, Modifiers::COMMAND));
    assert!(crate::support::files(&mut editor).contains(&made));
    frame(&mut editor, crate::support::key(Key::S, Modifiers::COMMAND));
    assert!(!editor.has_unsaved());
    assert!(scratch.path().join("game").join(&made).is_file());
}
