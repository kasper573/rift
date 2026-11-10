//! The desktop client's scenarios: the scenes a player meets, each played through the client's
//! own bindings at the places and windows the game lends them.

use engine::core::content::Content;
use engine::core::units::{Degrees, Seconds};
use engine::systems::GameSettings;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;
use engine::systems::staging::Staging;

use super::ScenarioSlots;
use crate::bench::frames::Class;
use crate::bench::places;
use crate::bench::scenario::{CameraOp, Cond, Scenario, Step, Target};

/// The client's opening, measured from the player's entry into the world until the scenery
/// stands whole where they arrive, on a profile no client has run the game on, so every cache is
/// built behind the loading screen.
pub const OPENING: &str = "open";

/// Every scenario of the catalogue, in the order they run; with `smoke`, one short scenario that
/// proves the harness alone.
pub fn catalogue(slots: &ScenarioSlots, content: &Content, smoke: bool) -> Vec<Scenario> {
    let Some(home) = slots.home else {
        return Vec::new();
    };
    let at = |place: Place| vec![Staging::Place { place }];
    let settled = || vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)];
    if smoke {
        let mut scenario = scenario(
            "smoke",
            Class::Typical,
            at(home),
            vec![Step::Camera(CameraOp::Home)],
            SMOKE,
            vec![Step::Wait(Cond::Seconds(SMOKE))],
        );
        scenario.warm_up = 0;
        return vec![scenario];
    }
    let mut scenarios = vec![
        scenario(
            "still",
            Class::Typical,
            at(home),
            vec![
                Step::Camera(CameraOp::Home),
                Step::Camera(CameraOp::Tilt(Degrees(90.0))),
                Step::Wait(Cond::Stood),
            ],
            Scenario::LEAST,
            vec![Step::Wait(Cond::Seconds(Scenario::LEAST))],
        ),
        scenario(
            "orbit",
            Class::Typical,
            at(home),
            settled(),
            Scenario::LEAST,
            orbit(),
        ),
        scenario(
            "horizon",
            Class::Busy,
            at(home),
            vec![
                Step::Camera(CameraOp::Home),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
                Step::Wait(Cond::Stood),
            ],
            Scenario::LEAST,
            vec![Step::Wait(Cond::Seconds(Scenario::LEAST))],
        ),
    ];
    if let Some(to) = slots.walk_to {
        let mut script = Vec::new();
        for leg in places::legs(content, home, to) {
            script.push(Step::Stage(Staging::Walk { to: leg }));
            script.push(Step::Wait(Cond::AtRest));
        }
        script.push(Step::Stage(Staging::Place { place: home }));
        script.push(Step::Wait(Cond::Arrived(home)));
        script.push(Step::Wait(Cond::Stood));
        scenarios.push(scenario(
            "walk",
            Class::Busy,
            at(home),
            settled(),
            Scenario::LEAST,
            script,
        ));
    }
    if let Some(beside) = places::beside_warp(content, home.area) {
        scenarios.push(scenario(
            "crossings",
            Class::Busy,
            at(beside),
            settled(),
            Scenario::LEAST,
            vec![
                Step::CrossWarp,
                Step::Wait(Cond::Stood),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
                Step::Stage(Staging::Place { place: beside }),
                Step::Wait(Cond::Arrived(beside)),
                Step::Wait(Cond::Stood),
            ],
        ));
    }
    if let Some(crowd) = slots.crowd_place {
        scenarios.push(scenario(
            "combat",
            Class::Busy,
            at(crowd),
            settled(),
            Scenario::LEAST,
            vec![Step::Repeat {
                times: FIGHTS,
                steps: vec![
                    Step::Stage(Staging::Fight),
                    Step::Wait(Cond::Seconds(FIGHT)),
                ],
            }],
        ));
    }
    if !slots.windows.is_empty() {
        let mut script = Vec::new();
        for &window in &slots.windows {
            script.push(Step::Key(window));
            script.push(Step::Wait(Cond::Settled));
        }
        script.push(Step::SweepInterface);
        script.push(Step::DragItems);
        script.push(Step::SlotOptions);
        script.push(Step::DragWindows);
        script.push(Step::Key(InputAction::ToggleTerminal));
        script.push(Step::Wait(Cond::Settled));
        script.push(Step::HoverField);
        script.push(Step::Wheel(3));
        script.push(Step::Wheel(-3));
        script.push(Step::Type(TYPED.to_owned()));
        script.push(Step::Key(InputAction::SubmitText));
        script.push(Step::Wait(Cond::Settled));
        script.push(Step::CloseWindows);
        scenarios.push(scenario(
            "ui-storm",
            Class::Typical,
            at(home),
            settled(),
            Scenario::LEAST,
            script,
        ));
    }
    let spawn = content.built::<GameSettings>().initial_spawn;
    let mut players = scenario(
        "players",
        Class::Busy,
        at(spawn),
        settled(),
        Scenario::LEAST,
        orbit(),
    );
    players.players = PLAYERS;
    scenarios.push(players);
    scenarios
}

/// The emptiest view from `home`: low over the ground the slots name as emptiest, else over
/// whatever lies ahead.
pub fn empty_view(slots: &ScenarioSlots, home: Place) -> Scenario {
    let mut setup = vec![Step::Camera(CameraOp::Home)];
    if let Some(toward) = slots.empty_view {
        setup.push(Step::Camera(CameraOp::Face(toward)));
    }
    setup.push(Step::Camera(CameraOp::Tilt(Degrees(-90.0))));
    setup.push(Step::Wait(Cond::Stood));
    scenario(
        "empty-view",
        Class::Light,
        vec![Staging::Place { place: home }],
        setup,
        Scenario::LEAST,
        vec![Step::Wait(Cond::Seconds(Scenario::LEAST))],
    )
}

fn scenario(
    name: &str,
    class: Class,
    stage: Vec<Staging>,
    setup: Vec<Step>,
    duration: Seconds,
    script: Vec<Step>,
) -> Scenario {
    Scenario {
        name: name.to_owned(),
        target: Target::Client,
        class,
        stage,
        setup,
        warm_up: Scenario::WARM_UP,
        duration,
        script,
        players: 0,
    }
}

fn orbit() -> Vec<Step> {
    vec![Step::Camera(CameraOp::Turn(Degrees(360.0)))]
}

const SMOKE: Seconds = Seconds(3.0);
/// The fights a combat scenario stages, each against the nearest foe left, and how long each
/// lasts: together at least a scenario's least time, so the script plays once and asks for no
/// more foes than this.
const FIGHTS: u32 = 2;
const FIGHT: Seconds = Seconds(3.0);
const PLAYERS: u32 = 50;
const TYPED: &str = "hello";
