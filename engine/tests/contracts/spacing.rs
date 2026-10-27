use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::warp::warp;
use engine::systems::area::{AreaDef, AreaId};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::movement::{self, MoveRequest, MoveSeq, SPACING, moving, position};
use engine::systems::npc::body::{self, Act, Priority};
use engine::systems::npc::{self, Npc, NpcDef, Pack};
use engine::systems::script::{self, Place};
use engine::systems::stat::StatKind;
use engine::systems::{actor, skill, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

const FIGHTERS: Pack = Pack(1);
const VICTIMS: Pack = Pack(2);
/// How far a brute strikes from, the `range` of its stats.
const REACH: Meters = Meters(1.5);

#[test]
fn two_actors_reaching_for_each_other_end_in_reach_with_their_bodies_apart() {
    for gap in [2.0, 3.0, 4.0, 5.0] {
        for direction in [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (-1.0, 0.0)] {
            let mut arena = Arena::new();
            let player = arena.player();
            let foe = arena.fighter(arena.at(direction.0 * gap, direction.1 * gap));

            arena.send(AttackRequest { target: foe });
            arena.attack(foe, player);
            let rested_apart = arena.watch(4.0, |world| {
                !(resting(world, player) && resting(world, foe) && overlapping(world, player, foe))
            });

            assert!(
                rested_apart,
                "gap {gap} {direction:?}: rested on one another"
            );
            let world = arena.world();
            assert!(resting(world, player) && resting(world, foe));
            assert!(
                in_reach(world, player, foe) && !overlapping(world, player, foe),
                "gap {gap} {direction:?}: {:?} and {:?}",
                at(world, player),
                at(world, foe)
            );
        }
    }
}

#[test]
fn reach_met_on_the_way_never_lands_on_the_target() {
    for offset in [
        (3.0, 0.0),
        (2.0, 1.0),
        (3.0, 2.0),
        (-2.0, 3.0),
        (4.0, -1.0),
        (1.0, -3.0),
    ] {
        let mut arena = Arena::new();
        let player = arena.player();
        let target = arena.victim(arena.at(offset.0, offset.1));

        arena.send(AttackRequest { target });
        let never_on_it = arena.watch(4.0, |world| {
            !(resting(world, player) && overlapping(world, player, target))
        });

        assert!(never_on_it, "{offset:?}: came to rest on the target");
        let world = arena.world();
        assert!(resting(world, player), "{offset:?}");
        assert!(in_reach(world, player, target) && !overlapping(world, player, target));
    }
}

#[test]
fn a_pack_from_one_side_fills_distinct_berths_where_its_target_stops() {
    for walks_away in [false, true] {
        let mut arena = Arena::new();
        let player = arena.player();
        let pack: Vec<Entity> = (0..4)
            .map(|row| arena.fighter(arena.at(4.0, row as f32 - 1.0)))
            .collect();

        for &fighter in &pack {
            arena.attack(fighter, player);
        }
        let away = arena.at(-4.0, 0.0);
        if walks_away {
            arena.send(MoveRequest {
                pos: away,
                seq: MoveSeq(1),
            });
        }
        arena.run(8.0);

        let world = arena.world();
        if walks_away {
            assert_eq!(position(world, player), Some(away));
        }
        assert!(
            pack.iter().all(|&f| resting(world, f)),
            "walks away: {walks_away}"
        );
        assert!(
            pack.iter()
                .all(|&f| in_reach(world, f, player) && !overlapping(world, f, player)),
            "walks away: {walks_away}, {:?}",
            positions(world, &pack)
        );
        assert_eq!(
            overlapping_pairs(world, &pack),
            0,
            "walks away: {walks_away}, two of the pack share a berth: {:?}",
            positions(world, &pack)
        );
    }
}

#[test]
fn berths_hold_while_the_target_steps_within_reach() {
    let mut arena = Arena::new();
    let player = arena.player();
    let fighter = arena.fighter(arena.at(0.0, 3.0));
    arena.attack(fighter, player);
    arena.run(4.0);
    let berth = at(arena.world(), fighter);

    let step = arena.origin.lerp(berth, 0.3);
    arena.send(MoveRequest {
        pos: step,
        seq: MoveSeq(1),
    });
    let held = arena.watch(3.0, |world| position(world, fighter) == Some(berth));

    assert!(held, "the fighter left a berth still in reach");
    assert_eq!(position(arena.world(), player), Some(step));
}

#[test]
fn a_fighter_chasing_a_target_walking_away_strikes_it_again_and_again_before_the_walk_ends() {
    for direction in [(-1.0, 0.0), (0.0, 1.0), (-1.0, -1.0)] {
        let mut arena = Arena::new();
        let player = arena.player();
        let fighter = arena.fighter(arena.at(-direction.0 * 2.0, -direction.1 * 2.0));
        let away = arena.at(direction.0 * 10.0, direction.1 * 10.0);

        arena.attack(fighter, player);
        arena.send(MoveRequest {
            pos: away,
            seq: MoveSeq(1),
        });
        let mut health = stat::effective(arena.world(), player, StatKind::Health);
        let mut strikes = 0;
        arena.watch_until(5.0, |world| {
            let now = stat::effective(world, player, StatKind::Health);
            if now < health && moving(world, player) {
                strikes += 1;
            }
            health = now;
            position(world, player) == Some(away)
        });

        assert!(
            strikes >= 3,
            "{direction:?}: struck {strikes} times on the way"
        );
    }
}

#[test]
fn a_ground_click_onto_an_occupied_spot_is_honoured() {
    let mut arena = Arena::new();
    let player = arena.player();
    let spot = arena.at(3.0, 1.0);
    arena.victim(spot);

    arena.send(MoveRequest {
        pos: spot,
        seq: MoveSeq(1),
    });
    arena.run(4.0);

    let world = arena.world();
    assert_eq!(position(world, player), Some(spot));
    assert!(resting(world, player));
}

#[test]
fn no_attacker_is_ever_held_out_of_reach_however_many_come_and_go() {
    for count in [9, 14] {
        let mut arena = Arena::new();
        let player = arena.player();
        let mut pack = arena.ring_of_fighters(count, player);

        arena.run(8.0);

        let world = arena.world();
        assert!(
            pack.iter().all(|&fighter| resting(world, fighter)),
            "{count}"
        );
        assert!(in_reach_of(world, &pack, player), "{count}");
        assert_eq!(
            overlapping_pairs(world, &pack),
            0,
            "{count}: {:?}",
            positions(world, &pack)
        );

        let gone = pack.remove(0);
        npc::despawn(world, gone);
        arena.run(6.0);

        let world = arena.world();
        assert!(in_reach_of(world, &pack, player), "{count} after one left");
        assert_eq!(
            overlapping_pairs(world, &pack),
            0,
            "{count} after one left: {:?}",
            positions(world, &pack)
        );
    }
}

#[test]
fn players_and_npcs_pick_berths_alike() {
    for offset in [(3.0, 0.0), (3.0, 2.0), (-2.0, 4.0), (0.0, -3.0)] {
        let berths = [true, false].map(|as_player| {
            let mut arena = Arena::new();
            let player = arena.player();
            let target = arena.victim(arena.at(0.0, 0.0));
            let attacker = if as_player {
                let start = arena.at(offset.0, offset.1);
                movement::relocate(arena.world(), player, start);
                arena.send(AttackRequest { target });
                player
            } else {
                let aside = arena.at(-5.0, -5.0);
                movement::relocate(arena.world(), player, aside);
                let fighter = arena.fighter(arena.at(offset.0, offset.1));
                arena.attack(fighter, target);
                fighter
            };
            let arrived = arena.watch_until(4.0, |world| {
                resting(world, attacker)
                    && in_reach(world, attacker, target)
                    && !overlapping(world, attacker, target)
            });
            assert!(arrived, "{offset:?} as player: {as_player}");
            at(arena.world(), attacker)
        });

        assert!(
            berths[0].distance(berths[1]) < Meters(1e-3),
            "{offset:?}: {berths:?}"
        );
    }
}

#[test]
fn wandering_never_ends_on_a_taken_spot() {
    let mut arena = Arena::new();
    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, -2.0), (2.0, 2.0)] {
        arena.victim(arena.at(dx, dy));
    }
    npc::spawn_all(arena.world());

    let apart = arena.watch(20.0, |world| {
        let mut actors = world.query_filtered::<Entity, With<Npc>>();
        let resting: Vec<Entity> = actors
            .iter(world)
            .filter(|&actor| resting(world, actor))
            .collect();
        overlapping_pairs(world, &resting) == 0
    });

    assert!(apart, "a wanderer came to rest on a taken spot");
}

