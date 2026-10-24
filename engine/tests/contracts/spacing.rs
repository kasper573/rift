use std::collections::HashMap;

use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::core::units::{Cell, CellPos, Meters, TilePos};
use engine::systems::area::warp::warp;
use engine::systems::area::{AreaDef, AreaId};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::movement::{self, MoveRequest, MoveSeq, moving, position};
use engine::systems::npc::body::{self, Act, Priority};
use engine::systems::npc::{self, Npc, NpcDef, Pack};
use engine::systems::script::{self, Place};
use engine::systems::{actor, skill, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

const FIGHTERS: Pack = Pack(1);
const VICTIMS: Pack = Pack(2);

#[test]
fn two_actors_reaching_for_each_other_end_on_neighbouring_tiles() {
    for gap in [2, 3, 4, 5] {
        for direction in [(1, 0), (0, 1), (1, 1), (-1, 0)] {
            let mut arena = Arena::new();
            let player = arena.player();
            let foe = arena.fighter(arena.at(direction.0 * gap, direction.1 * gap));

            arena.send(AttackRequest { target: foe });
            arena.attack(foe, player);
            let rested_apart = arena.watch(4.0, |world| {
                !(resting(world, player)
                    && resting(world, foe)
                    && cell(world, player) == cell(world, foe))
            });

            assert!(rested_apart, "gap {gap} {direction:?}: rested on one tile");
            let world = arena.world();
            assert!(resting(world, player) && resting(world, foe));
            assert_eq!(
                apart(cell(world, player), cell(world, foe)),
                1,
                "gap {gap} {direction:?}"
            );
        }
    }
}

#[test]
fn reach_met_mid_step_never_lands_on_the_target() {
    for offset in [(3, 0), (2, 1), (3, 2), (-2, 3), (4, -1), (1, -3)] {
        let mut arena = Arena::new();
        let player = arena.player();
        let target = arena.victim(arena.at(offset.0, offset.1));

        arena.send(AttackRequest { target });
        let never_on_it = arena.watch(4.0, |world| {
            !(resting(world, player) && cell(world, player) == cell(world, target))
        });

        assert!(never_on_it, "{offset:?}: came to rest on the target");
        let world = arena.world();
        assert!(resting(world, player), "{offset:?}");
        assert_eq!(apart(cell(world, player), cell(world, target)), 1);
    }
}

#[test]
fn a_pack_from_one_side_fills_distinct_berths_where_its_target_stops() {
    for walks_away in [false, true] {
        let mut arena = Arena::new();
        let player = arena.player();
        let pack: Vec<Entity> = (0..4)
            .map(|row| arena.fighter(arena.at(4, row - 1)))
            .collect();

        for &fighter in &pack {
            arena.attack(fighter, player);
        }
        let away = arena.at(-4, 0);
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
        let target = cell(world, player);
        let mut tiles: Vec<CellPos> = pack.iter().map(|&f| cell(world, f)).collect();
        assert!(
            pack.iter().all(|&f| resting(world, f)),
            "walks away: {walks_away}"
        );
        assert!(
            tiles.iter().all(|&tile| apart(tile, target) == 1),
            "walks away: {walks_away}, {tiles:?}"
        );
        tiles.sort_by_key(|tile| (tile.x, tile.y));
        tiles.dedup();
        assert_eq!(
            tiles.len(),
            pack.len(),
            "walks away: {walks_away}, two of the pack share a tile"
        );
    }
}

#[test]
fn berths_hold_while_the_target_steps_within_reach() {
    let mut arena = Arena::new();
    let player = arena.player();
    let fighter = arena.fighter(arena.at(0, 3));
    arena.attack(fighter, player);
    arena.run(4.0);
    let berth = cell(arena.world(), fighter);

    let step = arena.at(1, 0);
    arena.send(MoveRequest {
        pos: step,
        seq: MoveSeq(1),
    });
    let held = arena.watch(3.0, |world| {
        position(world, fighter) == Some(berth.center())
    });

    assert!(held, "the fighter left a berth still in reach");
    assert_eq!(position(arena.world(), player), Some(step));
}

#[test]
fn a_ground_click_onto_an_occupied_tile_is_honoured() {
    let mut arena = Arena::new();
    let player = arena.player();
    let spot = arena.at(3, 1);
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
        let crowded = crowds(world, &pack);
        assert!(
            pack.iter().all(|&fighter| resting(world, fighter)),
            "{count}"
        );
        assert!(in_reach_of(world, &pack, player), "{count}");
        assert_eq!(crowded.len(), 8, "{count}: {crowded:?}");
        assert!(
            crowded.values().all(|&crowd| crowd <= 2),
            "{count}: {crowded:?}"
        );

        let alone = pack
            .iter()
            .position(|&fighter| crowded[&cell(world, fighter)] == 1)
            .unwrap_or(0);
        let gone = pack.remove(alone);
        npc::despawn(world, gone);
        arena.run(6.0);

        let world = arena.world();
        let crowded = crowds(world, &pack);
        assert!(in_reach_of(world, &pack, player), "{count} after one left");
        assert_eq!(crowded.len(), 8, "{count} after one left: {crowded:?}");
        assert!(
            crowded.values().all(|&crowd| crowd <= 2),
            "{count} after one left"
        );
    }
}

