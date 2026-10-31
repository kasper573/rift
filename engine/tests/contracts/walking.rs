use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::schedule::IntoScheduleConfigs;
use engine::core::math::{Offset, Pos};
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::{Action, Actor};
use engine::systems::movement::{self, AGENT_RADIUS, HeardMove, MoveRequest, MoveSeq, position};
use engine::systems::player::CommandLock;
use serde_json::{Value, json};
use strum::VariantArray;
use testkit::{BODY, Game, Map, RUN_PACE, SPRINT_PACE, Sim, WALK_PACE, path, rig_clips};

const TICKS: usize = 240;

#[test]
fn asking_for_a_point_further_along_the_line_walks_on_as_if_asked_for_it_at_first() {
    for end in [
        Offset::new(8.0, 0.0),
        Offset::new(0.0, -8.0),
        Offset::new(-7.0, 7.0),
    ] {
        let once = Walk::start().toward(end).trail();
        let mut walk = Walk::start().toward(end * 0.375);
        let mut ticks = 0;
        let extended = walk.trail_while(|walk| {
            ticks += 1;
            if ticks == 12 {
                walk.ask(end);
            }
        });

        assert_same_trail(&extended, &once, &format!("walking toward {end:?}"));
    }
}

#[test]
fn asking_again_or_for_every_point_along_a_walk_as_it_goes_walks_it_as_one() {
    let straight = [
        Offset::new(8.0, 0.0),
        Offset::new(0.0, -8.0),
        Offset::new(-7.0, 7.0),
    ];
    let oblique = [
        Offset::new(8.0, 3.0),
        Offset::new(-3.0, 7.0),
        Offset::new(5.0, -6.0),
    ];
    for end in straight {
        let once = Walk::start().toward(end).trail();
        let again = Walk::start().toward(end).trail_while(|walk| walk.ask(end));
        assert_same_trail(&again, &once, &format!("asking again toward {end:?}"));
        for ahead in 1..=3 {
            let once = Walk::start().toward(end).trail();
            assert_same_trail(
                &spam(&once, ahead),
                &once,
                &format!("{ahead} ahead toward {end:?}"),
            );
        }
    }
    for end in oblique {
        let once = Walk::start().toward(end).trail();
        assert_same_trail(&spam(&once, 1), &once, &format!("toward {end:?}"));
    }
}

#[test]
fn asking_for_points_further_along_a_walk_as_it_goes_never_holds_it_up() {
    for end in [
        Offset::new(8.0, 3.0),
        Offset::new(-3.0, 7.0),
        Offset::new(5.0, -6.0),
    ] {
        for ahead in 2..=3 {
            let once = Walk::start().toward(end).trail();
            let spammed = spam(&once, ahead);
            let arrived = arrival(&once);

            assert_eq!(arrival(&spammed), arrived, "{ahead} ahead toward {end:?}");
            assert_eq!(spammed.last(), Some(&end));
            assert!(
                spammed[..=arrived]
                    .windows(2)
                    .all(|pair| pair[0] != pair[1]),
                "{ahead} ahead toward {end:?} paused on the way"
            );
        }
    }
}

#[test]
fn a_point_on_the_way_ends_the_walk_there() {
    let end = Offset::new(8.0, 3.0);
    let once = Walk::start().toward(end).trail();
    let stop = waypoints(end)[4];
    let mut walk = Walk::start().toward(end);
    let mut ticks = 0;
    let cut = walk.trail_while(|walk| {
        ticks += 1;
        if ticks == 5 {
            walk.ask(stop);
        }
    });

    let at_stop = |at: &Offset<Meters>| (*at - stop).length() < 1e-3;
    let arrived = cut
        .iter()
        .position(at_stop)
        .expect("the walk to reach the point on the way");
    assert_same_trail(&cut[..arrived], &once[..arrived], "up to the stop");
    assert!(cut[arrived..].iter().all(at_stop));
}