#[test]
fn joiners_at_the_spawn_land_on_distinct_spots_around_it() {
    let mut sim = field();
    let players: Vec<Entity> = (1..=4).map(|client| sim.join(client)).collect();
    let world = &sim.world;
    let spawn = at(world, players[0]);
    assert!(
        players
            .iter()
            .all(|&player| at(world, player).distance(spawn) <= Meters(SPACING.0 + 1e-3)),
        "{:?}",
        positions(world, &players)
    );
    assert_eq!(
        overlapping_pairs(world, &players),
        0,
        "{:?}",
        positions(world, &players)
    );
}

#[test]
fn a_warp_onto_an_occupied_spot_lands_on_the_nearest_free_one() {
    let mut arena = Arena::new();
    let player = arena.player();
    let spot = arena.at(4.0, 0.0);
    arena.victim(spot);
    let to = Place {
        area: arena.area,
        at: spot,
    };
    warp(arena.world(), player, to).expect("warped");

    let landed = at(&arena.sim.world, player);
    assert!(
        (landed.distance(spot).0 - SPACING.0).abs() < 1e-3,
        "landed at {landed:?}, {spot:?} taken"
    );
}

#[test]
fn a_berth_whose_holder_is_gone_or_fallen_is_free_again() {
    let leave: [fn(&mut World, Entity); 2] = [npc::despawn, |world, holder| {
        combat::inflict(world, holder, None, 1_000_000.0)
    }];
    let berth = {
        let mut control = Arena::new();
        let player = control.player();
        let fighter = control.fighter(control.at(0.0, 3.0));
        control.attack(fighter, player);
        control.run(4.0);
        at(control.world(), fighter)
    };
    for (case, leave) in leave.into_iter().enumerate() {
        let mut arena = Arena::new();
        let player = arena.player();
        let holder = arena.victim(berth);
        leave(arena.world(), holder);
        arena.run(0.2);

        let fighter = arena.fighter(arena.at(0.0, 3.0));
        arena.attack(fighter, player);
        arena.run(4.0);

        assert_eq!(at(arena.world(), fighter), berth, "case {case}");
    }
}

