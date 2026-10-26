use editor::views::{item, stance};
use engine::core::geometry::Degrees;
use engine::core::units::Meters;
use engine::systems::actor::skeleton::{LocalOffset, LocalTurn};
use engine::systems::actor::stance::StanceDef;
use engine::systems::item::{CARD_SIZE, Grip, ICON_SIZE, ItemDef, Wear};
use serde_json::json;
use testkit::path;

use crate::support;

const POTION: &str = "potion";

#[test]
fn an_items_pictures_are_checked_against_the_sizes_the_game_draws() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().item(POTION, json!({ "kind": { "type": "resource" } }));
    let mut editor = support::open(game, &scratch);
    let key = path("item", POTION);
    let checks = item::pictures(&mut editor, &key);
    assert_eq!(
        checks
            .iter()
            .map(|check| (check.field, check.wanted, check.fits()))
            .collect::<Vec<_>>(),
        vec![("icon", ICON_SIZE, true), ("card", CARD_SIZE, true)]
    );

    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .item(POTION, json!({ "kind": { "type": "resource" } }))
        .asset(&format!("icons/{POTION}.png"), testkit::png(48, 48));
    let mut editor = support::open_as_is(game, &scratch);
    let checks = item::pictures(&mut editor, &key);
    let icon = checks
        .iter()
        .find(|check| check.field == "icon")
        .expect("the icon is checked");
    assert_eq!(icon.size, Some((48, 48)));
    assert!(!icon.fits(), "a 48 px icon is not the 32 px the game draws");
}

#[test]
fn a_held_items_stance_and_grip_land_in_its_file_and_its_pairs_in_the_stances() {
    let scratch = tempfile::tempdir().expect("scratch");
    let (sword, dagger, dual) = ("sword", "dagger", "dual");
    let game = support::game()
        .stance(sword, 1, "main")
        .stance(dagger, 1, "either")
        .stance(dual, 2, "main")
        .item(
            "blade",
            json!({ "kind": { "type": "equipment", "slot": "hand", "stance": path("stance", sword) } }),
        );
    let mut editor = support::open(game, &scratch);
    let key = path("item", "blade");
    let grip = Grip {
        offset: LocalOffset([Meters(0.0), Meters(-0.25), Meters(0.0)]),
        turn: LocalTurn([Degrees(0.0), Degrees(180.0), Degrees(0.0)]),
    };

    assert!(item::wield::set_wield(
        &mut editor,
        &key,
        &path("stance", dagger),
        grip
    ));
    support::settled(&mut editor);
    let row = editor
        .read::<ItemDef>(&key)
        .expect("the item is in the project")
        .expect("the item reads");
    let reader = editor.reader().expect("a project");
    let Some(Wear::Hand {
        stance: held,
        grip: kept,
    }) = row.wear()
    else {
        panic!("the item is held");
    };
    assert_eq!(reader.path_of(*held), Some(path("stance", dagger).as_str()));
    assert_eq!(*kept, grip);

    assert!(stance::add_pair(
        &mut editor,
        &path("stance", dagger),
        &path("stance", dagger),
        &path("stance", dual),
    ));
    support::settled(&mut editor);
    let paired = editor
        .read::<StanceDef>(&path("stance", dagger))
        .expect("the stance is in the project")
        .expect("the stance reads");
    let reader = editor.reader().expect("a project");
    assert_eq!(
        paired
            .paired
            .iter()
            .map(|pairing| (
                reader.path_of(pairing.with),
                reader.path_of(pairing.becomes)
            ))
            .collect::<Vec<_>>(),
        vec![(
            Some(path("stance", dagger).as_str()),
            Some(path("stance", dual).as_str())
        )]
    );
}
