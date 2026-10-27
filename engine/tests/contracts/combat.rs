use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::time::UnixMillis;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, script, skill};
use serde_json::json;
use testkit::{APEX, CLIP, Game, Map, Sim};

#[test]
fn an_attack_walks_into_range_and_strikes() {
    let (mut sim, player) = fight();
    let brute = brute(&mut sim, player, Offset::new(4.0, 0.0));
    let full = stat::effective(&sim.world, brute, StatKind::Health);

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: brute });

    assert!(sim.run_until(10.0, |world| stat::effective(
        world,
        brute,
        StatKind::Health
    ) < full));
}

// Right beside the player, so the fight stays where it starts.
#[test]
fn an_attacks_timing_comes_from_its_clip_at_the_attackers_speed() {
    let (mut sim, player) = fight();
    let speed = stat::effective(&sim.world, player, StatKind::AttackSpeed);

    let playing = actor::playing::play(
        &mut sim.world,
        player,
        actor::Motion::Action(actor::Action::Attack),
        None,
    )
    .expect("an attack plays");

    let since = |at: UnixMillis| (at.0 - playing.started.0) as f32;
    assert!(
        (since(playing.apex) - APEX.0 / speed).abs() <= 1.0,
        "{playing:?}"
    );
    assert!(
        (since(playing.ends) - CLIP.0 / speed).abs() <= 1.0,
        "{playing:?}"
    );
}

#[test]
fn an_attacked_monster_is_fought_until_it_falls() {
    let (mut sim, player) = fight();
    let brute = brute(&mut sim, player, Offset::new(1.0, 0.0));

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: brute });

    assert!(sim.run_until(60.0, |world| stat::is_dead(world, brute)));
}

#[test]
fn an_immortal_player_is_worn_down_but_never_falls() {
    let (mut sim, player) = fight();
    let full = stat::effective(&sim.world, player, StatKind::Health);

    stat::apply_damage(&mut sim.world, player, full * 0.5);
    assert!(stat::effective(&sim.world, player, StatKind::Health) < full);

    stat::apply_damage(&mut sim.world, player, full * 10.0);
    sim.tick();
    assert!(!stat::is_dead(&sim.world, player));
}

#[test]
fn a_player_crossing_into_another_area_stays_immortal() {
    let content = Game::new()
        .area("field", &Map::open(7, 7), json!([]))
        .area("meadow", &Map::open(7, 7), json!([]))
        .hero("field", 3.0, 3.0)
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let mut field = Sim::new(content.clone());
    field.serve("field");
    let mut meadow = Sim::new(content);
    meadow.serve("meadow");
    let player = field.join(1);

    let player = field.cross(player, &mut meadow, Pos::new(3.0, 3.0));
    let full = stat::effective(&meadow.world, player, StatKind::Health);
    stat::apply_damage(&mut meadow.world, player, full * 10.0);

    assert!(!stat::is_dead(&meadow.world, player));
}

fn fight() -> (Sim, Entity) {
    let content = Game::new()
        .area("field", &Map::open(11, 11), json!([]))
        .hero("field", 5.0, 5.0)
        .sword()
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field")
        .accept::<AttackRequest>()
        .add((
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            movement::advance,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    sim.equip(player, "sword");
    sim.tick();
    (sim, player)
}

/// A monster turned hostile that stands where it's put and never strikes back.
fn brute(sim: &mut Sim, player: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, player).expect("the player") + offset;
    let area = sim.id::<AreaDef>("field");
    let brute = sim.id::<NpcDef>("brute");
    let brute = npc::spawn(&mut sim.world, brute, at, area, Pack(u32::MAX));
    sim.world.entity_mut(brute).insert(Attitude::Hostile);
    brute
}