#[test]
fn a_goal_that_drifts_a_little_as_the_walk_goes_is_walked_on_to_without_a_pause() {
    let end = Offset::new(8.0, 0.0);
    let drifted = Offset::new(8.0, 0.4);
    let mut walk = Walk::start().toward(end);
    let mut ticks = 0;
    let trail = walk.trail_while(|walk| {
        ticks += 1;
        if ticks == 10 {
            walk.ask(drifted);
        }
    });

    assert_arrives(&trail, drifted, "a goal drifted a little");
    let arrived = arrival(&trail);
    assert!(
        trail[..=arrived].windows(2).all(|pair| pair[0] != pair[1]),
        "paused on the way to the drifted goal"
    );
}

#[test]
fn turning_back_mid_walk_turns_at_once() {
    let mut walk = Walk::start().toward(Offset::new(6.0, 0.0));
    walk.ticks(3);
    let before = walk.at();
    assert!(before.x > 0.0 && before.x < 1.0, "on the way: {before:?}");

    walk.ask(Offset::new(-6.0, 0.0));
    walk.ticks(1);

    assert!(walk.at().x < before.x);
    assert_eq!(walk.at().y, 0.0);
}

#[test]
fn a_walk_over_open_ground_keeps_to_the_straight_line() {
    for end in [
        Offset::new(7.0, 3.0),
        Offset::new(-2.0, 8.0),
        Offset::new(6.0, -5.0),
    ] {
        assert_keeps_to_the_line(&Walk::start().toward(end).trail(), end);
    }
}

/// A walk keeps a body out of what blocks, give or take a tenth of its radius, and through a
/// notch the body fits with that much to spare.
#[test]
fn a_walk_goes_round_what_blocks_and_through_a_notch_a_body_fits() {
    const GIVE: f32 = AGENT_RADIUS.0 / 10.0;
    let boulder = Walk::among(
        json!({ "type": "circle", "radius": 1.0 }),
        json!([{ "type": "prop", "prop": path("prop", "block"), "at": { "x": 7.0, "y": 7.0 } }]),
        Pos::new(2.0, 7.0),
    )
    .toward(Offset::new(10.0, 0.0))
    .trail();
    assert_arrives(&boulder, Offset::new(10.0, 0.0), "round the boulder");
    let center = Pos::new(7.0, 7.0);
    for at in &boulder {
        let gap = (Pos::new(2.0, 7.0) + *at).distance(center);
        assert!(
            gap >= Meters(1.0 + AGENT_RADIUS.0 - GIVE),
            "{at:?} is {gap:?} from the boulder's middle"
        );
    }

    let edge = Offset::new(5.0 - 1.0 - AGENT_RADIUS.0 + GIVE, 0.0);
    let hair_off = Walk::among(
        json!({ "type": "circle", "radius": 1.0 }),
        json!([{ "type": "prop", "prop": path("prop", "block"), "at": { "x": 7.0, "y": 7.0 } }]),
        Pos::new(2.0, 7.0),
    )
    .toward(edge)
    .trail();
    let last = hair_off.last().expect("a trail");
    assert!(
        (*last - edge).length() < 0.05,
        "asked for a hair past the boulder's edge, ended at {last:?}"
    );

    let wall = |gap: f32| {
        let width = 6.0 - gap / 2.0;
        let footprint = json!({ "type": "box", "width": width, "depth": 0.5 });
        let walls = json!([
            { "type": "prop", "prop": path("prop", "block"), "at": { "x": 1.0 + width / 2.0, "y": 7.0 } },
            { "type": "prop", "prop": path("prop", "block"), "at": { "x": 13.0 - width / 2.0, "y": 7.0 } },
        ]);
        Walk::among(footprint, walls, Pos::new(7.0, 3.0))
            .toward(Offset::new(0.0, 8.0))
            .trail()
    };
    let round_the_end = |trail: &[Offset<Meters>]| {
        trail
            .iter()
            .any(|at| at.x + 7.0 <= 1.0 || at.x + 7.0 >= 13.0)
    };
    let through = wall(2.0 * AGENT_RADIUS.0 + 0.3);
    assert_arrives(&through, Offset::new(0.0, 8.0), "through the notch");
    assert!(!round_the_end(&through), "went round the wall's end");
    let around = wall(2.0 * AGENT_RADIUS.0 - 0.1);
    assert_arrives(&around, Offset::new(0.0, 8.0), "round the wall");
    assert!(
        round_the_end(&around),
        "squeezed through a notch too narrow"
    );
}

