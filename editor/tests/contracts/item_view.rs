use editor::views::item;
use engine::systems::item::{CARD_SIZE, ICON_SIZE};
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
