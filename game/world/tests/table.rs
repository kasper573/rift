//! The content-table generic: a typed [`Id`] resolves a row by its string id, indexes back to the
//! row, and compares purely by index — independent of the row type. Exercised against a local table
//! so the contract holds for any game's content, not just rift's. The real loaders (and their
//! serde by-name wiring) are covered by the `assets` and `sim` tests.

use world::table::{Content, Id};

struct Color {
    name: &'static str,
}

static COLORS: [Color; 3] = [
    Color { name: "red" },
    Color { name: "green" },
    Color { name: "blue" },
];

impl Content for Color {
    fn table() -> &'static [Color] {
        &COLORS
    }
    fn id(&self) -> &str {
        self.name
    }
}

#[test]
fn by_name_resolves_a_known_id_and_get_returns_its_row() {
    let green = Id::<Color>::by_name("green").expect("green is in the table");
    assert_eq!(green.index(), 1);
    assert_eq!(green.get().name, "green");
}

#[test]
fn by_name_is_none_for_an_unknown_id() {
    assert!(Id::<Color>::by_name("purple").is_none());
}

#[test]
fn ids_compare_and_default_by_index_independent_of_the_row() {
    let red = Id::<Color>::by_name("red").expect("red is in the table");
    let blue = Id::<Color>::by_name("blue").expect("blue is in the table");
    assert!(red < blue);
    assert_eq!(red, Id::<Color>::new(0));
    assert_eq!(Id::<Color>::default(), red);
}
