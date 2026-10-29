use engine::core::content::{Content, ContentScope, with_content};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::probe::{ProbeGame, ProbeQuestion, answer};
use serde::de::DeserializeOwned;
use serde_json::json;
use testkit::{Game, Map, path};

#[test]
fn a_driver_finds_the_games_things_through_the_probe() {
    let content = Game::new()
        .area(
            "yard",
            &Map::of(&[
                "..........",
                "..........",
                "....##....",
                "....##....",
                "..........",
                "..........",
            ]),
            json!([
                { "type": "resident", "npc": path("npc", "keeper"), "at": { "x": 1.5, "y": 1.5 } },
            ]),
        )
        .hero("yard", 8.0, 1.0)
        .sword()
        .npc("keeper", 40.0, json!([]))
        .engine_rows()
        .load();
    let area = content
        .by_path::<AreaDef>(&path("area", "yard"))
        .expect("the yard");

    let game: ProbeGame = ask(&content, &ProbeQuestion::Game);
    assert_eq!(game.initial_spawn.at, Pos::new(8.0, 1.0));
    let yard = game
        .areas
        .iter()
        .find(|shown| shown.area == area)
        .expect("the yard");
    let keeper = &yard.residents[0];
    assert_eq!(keeper.npc.path(&content), path("npc", "keeper"));
    assert_eq!(keeper.at, Pos::new(1.5, 1.5));
    let npc = game
        .npcs
        .iter()
        .find(|npc| npc.npc == keeper.npc)
        .expect("the keeper among the npcs");
    assert_eq!(npc.name, "keeper");
    let sword = game
        .items
        .iter()
        .find(|item| item.item.path(&content) == path("item", "sword"))
        .expect("the sword among the items");
    assert_eq!(
        sword.stance.map(|stance| stance.path(&content).to_owned()),
        Some(path("stance", "sword"))
    );
    assert!(game.skills.iter().any(|skill| skill.name == "Strike"));
    assert!(game.spacing > Meters(0.0));

    let walkable: Vec<bool> = ask(
        &content,
        &ProbeQuestion::Batch {
            questions: [Pos::new(1.5, 1.5), Pos::new(4.5, 2.5)]
                .map(|at| ProbeQuestion::Walkable { area, at })
                .to_vec(),
        },
    );
    assert_eq!(walkable, [true, false]);
    let route: Option<Vec<Pos<Meters>>> = ask(
        &content,
        &ProbeQuestion::Route {
            area,
            from: Pos::new(2.0, 3.0),
            to: Pos::new(8.0, 3.0),
        },
    );
    let corners = route.expect("a way around the wall");
    assert_eq!(corners.last(), Some(&Pos::new(8.0, 3.0)));
    assert!(corners.len() > 1, "the way bends around the wall");
}

fn ask<Answer: DeserializeOwned>(content: &Content, question: &ProbeQuestion) -> Answer {
    let line = answer(content, question).expect("an answer without a frame");
    with_content(ContentScope::of(content), || serde_json::from_str(&line))
        .0
        .unwrap_or_else(|error| panic!("an unreadable answer {line}: {error}"))
}
