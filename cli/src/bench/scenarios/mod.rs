//! The catalogues of scenarios the benches play: the desktop client's, and the editor's. What
//! each scenario plays with of a game comes from the game's own scenarios file, or from the
//! content where the file leaves a slot empty.

pub mod client;
pub mod editor;

use std::path::Path;

use engine::core::content::{Content, ContentId};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::GameSettings;
use engine::systems::input::map::InputAction;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::script::Place;
use serde::{Deserialize, Serialize};

use super::places;

/// What a game lends its scenarios: the places they play at, the windows they open, and the
/// character they play. Read from `bench/scenarios.json` in the game's folder, every slot it
/// leaves empty filled in from the content.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ScenarioSlots {
    /// Where the scenarios play, and the area the editor opens.
    pub home: Option<Place>,
    /// Where foes crowd.
    pub crowd_place: Option<Place>,
    /// Ground the emptiest view from home looks toward.
    pub empty_view: Option<Pos<Meters>>,
    /// The actions that open the windows the interface scenarios open, each a toggle.
    pub windows: Vec<InputAction>,
    /// Ground a walk from home ends at, far enough off that new ground comes into view.
    pub walk_to: Option<Pos<Meters>>,
    /// The test character the scenarios play, else the game's first.
    pub character: Option<ContentId<CharacterSnapshot>>,
}

pub const SLOTS_FILE: &str = "bench/scenarios.json";

impl ScenarioSlots {
    /// The game's own slots from its scenarios file at `path`, every slot it leaves empty filled
    /// from the content, else slots filled from the content alone where there is no file.
    pub fn of(path: &Path, content: &Content) -> Result<ScenarioSlots, String> {
        let mut slots = if path.exists() {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let (slots, _) = engine::core::content::with_content(
                engine::core::content::ContentScope::of(content),
                || serde_json::from_str::<ScenarioSlots>(&text),
            );
            slots.map_err(|error| format!("{}: {error}", path.display()))?
        } else {
            ScenarioSlots::default()
        };
        slots.fill(content);
        Ok(slots)
    }

    /// Fills every empty slot from the content: where the player starts for home, every window
    /// toggle, and a walk from home across open ground.
    pub fn fill(&mut self, content: &Content) {
        let home = *self
            .home
            .get_or_insert(content.built::<GameSettings>().initial_spawn);
        if self.windows.is_empty() {
            self.windows = vec![
                InputAction::ToggleInventory,
                InputAction::ToggleEquipment,
                InputAction::ToggleStats,
                InputAction::ToggleQuestLog,
                InputAction::ToggleSkills,
                InputAction::ToggleHistory,
                InputAction::ToggleSettings,
            ];
        }
        if self.walk_to.is_none() {
            self.walk_to = places::walkable_toward(content, home, &WALKS);
        }
    }
}

/// How far the walk goes: half a minute of walking at the most.
const WALKS: [Meters; 3] = [Meters(120.0), Meters(90.0), Meters(60.0)];
