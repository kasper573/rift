use engine::core::content::{ContentRegistry, ContentRow};
use engine::systems::registry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::support::{TempFolder, load};

const SHIPPED: &str = include_str!("../../../example_game/game.json");

#[test]
fn the_shipped_folder_carries_the_version_of_this_build() {
    let game: serde_json::Value = serde_json::from_str(SHIPPED).expect("game.json");
    assert_eq!(
        game["content_version"].as_str(),
        Some(registry().version().0.as_str())
    );
}

#[test]
fn the_version_is_pinned() {
    assert_eq!(registry().version().0, "8c68e972");
}

#[test]
fn another_version_is_refused_last() {
    let folder = TempFolder::of_template();
    folder.edit("game.json", |game| {
        game["content_version"] = serde_json::Value::String("deadbeef".to_owned());
        game["initial_spawn"]["area"] = serde_json::Value::String("strat".to_owned());
    });

    let report = load(folder.path()).err().expect("refused");

    let rules: Vec<&str> = report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect();
    assert_eq!(rules.last(), Some(&"content version"));
    assert!(rules.contains(&"unknown row"), "{rules:?}");
    let version = report.errors.last().expect("an error");
    assert_eq!(version.problem.found, "\"deadbeef\"");
}

#[test]
fn only_a_changed_shape_moves_the_version() {
    assert_eq!(version_of::<Narrow>(), version_of::<SameShape>());
    assert_ne!(version_of::<Narrow>(), version_of::<Wide>());
    assert_ne!(version_of::<Narrow>(), version_of::<Retyped>());
    assert_eq!(version_of::<Narrow>().len(), 8);
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Narrow {
    name: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Wide {
    name: String,
    #[serde(default)]
    weight: u32,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Retyped {
    name: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SameShape {
    name: String,
}

impl ContentRow for Narrow {
    const TABLE: &'static str = "thing";
}
impl ContentRow for Wide {
    const TABLE: &'static str = "thing";
}
impl ContentRow for Retyped {
    const TABLE: &'static str = "thing";
}
impl ContentRow for SameShape {
    const TABLE: &'static str = "thing";
}

fn version_of<Row: ContentRow>() -> String {
    let mut registry = ContentRegistry::default();
    registry.row::<Row>();
    registry.version().0
}