#[test]
fn the_server_owns_up_to_the_latest_move_asked_of_it_heeded_or_not() {
    let mut walk = Walk::start();
    walk.ask(Offset::new(3.0, 0.0));
    walk.ask(Offset::new(4.0, 0.0));
    walk.ticks(1);
    walk.sim.world.entity_mut(walk.player).insert(CommandLock);
    walk.ask(Offset::new(-4.0, 0.0));
    walk.ticks(1);

    assert_eq!(walk.heard(), HeardMove(MoveSeq(3)));
}

#[test]
fn a_walker_covers_ground_in_the_gait_whose_clip_covers_nearest_its_speed() {
    let gaits = [Action::Walk, Action::Run, Action::Sprint];
    let no_sprint = json!({
        "base": { "clips": Action::VARIANTS
            .iter()
            .filter(|&&action| action != Action::Sprint)
            .map(|action| (action.as_ref(), testkit::clip(action.as_ref())))
            .collect::<BTreeMap<_, _>>() },
        "paces": [
            { "clip": testkit::clip("walk"), "pace": WALK_PACE.0.0 },
            { "clip": testkit::clip("run"), "pace": RUN_PACE.0.0 },
        ],
    });
    for (speed, sprints, gait) in [
        (WALK_PACE.0.0 * 0.8, true, Action::Walk),
        (RUN_PACE.0.0 * 0.85, true, Action::Run),
        (SPRINT_PACE.0.0 * 0.85, true, Action::Sprint),
        (SPRINT_PACE.0.0 * 0.85, false, Action::Run),
    ] {
        let fields = if sprints {
            json!({})
        } else {
            no_sprint.clone()
        };
        let mut walk = Walk::on(
            Game::new()
                .area("field", &Map::open(19, 19), json!([]))
                .skeleton_of(BODY, &rig_clips(), fields)
                .hero("field", 2.0, 9.0)
                .edit("player", "hero", |hero| {
                    hero["stats"]["movement_speed"] = json!(speed)
                }),
        )
        .toward(Offset::new(14.0, 0.0));
        let mut seen = Vec::new();
        for _ in 0..20 {
            walk.sim.tick();
            let action = walk
                .sim
                .world
                .get::<Actor>(walk.player)
                .expect("the walker")
                .action;
            if gaits.contains(&action) && !seen.contains(&action) {
                seen.push(action);
            }
        }
        assert_eq!(
            seen,
            vec![gait],
            "at {speed} m/s, with{} a sprint, the walker covers ground in one gait",
            if sprints { "" } else { "out" }
        );
    }
}

struct Walk {
    sim: Sim,
    conn: Entity,
    player: Entity,
    start: Pos<Meters>,
    asked: MoveSeq,
}

impl Walk {
    fn start() -> Walk {
        Walk::on(
            Game::new()
                .area("field", &Map::open(19, 19), json!([]))
                .hero("field", 9.0, 9.0),
        )
    }

    /// A walk on a field where props with `footprint` stand as `placed`.
    fn among(footprint: Value, placed: Value, from: Pos<Meters>) -> Walk {
        Walk::on(
            Game::new()
                .prop("block", footprint)
                .area("field", &Map::open(15, 15), placed)
                .hero("field", from.x, from.y),
        )
    }