#[test]
fn a_fighter_rooted_in_place_strikes_whoever_steps_into_its_reach() {
    let mut arena = Arena::new();
    let player = arena.player();
    let post = arena.post(arena.at(4.0, 0.0));
    arena.attack(post, player);
    arena.run(1.0);
    let full = stat::effective(arena.world(), player, StatKind::Health);

    arena.send(MoveRequest {
        pos: arena.at(3.0, 0.0),
        seq: MoveSeq(1),
    });

    assert!(arena.watch_until(5.0, |world| {
        stat::effective(world, player, StatKind::Health) < full
    }));
}

struct Arena {
    sim: Sim,
    area: AreaId,
    origin: Pos<Meters>,
}

impl Arena {
    /// A player alone in the middle of open ground they can walk all over, twelve metres each way.
    fn new() -> Arena {
        let mut sim = field();
        let player = sim.join(1);
        sim.equip(player, "sword");
        let origin = at(&sim.world, player);
        let area = sim.id::<AreaDef>("field");
        Arena { sim, area, origin }
    }

    fn world(&mut self) -> &mut World {
        &mut self.sim.world
    }

    fn player(&mut self) -> Entity {
        self.sim.player(1)
    }

    fn send<M: Message>(&mut self, message: M) {
        let conn = self.sim.conn(1);
        self.sim.send(conn, message);
    }

    fn at(&self, dx: f32, dy: f32) -> Pos<Meters> {
        self.origin + Offset::new(dx, dy)
    }

    fn fighter(&mut self, at: Pos<Meters>) -> Entity {
        self.brute(at, FIGHTERS)
    }

    fn victim(&mut self, at: Pos<Meters>) -> Entity {
        self.brute(at, VICTIMS)
    }

    fn brute(&mut self, at: Pos<Meters>, pack: Pack) -> Entity {
        let area = self.area;
        let def = self.sim.id::<NpcDef>("brute");
        let brute = npc::spawn(self.world(), def, at, area, pack);
        self.world().entity_mut(brute).insert(Attitude::Hostile);
        brute
    }

    /// A fighter that never takes a step.
    fn post(&mut self, at: Pos<Meters>) -> Entity {
        let area = self.area;
        let def = self.sim.id::<NpcDef>("post");
        let post = npc::spawn(self.world(), def, at, area, FIGHTERS);
        self.world().entity_mut(post).insert(Attitude::Hostile);
        post
    }