#[test]
fn players_and_npcs_pick_berths_alike() {
    for offset in [(3, 0), (3, 2), (-2, 4), (0, -3)] {
        let berths = [true, false].map(|as_player| {
            let mut arena = Arena::new();
            let player = arena.player();
            let target = arena.victim(arena.at(0, 0));
            let attacker = if as_player {
                let start = arena.at(offset.0, offset.1);
                movement::relocate(arena.world(), player, start);
                arena.send(AttackRequest { target });
                player
            } else {
                let aside = arena.at(-5, -5);
                movement::relocate(arena.world(), player, aside);
                let fighter = arena.fighter(arena.at(offset.0, offset.1));
                arena.attack(fighter, target);
                fighter
            };
            let arrived = arena.watch_until(4.0, |world| {
                resting(world, attacker) && apart(cell(world, attacker), cell(world, target)) == 1
            });
            assert!(arrived, "{offset:?} as player: {as_player}");
            cell(arena.world(), attacker)
        });

        assert_eq!(berths[0], berths[1], "{offset:?}");
    }
}

#[test]
fn wandering_never_ends_on_a_claimed_tile() {
    let mut arena = Arena::new();
    for (dx, dy) in [(0, 0), (1, 0), (-1, 1), (0, -2), (2, 2)] {
        arena.victim(arena.at(dx, dy));
    }
    npc::spawn_all(arena.world());

    let apart = arena.watch(20.0, |world| {
        let mut resting_on: HashMap<CellPos, u32> = HashMap::new();
        let mut actors = world.query_filtered::<Entity, With<Npc>>();
        for actor in actors.iter(world).collect::<Vec<_>>() {
            if resting(world, actor) {
                *resting_on.entry(cell(world, actor)).or_default() += 1;
            }
        }
        resting_on.values().all(|&count| count == 1)
    });

    assert!(apart, "a wanderer came to rest on a claimed tile");
}

#[test]
fn joiners_at_the_spawn_land_on_distinct_tiles_around_it() {
    let mut sim = field();
    let players: Vec<Entity> = (1..=4).map(|client| sim.join(client)).collect();
    let world = &sim.world;
    let mut tiles: Vec<CellPos> = players.iter().map(|&p| cell(world, p)).collect();
    let spawn = tiles[0];
    assert!(
        tiles.iter().all(|&tile| apart(tile, spawn) <= 1),
        "{tiles:?}"
    );
    tiles.sort_by_key(|tile| (tile.x, tile.y));
    tiles.dedup();
    assert_eq!(tiles.len(), players.len());
}

#[test]
fn a_warp_onto_an_occupied_tile_lands_on_the_nearest_free_one() {
    let mut arena = Arena::new();
    let player = arena.player();
    let spot = arena.at(4, 0);
    arena.victim(spot);
    let to = Place {
        area: arena.area,
        at: spot,
    };
    warp(arena.world(), player, to).expect("warped");

    assert_eq!(cell(&arena.sim.world, player), arena.at(4, 1).cell());
}

#[test]
fn a_berth_whose_holder_is_gone_or_fallen_is_free_again() {
    let leave: [fn(&mut World, Entity); 2] = [npc::despawn, |world, holder| {
        combat::inflict(world, holder, None, 1_000_000.0)
    }];
    for (case, leave) in leave.into_iter().enumerate() {
        let mut arena = Arena::new();
        let player = arena.player();
        let berth = arena.at(0, 1);
        let holder = arena.victim(berth);
        leave(arena.world(), holder);
        arena.run(0.2);

        let fighter = arena.fighter(arena.at(0, 3));
        arena.attack(fighter, player);
        arena.run(4.0);

        assert_eq!(cell(arena.world(), fighter), berth.cell(), "case {case}");
    }
}

struct Arena {
    sim: Sim,
    area: AreaId,
    origin: CellPos,
}

impl Arena {
    /// A player alone in the middle of open ground they can walk all over, twelve tiles each way.
    fn new() -> Arena {
        let mut sim = field();
        let player = sim.join(1);
        let origin = cell(&sim.world, player);
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

    fn at(&self, dx: i32, dy: i32) -> Pos<Meters> {
        self.origin.step((dx, dy)).center()
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
                let (dx, dy) = (4.0 * angle.cos(), 4.0 * angle.sin());
                self.fighter(self.at(dx.round() as i32, dy.round() as i32))
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
        .wearing(&["sword"])
        .npc("brute", 100_000.0, json!([]))
        .edit("npc", "brute", |brute| {
            brute["basic_attack"] = json!(path("skill", "strike"))
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
    !moving(world, actor) && position(world, actor).is_some_and(|at| at.on_center())
}

fn cell(world: &World, actor: Entity) -> CellPos {
    position(world, actor).expect("position").cell()
}

fn apart(a: CellPos, b: CellPos) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

fn crowds(world: &World, actors: &[Entity]) -> HashMap<CellPos, u32> {
    let mut crowds = HashMap::new();
    for &actor in actors {
        *crowds.entry(cell(world, actor)).or_default() += 1;
    }
    crowds
}

fn in_reach_of(world: &World, actors: &[Entity], target: Entity) -> bool {
    let target = cell(world, target);
    actors
        .iter()
        .all(|&actor| apart(cell(world, actor), target) <= 1)
}
