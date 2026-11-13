use editor::app::Editor;
use editor::project::ContentEdit;
use engine::systems::area::{AreaDef, AreaModule, TimeOfDay, TimeOfDayName};
use engine::systems::npc::NpcDef;
use engine::systems::script::condition;
use serde_json::json;
use testkit::{Game, Map, path};

use crate::support;

#[test]
fn a_time_of_day_renamed_or_taken_away_is_carried_into_everything_that_names_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = support::open(game_with_a_day(), &scratch);
    assert_eq!(named(&editor), ["Night", "Night"]);

    let renamed = retime(&mut editor, |times| {
        times[2].name = TimeOfDayName("Dark".to_owned());
    });
    assert_eq!(renamed, 2, "the baker's absence and the wolves' share");
    assert_eq!(named(&editor), ["Dark", "Dark"]);

    retime(&mut editor, |times| {
        times.remove(2);
    });
    assert_eq!(
        named(&editor),
        ["Dusk", "Dusk"],
        "what followed a time taken away follows the one before it"
    );
}

/// Changes the times of day in the game's settings as `change` does, carried into what names
/// them; how many places that changed.
fn retime(editor: &mut Editor, change: impl FnOnce(&mut Vec<TimeOfDay>)) -> usize {
    let project = editor.project.as_ref().expect("a project");
    let mut settings = (*project.settings().expect("the settings read")).clone();
    let cycle = settings.daylight.as_mut().expect("the game has a day");
    let before = cycle.times.clone();
    change(&mut cycle.times);
    let after = cycle.times.clone();
    let mut edit = ContentEdit::new("Edit the game");
    edit.write_settings(project, &settings)
        .expect("the settings write");
    let places = editor::day::carry_names(project, &mut edit, &before, &after);
    assert!(editor.apply(edit), "the game still loads once retimed");
    support::settled(editor);
    places
}

/// The time of day the baker is away for, and the one the wolves come out in.
fn named(editor: &Editor) -> [String; 2] {
    let project = editor.project.as_ref().expect("a project");
    let baker = project
        .read::<NpcDef>(&path("npc", "baker"))
        .expect("the baker reads");
    let away = baker.absence().expect("the baker is away at times").spans[0]
        .time
        .0
        .clone();
    let area = project
        .read::<AreaDef>(&support::area())
        .expect("the area reads");
    let wolves = nodes_of(&area)
        .find_map(|module| match module {
            AreaModule::Spawner(spawner) => spawner.spawns[0].when.clone(),
            _ => None,
        })
        .expect("the wolves follow the sky");
    let out = condition::times(wolves.condition())[0].0.clone();
    [away, out]
}

fn nodes_of(area: &AreaDef) -> impl Iterator<Item = &AreaModule> {
    let mut modules = Vec::new();
    engine::systems::area::scene::walk(&area.scene, &mut |_, node, _| {
        if let engine::systems::area::scene::SceneNode::Module(node) = node {
            modules.push(&node.module);
        }
    });
    modules.into_iter()
}

fn game_with_a_day() -> Game {
    let time = |name: &str, from: f32| json!({ "name": name, "from": from, "eases": 0.0, "color": [0, 0, 0], "icon": "icons/time.png" });
    Game::new()
        .area(
            support::AREA,
            &Map::open(30, 30),
            json!([{
                "type": "spawner",
                "spawns": [{
                    "type": "npc",
                    "npc": path("npc", "wolf"),
                    "when": { "type": "during", "time": "Night" },
                }],
                "count": 2,
                "respawn": 30.0,
            }]),
        )
        .hero(support::AREA, 1.0, 1.0)
        .asset("icons/time.png", testkit::png(1, 1))
        .edit_settings(|settings| {
            settings["daylight"] = json!({
                "day_length": 1200.0,
                "start": 12.0,
                "times": [time("Day", 6.0), time("Dusk", 18.0), time("Night", 21.0)],
            });
        })
        .npc("wolf", 20.0, json!([]))
        .npc(
            "baker",
            40.0,
            json!([{ "type": "absence", "spans": [{ "time": "Night" }] }]),
        )
        .engine_rows()
        .whole()
}
