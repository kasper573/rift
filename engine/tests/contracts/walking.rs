use bevy::math::Vec3;
use bevy_ecs::entity::Entity;
use engine::core::content::gltf::Triangle;
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::math::{Offset, Pos};
use engine::core::units::{Degrees, MeterPos, Meters};
use engine::systems::actor::{Action, Actor};
use engine::systems::area;
use engine::systems::area::prop::{Footprint, ModelHash};
use engine::systems::movement::{self, Bodies, HeardMove, MoveRequest, MoveSeq, position};
use engine::systems::player::CommandLock;
use engine::systems::region;
use serde_json::{Value, json};
use testkit::{BODY, Game, HEIGHTS, Map, RUN_PACE, Sim, WALK_PACE, path, rig_clips};

const TICKS: usize = 240;

#[test]
fn asking_again_or_further_along_as_a_walk_goes_walks_on_as_one_without_a_pause() {
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
        let mut walk = Walk::start().toward(end * 0.375);
        let mut ticks = 0;
        let extended = walk.trail_while(|walk| {
            ticks += 1;
            if ticks == 12 {
                walk.ask(end);
            }
        });
        assert_same_trail(&extended, &once, &format!("walking on toward {end:?}"));
        let again = Walk::start().toward(end).trail_while(|walk| walk.ask(end));
        assert_same_trail(&again, &once, &format!("asking again toward {end:?}"));
        for ahead in 1..=3 {
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
        for ahead in 2..=3 {
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

/// A ground far wider than a field walks as one: a walk across it is the straight line where
/// nothing blocks, goes round a boulder wherever on it one stands, and round a rock by its end
/// wherever the rock lies, even straddling a seam of the ground unevenly.
#[test]
fn a_wide_ground_walks_as_one_wherever_a_boulder_or_a_rock_stands() {
    let radius = radius();
    let give = radius / 10.0;
    let wide = || Map::open(500, 250);
    for end in [Offset::new(480.0, 0.0), Offset::new(480.0, 230.0)] {
        let open = Walk::on(
            Game::new()
                .area("field", &wide(), json!([]))
                .hero("field", 10.0, 10.0),
        )
        .toward(end)
        .trail_to_rest();
        assert_arrives(&open, end, "across the open ground");
        assert_keeps_to_the_line(&open, end);
    }

    let boulders: Vec<Pos<Meters>> = (2..=18)
        .map(|n| Pos::new(25.0 * n as f32 + 0.3 * (n % 4) as f32, 125.0))
        .collect();
    let placed: Vec<Value> = boulders
        .iter()
        .map(|at| json!({ "type": "prop", "prop": path("prop", "block"), "at": { "x": at.x, "y": at.y } }))
        .collect();
    let start = Pos::new(10.0, 125.0);
    let end = Offset::new(480.0, 0.0);
    let strewn = Walk::over(
        wide(),
        json!({ "type": "circle", "radius": 1.0 }),
        Value::Array(placed),
        start,
    )
    .toward(end)
    .trail_to_rest();
    assert_arrives(&strewn, end, "past every boulder");
    let closest = strewn
        .iter()
        .inspect(|at| assert!(at.y.abs() <= 2.0, "{at:?} strays from the boulders' line"))
        .flat_map(|at| {
            boulders
                .iter()
                .map(move |boulder| (start + *at).distance(*boulder))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("a trail");
    assert!(
        closest >= Meters(1.0 + radius - give),
        "came within {closest:?} of a boulder's middle"
    );

    let rock = |x: usize, y: usize| {
        (100..120).contains(&y) && (250..262).contains(&x)
            || (120..140).contains(&y) && (256..262).contains(&x)
    };
    let rows: Vec<String> = (0..250)
        .map(|y| {
            (0..500)
                .map(|x| if rock(x, y) { '#' } else { '.' })
                .collect()
        })
        .collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let end = Offset::new(32.0, 0.0);
    let past_the_rock = Walk::on(
        Game::new()
            .area("field", &Map::of(&rows), json!([]))
            .hero("field", 240.0, 130.0),
    )
    .toward(end)
    .trail_to_rest();
    assert_arrives(&past_the_rock, end, "round the rock");
    for at in &past_the_rock {
        assert!(
            at.y >= -give && at.y <= 10.0 + radius + give,
            "{at:?} goes round the rock the long way"
        );
    }
}

/// A walk keeps a body out of what blocks, give or take a tenth of its radius, and through a
/// notch the body fits with that much to spare.
#[test]
fn a_walk_goes_round_what_blocks_and_through_a_notch_a_body_fits() {
    let radius = radius();
    let give = radius / 10.0;
    let stands = [
        (
            "a boulder",
            json!({ "type": "prop", "prop": path("prop", "block"), "at": { "x": 7.0, "y": 7.0 } }),
        ),
        (
            "an object that looks like a boulder",
            json!({
                "type": "object",
                "shape": { "type": "point", "x": 7.0, "y": 7.0 },
                "look": path("prop", "block"),
            }),
        ),
    ];
    for (what, standing) in stands {
        let boulder = Walk::among(
            json!({ "type": "circle", "radius": 1.0 }),
            json!([standing]),
            Pos::new(2.0, 7.0),
        )
        .toward(Offset::new(10.0, 0.0))
        .trail();
        assert_arrives(&boulder, Offset::new(10.0, 0.0), what);
        let center = Pos::new(7.0, 7.0);
        for at in &boulder {
            let gap = (Pos::new(2.0, 7.0) + *at).distance(center);
            assert!(
                gap >= Meters(1.0 + radius - give),
                "{at:?} is {gap:?} from the middle of {what}"
            );
        }
    }

    let edge = Offset::new(5.0 - 1.0 - radius + give, 0.0);
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
    let through = wall(2.0 * radius + 0.3);
    assert_arrives(&through, Offset::new(0.0, 8.0), "through the notch");
    assert!(!round_the_end(&through), "went round the wall's end");
    let around = wall(2.0 * radius - 0.1);
    assert_arrives(&around, Offset::new(0.0, 8.0), "round the wall");
    assert!(
        round_the_end(&around),
        "squeezed through a notch too narrow"
    );

    // A cup open toward the walker: the walk leaves it and goes round, off every side.
    let cup = [
        (-3.0, -2.0),
        (3.0, -2.0),
        (3.0, 2.0),
        (2.0, 2.0),
        (2.0, -1.0),
        (-2.0, -1.0),
        (-2.0, 2.0),
        (-3.0, 2.0),
    ];
    let points: Vec<Value> = cup.iter().map(|(x, y)| json!({ "x": x, "y": y })).collect();
    let round_the_cup = Walk::among(
        json!({ "type": "polygon", "points": points }),
        json!([{ "type": "prop", "prop": path("prop", "block"), "at": { "x": 7.0, "y": 7.0 } }]),
        Pos::new(7.0, 9.5),
    )
    .toward(Offset::new(0.0, -6.0))
    .trail();
    assert_arrives(&round_the_cup, Offset::new(0.0, -6.0), "round the cup");
    let sides: Vec<(Pos<Meters>, Pos<Meters>)> = (0..cup.len())
        .map(|at| {
            let corner = |(x, y): (f32, f32)| Pos::new(7.0 + x, 7.0 + y);
            (corner(cup[at]), corner(cup[(at + 1) % cup.len()]))
        })
        .collect();
    for at in &round_the_cup {
        let stands = Pos::new(7.0, 9.5) + *at;
        let gap = sides
            .iter()
            .map(|&(a, b)| distance_to_side(stands, a, b))
            .fold(f32::INFINITY, f32::min);
        assert!(gap >= radius - give, "{stands:?} is {gap} m from the cup");
    }
}

fn distance_to_side(at: Pos<Meters>, a: Pos<Meters>, b: Pos<Meters>) -> f32 {
    let along = b - a;
    let t = ((at - a).dot(along) / along.square_length()).clamp(0.0, 1.0);
    (at - (a + along * t)).length()
}

#[test]
fn a_footprint_fitted_to_a_model_takes_the_ground_under_all_of_it_a_walker_meets() {
    // A boulder on a narrow foot that bulges at the waist, under a crown far overhead.
    let boulder: Vec<Triangle> = [
        walls((0.0, 0.2), (1.0, 1.5)),
        walls((1.0, 1.5), (2.0, 0.2)),
        walls((4.0, 3.0), (5.0, 3.0)),
    ]
    .concat();
    let fitted = |tall: f32| {
        Footprint::fitted(&boulder, Meters(tall), ModelHash("boulder".to_owned()))
            .expect("a model standing on the ground has a footprint")
    };
    let takes = |footprint: &Footprint, x: f32| {
        footprint.covers(Pos::origin(), Degrees(0.0), 1.0, Pos::new(x, 0.0))
    };
    let grown = fitted(1.8);
    assert!(takes(&grown, 1.4), "the bulge at their waist is in the way");
    assert!(!takes(&grown, 2.5), "the crown overhead is walked under");
    let small = fitted(0.3);
    assert!(
        takes(&small, 0.5),
        "as far as the boulder reaches at their height"
    );
    assert!(
        !takes(&small, 1.4),
        "the bulge is over a small walker's head"
    );
}

/// The four walls of a square shaft that is `low.1` to each side at the height `low.0` and
/// `high.1` to each side at `high.0`.
fn walls(low: (f32, f32), high: (f32, f32)) -> Vec<Triangle> {
    let corner = |(height, half): (f32, f32), turn: usize| {
        let (x, z) = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)][turn % 4];
        Vec3::new(x * half, height, z * half)
    };
    (0..4)
        .flat_map(|turn| {
            let (a, b) = (corner(low, turn), corner(low, turn + 1));
            let (c, d) = (corner(high, turn + 1), corner(high, turn));
            [[a, b, c], [a, c, d]]
        })
        .map(|corners| Triangle {
            corners,
            normals: None,
        })
        .collect()
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
    for (speed, gait) in [
        (WALK_PACE.0.0 * 0.8, Action::Walk),
        (RUN_PACE.0.0 * 0.85, Action::Run),
        (RUN_PACE.0.0 * 2.0, Action::Run),
    ] {
        let mut walk = Walk::on(
            Game::new()
                .area("field", &Map::open(19, 19), json!([]))
                .skeleton_of(BODY, &rig_clips(), json!({}))
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
            if Action::GAITS.contains(&action) && !seen.contains(&action) {
                seen.push(action);
            }
        }
        assert_eq!(
            seen,
            vec![gait],
            "at {speed} m/s, the walker covers ground in one gait"
        );
    }
}

#[test]
fn a_wader_slows_as_the_water_deepens_and_keeps_pace_where_water_slows_no_one() {
    let deep = 0.5;
    for (wading, kept) in [
        (Some(json!({ "deepest": 1.0, "slowest": 0.5 })), 0.75),
        (Some(json!({ "deepest": 0.25, "slowest": 0.4 })), 0.4),
        (None, 1.0),
    ] {
        let water = WaterSurface::from_fn(
            GroundSize {
                width: 19,
                height: 19,
            },
            HEIGHTS,
            |column, _| Meters(if column >= 20 { deep } else { 0.0 }),
        );
        let game = water_parts(
            Game::new()
                .area("field", &Map::open(19, 19), json!([]))
                .hero("field", 2.0, 9.0),
            "field",
            &water,
        );
        let game = match &wading {
            Some(wading) => game.edit_settings(|settings| settings["wading"] = wading.clone()),
            None => game,
        };
        let mut walk = Walk::on(game).toward(Offset::new(15.0, 0.0));
        let trail = walk.trail();
        let steps: Vec<f32> = trail
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .collect();
        let dry = steps.first().copied().expect("a step on dry ground");
        let wet = trail
            .windows(2)
            .zip(&steps)
            .find(|(pair, _)| pair[0].x > 9.0 && pair[1].x < 14.0)
            .map(|(_, step)| *step)
            .expect("a step through the water");
        assert!(
            (wet / dry - kept).abs() < 0.02,
            "{wading:?}: a wader in {deep} m of water keeps {} of their pace, not {kept}",
            wet / dry
        );
    }
}

/// `game` with `water` lying on its area `area`.
fn water_parts(game: Game, area: &str, water: &WaterSurface) -> Game {
    let game = water
        .size()
        .each_tile()
        .filter_map(|at| Some((at, water.encode_tile(at)?)))
        .fold(game, |game, (at, bytes)| {
            game.part(
                "area",
                area,
                &area::WATER.named(&at.key(), "png"),
                bytes.to_vec(),
            )
        });
    let overview = water.encode_overview().expect("water to lie somewhere");
    game.part(
        "area",
        area,
        &area::WATER_OVERVIEW.named("", "png"),
        overview,
    )
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
        Walk::over(Map::open(15, 15), footprint, placed, from)
    }

    fn over(map: Map, footprint: Value, placed: Value, from: Pos<Meters>) -> Walk {
        Walk::on(
            Game::new()
                .prop("block", footprint)
                .area("field", &map, placed)
                .hero("field", from.x, from.y),
        )
    }

    fn on(game: Game) -> Walk {
        let content = game.load();
        let mut sim = Sim::new(content);
        sim.serve("field")
            .add(movement::move_request)
            .plan(movement::AdvanceWalks::default());
        region::hold_whole(&mut sim.world);
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

    /// The walk's trail until it stands still, however long it takes.
    fn trail_to_rest(&mut self) -> Vec<Offset<Meters>> {
        let mut trail = Vec::new();
        loop {
            self.sim.tick();
            let at = self.at();
            if trail.last() == Some(&at) {
                return trail;
            }
            trail.push(at);
        }
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

/// Positions are single floats, so a walk strays from its line by their precision at the
/// distance it covers, a few millimetres over hundreds of metres.
fn assert_keeps_to_the_line(trail: &[Offset<Meters>], end: Offset<Meters>) {
    let length = end.length();
    let give = 1e-3 + length * 1e-5;
    for at in trail {
        let off = (at.x * end.y - at.y * end.x).abs() / length;
        assert!(
            off <= give,
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

/// The radius of the test game's bodies, as its player's model is.
fn radius() -> f32 {
    let game = Game::new()
        .area("field", &Map::open(4, 4), json!([]))
        .hero("field", 1.0, 1.0);
    Bodies::of(&game.load())
        .expect("the test game knows its bodies")
        .radius
        .0
}
