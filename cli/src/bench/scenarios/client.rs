//! The desktop client's scenarios: the scenes a player meets, each played through the client's
//! own bindings at the places, spells and windows the game lends them.

use engine::core::units::{Degrees, Meters, Seconds};
use engine::systems::input::map::InputAction;
use engine::systems::staging::Staging;

use super::ScenarioSlots;
use crate::bench::frames::Class;
use crate::bench::scenario::{Button, CameraOp, Cond, Ground, Point, Scenario, Step, Target};

/// Every scenario of the catalogue, in the order they run; with `smoke`, one short scenario that
/// proves the harness alone.
pub fn catalogue(slots: &ScenarioSlots, smoke: bool) -> Vec<Scenario> {
    let Some(home) = slots.home() else {
        return Vec::new();
    };
    let at_home = vec![Staging::Place { place: home }];
    if smoke {
        return vec![Scenario {
            name: "smoke".to_owned(),
            target: Target::Client,
            class: Class::Typical,
            stage: at_home.clone(),
            setup: vec![Step::Camera(CameraOp::Home)],
            warm_up: 0,
            duration: SMOKE,
            script: vec![Step::Wait(Cond::Seconds(SMOKE))],
            criteria: None,
        }];
    }
    let mut scenarios = vec![
        scenario(
            "still",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Seconds(12.0),
            vec![Step::Wait(Cond::Seconds(Seconds(12.0)))],
        ),
        scenario(
            "orbit",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Scenario::LEAST,
            vec![Step::Camera(CameraOp::Turn(Degrees(360.0)))],
        ),
        scenario(
            "tilt-sweep",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Scenario::LEAST,
            vec![
                Step::Camera(CameraOp::Tilt(Degrees(90.0))),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
            ],
        ),
        scenario(
            "low-pitch-sweep",
            Class::Busy,
            at_home.clone(),
            vec![
                Step::Camera(CameraOp::Home),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
                Step::Wait(Cond::Stood),
            ],
            Scenario::LEAST,
            vec![
                Step::Camera(CameraOp::Turn(Degrees(360.0))),
                Step::Camera(CameraOp::Zoom(2)),
                Step::Camera(CameraOp::Turn(Degrees(360.0))),
                Step::Camera(CameraOp::Zoom(-2)),
                Step::Camera(CameraOp::Turn(Degrees(360.0))),
            ],
        ),
        scenario(
            "zoom-flicks",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Scenario::LEAST,
            vec![
                Step::Camera(CameraOp::Zoom(1)),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
                Step::Camera(CameraOp::Zoom(-2)),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
                Step::Camera(CameraOp::Zoom(2)),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
                Step::Camera(CameraOp::Zoom(-2)),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
                Step::Camera(CameraOp::Zoom(2)),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
                Step::Camera(CameraOp::Home),
            ],
        ),
    ];
    if let Some(to) = slots.walk_to {
        scenarios.push(scenario(
            "walk",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Scenario::LEAST,
            vec![
                Step::Stage(Staging::Walk { to }),
                Step::Wait(Cond::AtRest),
                Step::Stage(Staging::Place { place: home }),
                Step::Wait(Cond::Arrived(home)),
                Step::Wait(Cond::AtRest),
            ],
        ));
    }
    if slots.places.len() > 1 {
        let mut script = Vec::new();
        for place in &slots.places {
            script.push(Step::Stage(Staging::Place {
                place: place.place(),
            }));
            script.push(Step::Wait(Cond::Arrived(place.place())));
            script.push(Step::Wait(Cond::Stood));
            script.push(Step::Wait(Cond::Seconds(Seconds(1.0))));
        }
        let mut arrivals = scenario(
            "arrivals",
            Class::Busy,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home)],
            Scenario::LEAST,
            script,
        );
        arrivals.warm_up = 0;
        scenarios.push(arrivals);
    }
    if !slots.windows.is_empty() {
        let mut script = Vec::new();
        for &window in &slots.windows {
            script.push(Step::Key(window));
            script.push(Step::Wait(Cond::Settled));
        }
        script.push(Step::SweepInterface);
        for &window in slots.windows.iter().rev() {
            script.push(Step::Key(window));
            script.push(Step::Wait(Cond::Settled));
        }
        scenarios.push(scenario(
            "ui-storm",
            Class::Typical,
            at_home.clone(),
            vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
            Scenario::LEAST,
            script,
        ));
    }
    scenarios.push(scenario(
        "pointer-sweep",
        Class::Typical,
        at_home.clone(),
        vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
        Scenario::LEAST,
        vec![Step::Sweep(zigzag())],
    ));
    scenarios.push(scenario(
        "click-to-move",
        Class::Typical,
        at_home.clone(),
        vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
        Scenario::LEAST,
        vec![
            Step::Repeat {
                times: 5,
                steps: vec![
                    Step::Move(Point::Ground(Ground::NearMe {
                        dx: Meters(3.0),
                        dy: Meters(0.0),
                    })),
                    Step::Click {
                        button: Button::Left,
                        times: 1,
                    },
                    Step::Wait(Cond::Frames(12)),
                    Step::Move(Point::Ground(Ground::NearMe {
                        dx: Meters(-3.0),
                        dy: Meters(0.0),
                    })),
                    Step::Click {
                        button: Button::Left,
                        times: 1,
                    },
                    Step::Wait(Cond::Frames(12)),
                ],
            },
            Step::Wait(Cond::AtRest),
        ],
    ));
    if !slots.spells.is_empty() {
        for (pitch, class) in [
            (Degrees(-90.0), Class::Busy),
            (Degrees(0.0), Class::Typical),
        ] {
            let mut script = Vec::new();
            for spell in &slots.spells {
                script.push(Step::Stage(Staging::Weather {
                    spell: spell.clone(),
                }));
                script.push(Step::Wait(Cond::Seconds(Seconds(4.0))));
            }
            let name = if pitch.0 < 0.0 {
                "weather-low"
            } else {
                "weather"
            };
            scenarios.push(scenario(
                name,
                class,
                at_home.clone(),
                vec![
                    Step::Camera(CameraOp::Home),
                    Step::Camera(CameraOp::Tilt(pitch)),
                    Step::Wait(Cond::Stood),
                ],
                Scenario::LEAST,
                script,
            ));
        }
    }
    scenarios.push(scenario(
        "dialogue",
        Class::Typical,
        at_home.clone(),
        vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
        Scenario::LEAST,
        vec![
            Step::Stage(Staging::Talk),
            Step::Wait(Cond::Seconds(Seconds(5.0))),
            Step::Key(InputAction::Dismiss),
            Step::Wait(Cond::Seconds(Seconds(1.0))),
        ],
    ));
    scenarios.push(scenario(
        "combat",
        Class::Busy,
        at_home,
        vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)],
        Scenario::LEAST,
        vec![
            Step::Stage(Staging::Fight),
            Step::Wait(Cond::Seconds(Seconds(10.0))),
        ],
    ));
    scenarios
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
        criteria: None,
    }
}

/// Back and forth across the window, a little lower each pass.
fn zigzag() -> Vec<Point> {
    let mut through = Vec::new();
    for row in 0..8 {
        let down = 0.1 + 0.8 * row as f32 / 7.0;
        let (from, to) = if row % 2 == 0 {
            (0.05, 0.95)
        } else {
            (0.95, 0.05)
        };
        for step in 0..=20 {
            let share = step as f32 / 20.0;
            through.push(Point::Screen {
                across: from + (to - from) * share,
                down,
            });
        }
    }
    through
}

const SMOKE: Seconds = Seconds(3.0);