    fn on(game: Game) -> Walk {
        let content = game.load();
        let mut sim = Sim::new(content);
        sim.serve("field")
            .add((movement::move_request, movement::advance).chain());
        let player = sim.join(1);
        let conn = sim.conn(1);
        let start = position(&sim.world, player).expect("the player");
        Walk {
            sim,
            conn,
            player,
            start,
            asked: MoveSeq::default(),
        }
    }

    fn toward(mut self, to: Offset<Meters>) -> Walk {
        self.ask(to);
        self
    }

    fn ask(&mut self, to: Offset<Meters>) {
        self.asked = MoveSeq(self.asked.0 + 1);
        let pos = self.start + to;
        self.sim.send(
            self.conn,
            MoveRequest {
                pos,
                seq: self.asked,
            },
        );
    }

    fn ticks(&mut self, count: usize) {
        for _ in 0..count {
            self.sim.tick();
        }
    }

    fn at(&mut self) -> Offset<Meters> {
        position(&self.sim.world, self.player).expect("the player") - self.start
    }

    fn heard(&mut self) -> HeardMove {
        *self
            .sim
            .world
            .get::<HeardMove>(self.player)
            .expect("the latest move heard")
    }

    fn trail(&mut self) -> Vec<Offset<Meters>> {
        self.trail_while(|_| {})
    }

    fn trail_while(&mut self, mut each_tick: impl FnMut(&mut Walk)) -> Vec<Offset<Meters>> {
        (0..TICKS)
            .map(|_| {
                self.sim.tick();
                let at = self.at();
                each_tick(self);
                at
            })
            .collect()
    }
}

// Walks the way `once` went, asking each tick for the waypoint `ahead` metres past the one nearest.
fn spam(once: &[Offset<Meters>], ahead: usize) -> Vec<Offset<Meters>> {
    let points = waypoints(*once.last().expect("a trail"));
    let mut walk = Walk::start().toward(points[1]);
    walk.trail_while(|walk| {
        let at = walk.at();
        let nearest = (0..points.len())
            .min_by(|&a, &b| {
                (at - points[a])
                    .length()
                    .total_cmp(&(at - points[b]).length())
            })
            .expect("points along the walk");
        walk.ask(points[(nearest + ahead).min(points.len() - 1)]);
    })
}

fn assert_same_trail(trail: &[Offset<Meters>], expected: &[Offset<Meters>], what: &str) {
    assert_eq!(trail.len(), expected.len(), "{what}");
    for (tick, (at, wanted)) in trail.iter().zip(expected).enumerate() {
        assert!(
            (*at - *wanted).length() < 1e-3,
            "{what}: tick {tick} at {at:?}, expected {wanted:?}"
        );
    }
}

fn arrival(trail: &[Offset<Meters>]) -> usize {
    let end = trail.last().expect("a trail");
    trail
        .iter()
        .position(|at| at == end)
        .expect("the end of the trail")
}

fn assert_arrives(trail: &[Offset<Meters>], end: Offset<Meters>, what: &str) {
    let last = trail.last().expect("a trail");
    assert!(
        (*last - end).length() < 1e-3,
        "{what}: ended at {last:?}, not {end:?}"
    );
}

fn assert_keeps_to_the_line(trail: &[Offset<Meters>], end: Offset<Meters>) {
    let length = end.length();
    for at in trail {
        let off = (at.x * end.y - at.y * end.x).abs() / length;
        assert!(
            off <= 1e-3,
            "{at:?} strays {off} from the line toward {end:?}"
        );
    }
}

// The points a metre apart along the straight line to `end`, the start first and `end` last.
fn waypoints(end: Offset<Meters>) -> Vec<Offset<Meters>> {
    let length = end.length();
    let mut points: Vec<Offset<Meters>> = (0..length.floor() as usize)
        .map(|metre| end * (metre as f32 / length))
        .collect();
    points.push(end);
    points
}
