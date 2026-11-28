use bevy::input::keyboard::KeyCode;
use editor::rows;
use editor::schema::SchemaShapes;
use editor::tabs::EditorTab;
use editor::views::spreadsheet;
use engine::core::content::{ContentFile, Fixture};
use engine::core::vfx::VfxDef;
use engine::systems::input::map::{InputAction, InputBinding, InputDef, InputFixture};
use engine::systems::messages;
use serde_json::json;
use strum::VariantArray;
use testkit::path;

#[test]
fn a_fixture_stays_where_the_engine_reads_it_and_a_table_of_fixtures_alone_takes_no_rows() {
    let shapes = SchemaShapes::load();

    assert!(shapes.is_closed("input"));
    assert!(shapes.is_fixture(VfxDef::TABLE, &path("vfx", "skill_area")));
    assert!(!shapes.is_closed("vfx_shader"));
    assert!(!shapes.is_closed("vfx"));

    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = crate::support::open(crate::support::game(), &scratch);
    let files_before = crate::support::files(&mut editor).paths().count();
    let copied = rows::duplicate(&mut editor, &InputFixture::Game.path());
    assert!(copied.is_err_and(|refusal| refusal.contains("can't be copied")));
    assert_eq!(
        crate::support::files(&mut editor).paths().count(),
        files_before
    );

    let fixture = InputFixture::Game.path();
    let moved = rows::move_into(&mut editor, std::slice::from_ref(&fixture), "");
    assert!(
        moved.is_err_and(|refusal| refusal.contains("stays")),
        "a fixture never moves"
    );
    assert!(
        crate::support::files(&mut editor)
            .index()
            .is_content(&fixture)
    );
}

#[test]
fn a_fixture_keeps_the_parameter_the_engine_sets_with_its_name_and_kind_pinned() {
    let shapes = SchemaShapes::load();
    let locks = shapes.locks(VfxDef::TABLE, &path("vfx", "skill_area"));
    let hostile = json!({ "name": "hostile", "default": false });
    let color = json!({ "name": "color", "default": [255, 255, 255, 255] });

    assert!(locks.iter().any(|lock| lock.keeps("/params", &hostile)));
    assert!(!locks.iter().any(|lock| lock.keeps("/params", &color)));
    let mut pinned: Vec<&str> = locks
        .iter()
        .flat_map(|lock| lock.pinned("/params", &hostile))
        .collect();
    pinned.sort_unstable();
    assert_eq!(pinned, vec!["default", "name"]);
}

#[test]
fn the_input_file_opens_as_a_sheet_of_the_actions_the_engine_fixes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = crate::support::game().edit(InputDef::TABLE, InputFixture::Game.name(), |input| {
        input["actions"]["dismiss"]["bindings"] = json!([{ "type": "key", "key": "Escape" }]);
    });
    let mut editor = crate::support::open(game, &scratch);

    assert_eq!(
        EditorTab::for_row(InputDef::TABLE, &InputFixture::Game.path()),
        EditorTab::Spreadsheet {
            table: InputDef::TABLE.to_owned()
        }
    );
    let (rows, columns, _, _) = spreadsheet::shown_rows(&mut editor, InputDef::TABLE);
    let actions: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
    let fixed: Vec<&str> = InputAction::VARIANTS.iter().map(|&a| a.into()).collect();
    assert_eq!(actions, fixed);
    let titles: Vec<&str> = columns.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["label", "bindings"]);
    let dismiss = rows
        .iter()
        .find(|row| row.key == "dismiss")
        .expect("dismiss");
    let content = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
        .expect("the content loads");
    assert_eq!(
        columns[1].text(dismiss),
        Some(InputBinding::key(KeyCode::Escape).name(&messages::of(&content).input))
    );
}
