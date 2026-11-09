//! What a bench scenario is: a scene staged on the server, a script of the player's input the
//! driver plays through the client's own bindings, and the criteria its frames must meet. A
//! scenario names nothing of any game: the places and windows it plays with are
//! slots the game's own scenarios file fills, or that the engine fills from the content when the
//! game has none.

use engine::core::math::Pos;
use engine::core::units::{Degrees, Meters, Seconds};
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;
use engine::systems::staging::Staging;
use serde::{Deserialize, Serialize};

use super::frames::Class;

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: String,
    pub target: Target,
    pub class: Class,
    /// What the server stages for the player before the scenario starts.
    pub stage: Vec<Staging>,
    /// Played before the frames count, to bring the scene about.
    pub setup: Vec<Step>,
    /// How many frames of the script warm the scene up before the frames count.
    pub warm_up: u32,
    /// How long the script is played for, at least: it plays through to its end once this has
    /// passed.
    pub duration: Seconds,
    pub script: Vec<Step>,
    /// Other players connected to the server while it plays, each standing where the game's
    /// characters start.
    pub players: u32,
}

impl Scenario {
    /// How many frames warm a scene up before its frames count.
    pub const WARM_UP: u32 = 120;
    /// How long a scenario's script plays for at the least.
    pub const LEAST: Seconds = Seconds(5.0);
}

#[derive(Serialize, Deserialize, schemars::JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Client,
    Editor,
}

/// One thing the driver does, as a player would, each confirmed through the probe before the
/// next.
#[derive(Clone, Debug)]
pub enum Step {
    /// The wheel turned this many lines, up when positive.
    Wheel(i32),
    /// The key the action is bound to.
    Key(InputAction),
    /// Typed a key at a time, as a player types.
    Type(String),
    Wait(Cond),
    Repeat {
        times: u32,
        steps: Vec<Step>,
    },
    /// The pointer swept over every element of the interface in turn.
    SweepInterface,
    /// The pointer moved onto the field that takes typing.
    HoverField,
    /// Every window of the interface dragged a little way by its title, and back.
    DragWindows,
    /// The first item shown in a slot dragged onto the next, with the carry binding; nothing
    /// when fewer than two are shown.
    DragItems,
    /// The options of the first item shown in a slot opened by their binding, then dismissed;
    /// nothing when no slot is shown.
    SlotOptions,
    /// Everything open over the world dismissed, until nothing covers it.
    CloseWindows,
    Camera(CameraOp),
    /// Staged on the server mid-script.
    Stage(Staging),
    /// Walked into the nearest warp the client shows, into the area it leads to.
    CrossWarp,
}

#[derive(Clone, Debug)]
pub enum Cond {
    Seconds(Seconds),
    /// The player stands in the area at the place, and the screen is done fading.
    Arrived(Place),
    /// Everything the view takes in stands as wanted.
    Stood,
    /// The player has come to rest; a player that neither rests nor moves on fails the harness.
    AtRest,
    /// The interface has stopped moving.
    Settled,
}

#[derive(Clone, Copy, Debug)]
pub enum CameraOp {
    /// Turned by dragging with the turn binding until the yaw has come round by this much.
    Turn(Degrees),
    /// Tilted by dragging with the tilt binding until the pitch has changed by this much, down
    /// when positive, or as far as the camera tilts.
    Tilt(Degrees),
    /// Turned until the ground point lies straight ahead of the player on screen.
    Face(Pos<Meters>),
    Home,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    Other(u16),
}

impl Button {
    pub fn x_number(self) -> u16 {
        match self {
            Button::Left => 1,
            Button::Middle => 2,
            Button::Right => 3,
            Button::Back => 8,
            Button::Forward => 9,
            Button::Other(number) => number,
        }
    }
}
