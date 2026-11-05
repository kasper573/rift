//! The catalogues of scenarios the benches play: the desktop client's, and the editor's. What
//! each scenario plays with of a game comes from the game's own scenarios file, or from the
//! content where the file leaves a slot empty.

pub mod client;
pub mod editor;

use std::path::Path;

use engine::core::content::{Content, ContentId};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaId;
use engine::systems::input::map::InputAction;
use engine::systems::notification::{NotificationDef, NotificationId};
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::script::Place;
use engine::systems::{GameSettings, area};
use serde::{Deserialize, Serialize};

use super::places;

/// What a game lends its scenarios: the places they play at, the spells they hold the weather
/// at, the windows they open, and the character they play. Read from `bench/scenarios.json` in
/// the game's folder, every slot it leaves empty filled in from the content.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ScenarioSlots {
    /// Where the scenarios play, the first the home they return to; teleport arrivals go to each
    /// in turn.
    pub places: Vec<NamedPlace>,
    /// A light scene, for the scenarios that have one.
    pub light_place: Option<NamedPlace>,
    /// Where someone stands with something to say.
    pub talk_place: Option<NamedPlace>,
    /// Where foes crowd.
    pub crowd_place: Option<NamedPlace>,
    /// Ground the emptiest view from home looks toward.
    pub empty_view: Option<Pos<Meters>>,
    /// Spells of the home area's climate, by name, in the order the weather scenarios hold them.
    pub spells: Vec<String>,
    /// The actions that open the windows the interface scenarios open, each a toggle.
    pub windows: Vec<InputAction>,
    /// Ground a walk of a minute's length ends at, from home.
    pub walk_to: Option<Pos<Meters>>,
    /// Ground a long walk across the home area's tiles ends at, from home.
    pub far_walk_to: Option<Pos<Meters>>,
    /// A notification the player is told, for the shot of a notice on screen.
    pub notification: Option<NotificationId>,
    /// The test character the scenarios play, else the game's first.
    pub character: Option<ContentId<CharacterSnapshot>>,
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

    /// Fills every empty slot from the content: the spawn and where the warps lead and the open
    /// middle of each other area for the places, the home area's spells, every window toggle,
    /// the game's first notification, and walks from home across open ground.
    pub fn fill(&mut self, content: &Content) {
        if self.places.is_empty() {
            let spawn = content.built::<GameSettings>().initial_spawn;
            self.places = places::arrivals(content, spawn)
                .into_iter()
                .map(|(name, place)| NamedPlace {
                    name,
                    area: place.area,
                    at: place.at,
                })
                .collect();
        }
        let Some(home) = self.home() else {
            return;
        };
        if self.spells.is_empty() {
            self.spells = area::load(content, home.area)
                .climate
                .spells
                .iter()
                .map(|spell| spell.name.clone())
                .collect();
        }
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
        if self.notification.is_none() {
            self.notification = content.ids::<NotificationDef>().next();
        }
        if self.walk_to.is_none() {
            self.walk_to = places::walkable_toward(content, home, &WALKS);
        }
        if self.far_walk_to.is_none() {
            self.far_walk_to = places::walkable_toward(content, home, &FAR_WALKS);
        }
    }

    pub fn home(&self) -> Option<Place> {
        self.places.first().map(NamedPlace::place)
    }
}

/// How far a walk across an area goes, the farthest first that reaches open ground.
const WALKS: [Meters; 3] = [Meters(60.0), Meters(40.0), Meters(20.0)];
/// How far a long walk goes: across several of the area's tiles.
const FAR_WALKS: [Meters; 3] = [Meters(1200.0), Meters(1000.0), Meters(800.0)];
