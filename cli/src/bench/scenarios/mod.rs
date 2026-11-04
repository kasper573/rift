//! The catalogues of scenarios the benches play: the desktop client's, and the editor's. What
//! each scenario plays with of a game comes from the game's own scenarios file, or from the
//! content when the game has none.

pub mod client;

use std::path::Path;

use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaId;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;
use engine::systems::{GameSettings, area};
use serde::{Deserialize, Serialize};

use super::places;

/// What a game lends its scenarios: the places they play at, the spells they hold the weather
/// at, the windows they open, and the items they handle. Read from `bench/scenarios.json` in the
/// game's folder, or filled in from the content.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ScenarioSlots {
    /// Where the scenarios play, the first the home they return to; teleport arrivals go to each
    /// in turn.
    pub places: Vec<NamedPlace>,
    /// A light scene, for the scenarios that have one.
    pub light_place: Option<NamedPlace>,
    /// Spells of the home area's climate, by name, in the order the weather scenarios hold them.
    pub spells: Vec<String>,
    /// The actions that open the windows the interface scenarios open, each a toggle.
    pub windows: Vec<InputAction>,
    /// Ground a walk of a minute's length ends at, from home.
    pub walk_to: Option<Pos<Meters>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct NamedPlace {
    pub name: String,
    pub area: AreaId,
    pub at: Pos<Meters>,
}

impl NamedPlace {
    pub fn place(&self) -> Place {
        Place {
            area: self.area,
            at: self.at,
        }
    }
}

pub const SLOTS_FILE: &str = "bench/scenarios.json";

impl ScenarioSlots {
    /// The game's own slots where it keeps a scenarios file, else slots filled from its content.
    pub fn of(game: &Path, content: &Content) -> Result<ScenarioSlots, String> {
        let path = game.join(SLOTS_FILE);
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let (slots, _) = engine::core::content::with_content(
                engine::core::content::ContentScope::of(content),
                || serde_json::from_str::<ScenarioSlots>(&text),
            );
            return slots.map_err(|error| format!("{}: {error}", path.display()));
        }
        Ok(ScenarioSlots::generated(content))
    }

    /// Slots filled from the content alone: the spawn, where the warps lead, the open middle of
    /// each other area, the spawn area's spells and every window toggle.
    pub fn generated(content: &Content) -> ScenarioSlots {
        let spawn = content.built::<GameSettings>().initial_spawn;
        let places: Vec<NamedPlace> = places::arrivals(content, spawn)
            .into_iter()
            .map(|(name, place)| NamedPlace {
                name,
                area: place.area,
                at: place.at,
            })
            .collect();
        let spells = area::load(content, spawn.area)
            .climate
            .spells
            .iter()
            .map(|spell| spell.name.clone())
            .collect();
        ScenarioSlots {
            places,
            light_place: None,
            spells,
            windows: vec![
                InputAction::ToggleInventory,
                InputAction::ToggleEquipment,
                InputAction::ToggleStats,
                InputAction::ToggleQuestLog,
                InputAction::ToggleSkills,
                InputAction::ToggleHistory,
                InputAction::ToggleSettings,
            ],
            walk_to: places::walkable_toward(content, spawn, &WALKS),
        }
    }

    pub fn home(&self) -> Option<Place> {
        self.places.first().map(NamedPlace::place)
    }
}

/// How far a walk across an area goes, the farthest first that reaches open ground.
const WALKS: [Meters; 3] = [Meters(60.0), Meters(40.0), Meters(20.0)];