    fn attack(&mut self, attacker: Entity, target: Entity) {
        body::order(
            self.world(),
            attacker,
            Act::Attack {
                target,
                within: Meters(40.0),
            },
            Priority(9),
            None,
        );
    }

    /// `count` fighters set around the target out of reach, all coming for it.
    fn ring_of_fighters(&mut self, count: usize, target: Entity) -> Vec<Entity> {
        let fighters: Vec<Entity> = (0..count)
            .map(|index| {
                let angle = std::f32::consts::TAU * index as f32 / count as f32;
                self.fighter(self.at(4.0 * angle.cos(), 4.0 * angle.sin()))
            })
            .collect();
        for &fighter in &fighters {
            self.attack(fighter, target);
        }
        fighters
    }

    fn run(&mut self, seconds: f32) {
        self.sim.run(seconds);
    }

    /// Plays on for `seconds`, as long as `holds` holds at every tick.
    fn watch(&mut self, seconds: f32, mut holds: impl FnMut(&mut World) -> bool) -> bool {
        let mut held = true;
        self.sim.run_until(seconds, |world| {
            held &= holds(world);
            !held
        });
        held
    }

    fn watch_until(&mut self, seconds: f32, done: impl FnMut(&mut World) -> bool) -> bool {
        self.sim.run_until(seconds, done)
    }
}

/// Open ground, with brutes to fight that hit back with a strike but hardly ever fall.
fn field() -> Sim {
    let wanderers = [(10.0, 12.0), (14.0, 11.0), (13.0, 14.0), (11.0, 11.0)];
    let residents: Vec<Value> = (0..wanderers.len())
        .zip(wanderers)
        .map(|(index, (x, y))| {
            json!({
                "type": "resident",
                "npc": path("npc", &format!("wanderer_{index}")),
                "at": { "x": x, "y": y },
            })
        })
        .collect();
    let game = (0..wanderers.len()).fold(Game::new(), |game, index| {
        game.npc(
            &format!("wanderer_{index}"),
            100_000.0,
            json!([{ "type": "script", "script": "wanderer_roam.script.json" }]),
        )
    });
    let content = game
        .area("field", &Map::open(25, 25), json!(residents))
        .hero("field", 12.0, 12.0)
        .sword()
        .npc("brute", 100_000.0, json!([]))
        .edit("npc", "brute", |brute| {
            brute["basic_attack"] = json!(path("skill", "strike"))
        })
        .npc("post", 100_000.0, json!([]))
        .edit("npc", "post", |post| {
            post["basic_attack"] = json!(path("skill", "strike"));
            post["stats"]["movement_speed"] = json!(0.0);
        })
        .script(
            "wanderer_roam",
            "npc",
            json!({
                "spawned": { "type": "on", "event": "started", "next": "roam" },
                "roam": { "type": "wander", "leash": 2.5, "priority": 1, "next": "rest" },
                "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "roam" },
            }),
        )
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field")
        .accept::<AttackRequest>()
        .accept::<MoveRequest>()
        .add((
            movement::move_request,
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            npc::body::drive,
            movement::advance,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    script::spawn_stateless(&mut sim.world);
    sim
}

fn resting(world: &World, actor: Entity) -> bool {
    !moving(world, actor)
}

fn at(world: &World, actor: Entity) -> Pos<Meters> {
    position(world, actor).expect("position")
}

fn positions(world: &World, actors: &[Entity]) -> Vec<Pos<Meters>> {
    actors.iter().map(|&actor| at(world, actor)).collect()
}

/// Whether the two bodies stand on one another, two just touching not counted.
fn overlapping(world: &World, a: Entity, b: Entity) -> bool {
    at(world, a).distance(at(world, b)) < Meters(SPACING.0 - 1e-3)
}

fn overlapping_pairs(world: &World, actors: &[Entity]) -> usize {
    actors
        .iter()
        .enumerate()
        .flat_map(|(index, &a)| actors[index + 1..].iter().map(move |&b| (a, b)))
        .filter(|&(a, b)| overlapping(world, a, b))
        .count()
}

fn in_reach(world: &World, actor: Entity, target: Entity) -> bool {
    at(world, actor).distance(at(world, target)) <= Meters(REACH.0 + 1e-3)
}

fn in_reach_of(world: &World, actors: &[Entity], target: Entity) -> bool {
    actors.iter().all(|&actor| in_reach(world, actor, target))
}
