//! The desktop client's scenarios: the scenes a player meets, each played through the client's
//! own bindings at the places, spells and windows the game lends them.

use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::render::video::{BLOOM_TOGGLE, FULLSCREEN_TOGGLE, LIGHT_SHAFTS_TOGGLE};
use engine::core::units::{Degrees, Meters, Seconds};
use engine::systems::GameSettings;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;
use engine::systems::staging::Staging;

use super::ScenarioSlots;
use crate::bench::desktop::SCREEN;
use crate::bench::frames::Class;
use crate::bench::places;
use crate::bench::scenario::{
    Button, CameraOp, Cond, Ground, Point, Scenario, Step, Target, UiTarget,
};

/// The client's opening, measured from the player's entry into the world until the scenery
/// stands whole where they arrive: cold, on a profile no client has run the game on, so every
/// cache is built behind the loading screen; and warm, on the profile the cold start kept its
/// caches under.
pub const OPEN_COLD: &str = "open-cold";
pub const OPEN_WARM: &str = "open-warm";

/// Every scenario of the catalogue, in the order they run; with `smoke`, one short scenario that
/// proves the harness alone.
pub fn catalogue(slots: &ScenarioSlots, content: &Content, smoke: bool) -> Vec<Scenario> {
    let Some(home) = slots.home() else {
        return Vec::new();
    };
    let at = |place: Place| vec![Staging::Place { place }];
    let settled = || vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)];
    let low = || {
        vec![
            Step::Camera(CameraOp::Home),
            Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
            Step::Wait(Cond::Stood),
        ]
    };
    let top_down = || {
        vec![
            Step::Camera(CameraOp::Home),
            Step::Camera(CameraOp::Tilt(Degrees(90.0))),
            Step::Wait(Cond::Stood),
        ]
    };
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
    let crowd = slots.crowd_place.as_ref().map(|place| place.place());
    let mut scenarios = vec![
        scenario(
            "still",
            Class::Typical,
            at(home),
            top_down(),
            STILL,
            vec![Step::Wait(Cond::Seconds(STILL))],
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
            "tilt-sweep",
            Class::Typical,
            at(home),
            settled(),
            Scenario::LEAST,
            vec![
                Step::Camera(CameraOp::Tilt(Degrees(90.0))),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
                Step::Camera(CameraOp::Home),
            ],
        ),
        scenario(
            "low-pitch-sweep",
            Class::Busy,
            at(home),
            low(),
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
            at(home),
            settled(),
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
    if let Some(light) = &slots.light_place {
        scenarios.push(scenario(
            "still-light",
            Class::Light,
            at(light.place()),
            top_down(),
            STILL,
            vec![Step::Wait(Cond::Seconds(STILL))],
        ));
    }
    if let Some(to) = slots.walk_to {
        scenarios.push(scenario(
            "walk",
            Class::Typical,
            at(home),
            vec![
                Step::Camera(CameraOp::Home),
                Step::Camera(CameraOp::Face(to)),
                Step::Camera(CameraOp::Tilt(Degrees(-90.0))),
                Step::Wait(Cond::Stood),
            ],
            Scenario::LEAST,
            walk_and_back(to, home),
        ));
    }
    if let Some(to) = slots.far_walk_to {
        let mut script = Vec::new();
        for leg in places::legs(content, home, to) {
            script.push(Step::Stage(Staging::Walk { to: leg }));
            script.push(Step::Wait(Cond::AtRest));
        }
        script.push(Step::Stage(Staging::Place { place: home }));
        script.push(Step::Wait(Cond::Arrived(home)));
        script.push(Step::Wait(Cond::Stood));
        scenarios.push(scenario(
            "far-walk",
            Class::Busy,
            at(home),
            settled(),
            Scenario::LEAST,
            script,
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
            at(home),
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
        script.push(Step::DragItems);
        script.push(Step::SlotOptions);
        script.push(Step::DragWindows);
        script.push(Step::Key(InputAction::ToggleTerminal));
        script.push(Step::Wait(Cond::Settled));
        script.push(Step::HoverUi(UiTarget::Field));
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
    scenarios.push(scenario(
        "pointer-sweep",
        Class::Typical,
        at(crowd.unwrap_or(home)),
        settled(),
        Scenario::LEAST,
        vec![
            Step::Sweep(zigzag()),
            Step::Move(Point::Ground(Ground::NearMe {
                dx: Meters(0.0),
                dy: Meters(0.0),
            })),
            Step::Down(Button::Left),
            Step::Wait(Cond::Seconds(HELD)),
            Step::Up(Button::Left),
            Step::Wait(Cond::AtRest),
        ],
    ));
    scenarios.push(scenario(
        "click-to-move",
        Class::Typical,
        at(home),
        settled(),
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
                    Step::Wait(Cond::Seconds(CLICK_PACE)),
                    Step::Move(Point::Ground(Ground::NearMe {
                        dx: Meters(-3.0),
                        dy: Meters(0.0),
                    })),
                    Step::Click {
                        button: Button::Left,
                        times: 1,
                    },
                    Step::Wait(Cond::Seconds(CLICK_PACE)),
                ],
            },
            Step::Wait(Cond::AtRest),
        ],
    ));
    if let Some(crowd) = crowd {
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
    if let Some(talk) = &slots.talk_place {
        scenarios.push(scenario(
            "dialogue",
            Class::Typical,
            at(talk.place()),
            settled(),
            Scenario::LEAST,
            vec![
                Step::Stage(Staging::Talk),
                Step::Wait(Cond::Seconds(Seconds(5.0))),
                Step::Key(InputAction::Dismiss),
                Step::Wait(Cond::Seconds(Seconds(1.0))),
            ],
        ));
    }
    if !slots.spells.is_empty() {
        for (name, class, setup) in [
            ("weather-low", Class::Busy, low()),
            ("weather", Class::Typical, settled()),
        ] {
            let mut script = Vec::new();
            for spell in &slots.spells {
                script.push(Step::Stage(Staging::Weather {
                    spell: spell.clone(),
                }));
                script.push(Step::Wait(Cond::Seconds(SPELL)));
            }
            scenarios.push(scenario(
                name,
                class,
                at(home),
                setup,
                Scenario::LEAST,
                script,
            ));
        }
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
    let mut worst_stage = at(crowd.unwrap_or(home));
    if let Some(storm) = slots.spells.last() {
        worst_stage.push(Staging::Weather {
            spell: storm.clone(),
        });
    }
    let mut worst_setup = low();
    for &window in &slots.windows {
        worst_setup.push(Step::Key(window));
        worst_setup.push(Step::Wait(Cond::Settled));
    }
    scenarios.push(scenario(
        "worst-case",
        Class::Busy,
        worst_stage,
        worst_setup,
        Scenario::LEAST,
        vec![
            Step::Sweep(zigzag()),
            Step::Camera(CameraOp::Turn(Degrees(360.0))),
        ],
    ));
    let settings_open = || {
        vec![
            Step::Camera(CameraOp::Home),
            Step::Key(InputAction::ToggleSettings),
            Step::Wait(Cond::Settled),
            Step::Wait(Cond::Stood),
        ]
    };
    scenarios.push(scenario(
        "video-switching",
        Class::Typical,
        at(home),
        settings_open(),
        Scenario::LEAST,
        vec![
            Step::ClickUi(UiTarget::Text(BLOOM_TOGGLE.to_owned())),
            Step::Camera(CameraOp::Turn(Degrees(90.0))),
            Step::ClickUi(UiTarget::Text(LIGHT_SHAFTS_TOGGLE.to_owned())),
            Step::Camera(CameraOp::Turn(Degrees(90.0))),
            Step::ClickUi(UiTarget::Text(BLOOM_TOGGLE.to_owned())),
            Step::Camera(CameraOp::Turn(Degrees(90.0))),
            Step::ClickUi(UiTarget::Text(LIGHT_SHAFTS_TOGGLE.to_owned())),
            Step::Camera(CameraOp::Turn(Degrees(90.0))),
            Step::DragUi {
                from: UiTarget::Text(BLOOM_TOGGLE.to_owned()),
                to: UiTarget::Text(LIGHT_SHAFTS_TOGGLE.to_owned()),
            },
        ],
    ));
    scenarios.push(scenario(
        "resize",
        Class::Typical,
        at(home),
        settings_open(),
        Scenario::LEAST,
        vec![
            Step::Resize {
                width: SCREEN.0 * 3 / 4,
                height: SCREEN.1 * 3 / 4,
            },
            Step::Wait(Cond::Stood),
            Step::Wait(Cond::Seconds(Seconds(1.0))),
            Step::Resize {
                width: SCREEN.0,
                height: SCREEN.1,
            },
            Step::Wait(Cond::Stood),
            Step::Wait(Cond::Seconds(Seconds(1.0))),
            Step::ClickUi(UiTarget::Text(FULLSCREEN_TOGGLE.to_owned())),
            Step::Wait(Cond::Stood),
            Step::Wait(Cond::Seconds(Seconds(1.0))),
            Step::ClickUi(UiTarget::Text(FULLSCREEN_TOGGLE.to_owned())),
            Step::Wait(Cond::Stood),
            Step::Wait(Cond::Seconds(Seconds(1.0))),
        ],
    ));
    if content.built::<GameSettings>().daylight.is_some() {
        let mut script = Vec::new();
        for step in 0..DUSK_STEPS {
            script.push(Step::Stage(Staging::Day {
                hour: DUSK_FROM + step as f32 * DUSK_STEP,
                turning: true,
            }));
            script.push(Step::Wait(Cond::Seconds(DUSK_PAUSE)));
        }
        scenarios.push(scenario(
            "dusk",
            Class::Busy,
            at(home),
            low(),
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
    scenarios.push(empty_view(slots, home));
    if let Some(to) = slots.walk_to {
        scenarios.push(scenario(
            "soak",
            Class::Typical,
            at(home),
            vec![
                Step::Camera(CameraOp::Home),
                Step::Camera(CameraOp::Tilt(SOAK_TILT)),
                Step::Wait(Cond::Stood),
            ],
            SOAK,
            walk_and_back(to, home),
        ));
    }
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
        criteria: None,
        players: 0,
    }
}

fn orbit() -> Vec<Step> {
    vec![Step::Camera(CameraOp::Turn(Degrees(360.0)))]
}

/// A walk to `to`, and back home.
fn walk_and_back(to: Pos<Meters>, home: Place) -> Vec<Step> {
    vec![
        Step::Stage(Staging::Walk { to }),
        Step::Wait(Cond::AtRest),
        Step::Stage(Staging::Place { place: home }),
        Step::Wait(Cond::Arrived(home)),
        Step::Wait(Cond::Stood),
    ]
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
const STILL: Seconds = Seconds(12.0);
const SOAK: Seconds = Seconds(600.0);
/// A soak walks at a middling tilt, this far up from home.
const SOAK_TILT: Degrees = Degrees(-25.0);
/// How long a press on the ground is held.
const HELD: Seconds = Seconds(0.5);
/// Five clicks a second.
const CLICK_PACE: Seconds = Seconds(0.2);
/// The fights a combat scenario stages, each against the nearest foe left, and how long each
/// lasts: together at least a scenario's least time, so the script plays once and asks for no
/// more foes than this.
const FIGHTS: u32 = 3;
const FIGHT: Seconds = Seconds(4.0);
const SPELL: Seconds = Seconds(4.0);
/// Dusk, staged half an hour at a time from late afternoon into the night.
const DUSK_FROM: f32 = 17.0;
const DUSK_STEP: f32 = 0.5;
const DUSK_STEPS: u32 = 7;
const DUSK_PAUSE: Seconds = Seconds(1.5);
const PLAYERS: u32 = 50;
const TYPED: &str = "hello";
