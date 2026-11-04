use editor::app::Editor;
use editor::project::ContentEdit;
use editor::views::area::climate;
use engine::core::units::Seconds;
use engine::systems::area::{AreaDef, Climate, NextSpell, Spell, SpellLength, Weather};

use crate::support::{self, area};

fn spell(name: &str, clouds: f32, next: &[&str]) -> Spell {
    Spell {
        name: name.to_owned(),
        weather: Weather {
            clouds,
            ..Weather::default()
        },
        lasts: SpellLength {
            shortest: Seconds(60.0),
            longest: Seconds(120.0),
        },
        next: next
            .iter()
            .map(|spell| NextSpell {
                spell: (*spell).to_owned(),
                weight: 1.0,
            })
            .collect(),
    }
}

fn climate_of(editor: &mut Editor) -> Climate {
    support::settled(editor);
    editor
        .read::<AreaDef>(&area())
        .expect("a project")
        .expect("the area reads")
        .climate
        .clone()
}

fn names(climate: &Climate) -> Vec<&str> {
    climate
        .spells
        .iter()
        .map(|spell| spell.name.as_str())
        .collect()
}

fn follows<'a>(climate: &'a Climate, name: &str) -> Vec<&'a str> {
    climate
        .spells
        .iter()
        .find(|spell| spell.name == name)
        .expect("the spell")
        .next
        .iter()
        .map(|next| next.spell.as_str())
        .collect()
}

#[test]
fn the_climate_page_keeps_the_spells_and_what_follows_each_in_step() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = support::open(support::game(), &scratch);
    assert!(
        editor.change::<AreaDef>(ContentEdit::new("Give the area weather"), &area(), |area| {
            area.climate.spells = vec![
                spell("Fair", 0.1, &["Showers"]),
                spell("Showers", 0.7, &["Fair", "Storm"]),
                spell("Storm", 1.0, &["Showers"]),
            ];
            Ok(())
        })
    );

    assert!(climate::move_spell(&mut editor, &area(), 2, 0));
    let moved = climate_of(&mut editor);
    assert_eq!(names(&moved), ["Storm", "Fair", "Showers"]);
    assert_eq!(
        moved.first().clouds,
        1.0,
        "the spell moved first is the weather a server starts on"
    );

    assert!(climate::rename_spell(&mut editor, &area(), 2, "Drizzle"));
    let renamed = climate_of(&mut editor);
    assert_eq!(names(&renamed), ["Storm", "Fair", "Drizzle"]);
    assert_eq!(
        follows(&renamed, "Fair"),
        ["Drizzle"],
        "a rename carries to what follows"
    );
    assert_eq!(follows(&renamed, "Storm"), ["Drizzle"]);
    assert!(
        !climate::rename_spell(&mut editor, &area(), 0, "Fair"),
        "two spells never share a name"
    );

    assert!(climate::add_spell(&mut editor, &area(), 1));
    let added = climate_of(&mut editor);
    assert_eq!(names(&added), ["Storm", "Fair", "Fair 2", "Drizzle"]);
    assert_eq!(
        added.spells[2].weather, added.spells[1].weather,
        "a copy to change"
    );

    assert!(climate::remove_spell(&mut editor, &area(), 3));
    let removed = climate_of(&mut editor);
    assert_eq!(names(&removed), ["Storm", "Fair", "Fair 2"]);
    assert!(
        follows(&removed, "Fair").is_empty(),
        "a spell removed follows nothing any more"
    );

    for _ in 0..2 {
        assert!(climate::remove_spell(&mut editor, &area(), 0));
    }
    assert!(
        !climate::remove_spell(&mut editor, &area(), 0),
        "a climate keeps one spell at least"
    );
    assert_eq!(names(&climate_of(&mut editor)), ["Fair 2"]);
}
