//! Time budgets are in game-seconds, sized for a live 30 Hz server with its NPC population —
//! the in-process stage just gets there faster.

use std::collections::{HashSet, VecDeque};

use world::core::area::{self, Area, Portal};
use world::core::math::{Pos, Tiles};

use crate::{Player, Stage, eventually};

/// Binds every scenario below to a harness, one group at a time: `players_only` needs no NPCs.
#[macro_export]
macro_rules! for_each_scenario {
    ($bind:ident) => {
        $bind! { players_only:
            joining_spawns_a_living_player_in_the_spawn_area,
            a_move_command_walks_the_player_to_the_target,
            attacking_damages_then_kills_the_target(world::ACTION_DEAD),
            swings_are_discrete_with_recovery_between,
            a_dead_target_is_not_attackable(world::ACTION_ATTACK),
            respawn_restores_a_dead_player_at_spawn,
            moving_onto_a_portal_with_intent_crosses_areas,
            players_in_the_same_area_see_each_other,
            crossing_areas_removes_a_player_from_others_view,
            disconnecting_removes_a_player_from_others_view,
            spectating_requires_the_spectate_role,
            the_spectate_role_grants_spectating,
            players_never_see_spectators,
            a_spectator_follows_its_player_through_a_portal,
            spectators_see_players_beyond_view_distance,
        }
        $bind! { full:
            aggressive_npcs_attack_a_nearby_player,
            a_player_can_kill_a_nearby_npc,
            killing_an_npc_grants_xp_and_loot,
            consuming_a_potion_heals_and_destroys_it,
            inventories_are_private,
        }
    };
}

fn home() -> &'static Area {
    &area::areas()[world::spawn_zone() as usize]
}

fn exit_portal() -> &'static Portal {
    let home = home();
    home.portals
        .iter()
        .find(|portal| portal.dest_area != home.id)
        .expect("the spawn area has a portal to another area")
}

pub fn home_portal() -> Pos<Tiles> {
    exit_portal().rect.center()
}

fn away() -> world::core::area::AreaId {
    exit_portal().dest_area
}

fn walk_target() -> Pos<Tiles> {
    let home = home();
    nearest_tile(home, home.spawn, |_, depth| depth >= 12)
        .expect("the spawn area has walkable tiles away from spawn")
}

// The nearest such tile, so a live server's NPCs rarely cut the trek short.
fn far_from_exit() -> Pos<Tiles> {
    let portal = exit_portal();
    let destination = &area::areas()[portal.dest_area.0 as usize];
    let exit = portal.dest;
    nearest_tile(destination, exit, |tile, _| {
        tile.distance(exit) > world::VIEW_DISTANCE.0 + 4.0
    })
    .expect("the destination area extends beyond view distance from the portal exit")
}

fn nearest_tile(
    area: &Area,
    from: Pos<Tiles>,
    accept: impl Fn(Pos<Tiles>, usize) -> bool,
) -> Option<Pos<Tiles>> {
    let center =
        |tile: (i32, i32)| Pos::new(Tiles(tile.0 as f32 + 0.5), Tiles(tile.1 as f32 + 0.5));
    let on_portal = |tile| {
        area.portals
            .iter()
            .any(|portal| portal.rect.contains(center(tile)))
    };
    let start = (from.x.0 as i32, from.y.0 as i32);
    let mut queue = VecDeque::from([(start, 0usize)]);
    let mut seen = HashSet::from([start]);
    while let Some((tile, depth)) = queue.pop_front() {
        if depth > 0 && accept(center(tile), depth) && !on_portal(tile) {
            return Some(center(tile));
        }
        for step in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (tile.0 + step.0, tile.1 + step.1);
            if area.grid.walkable(center(next)) && seen.insert(next) {
                queue.push_back((next, depth + 1));
            }
        }
    }
    None
}

fn me_pos(player: &mut Box<dyn Player>) -> Pos<Tiles> {
    let view = player.view();
    let me = view.me().expect("client sees its own entity");
    me.pos
}

fn my_area(player: &mut Box<dyn Player>) -> Option<world::core::area::AreaId> {
    player.view().me().and_then(|me| me.area)
}

fn my_health(player: &mut Box<dyn Player>) -> f32 {
    player.view().me().and_then(|me| me.health).unwrap_or(0.0)
}

// Hostile NPCs can kill the traveler mid-trek on a live server — respawn and press on.
fn travel(
    stage: &mut dyn Stage,
    player: &mut Box<dyn Player>,
    target: Pos<Tiles>,
    seconds: f32,
) -> bool {
    eventually(stage, seconds, || {
        if me_pos(player).distance(target) < 1.5 {
            return true;
        }
        if my_health(player) <= 0.0 {
            player.respawn();
        }
        player.move_to(target.x.0, target.y.0);
        false
    })
}

fn cross_portal(
    stage: &mut dyn Stage,
    player: &mut Box<dyn Player>,
    portal: Pos<Tiles>,
    destination: world::core::area::AreaId,
    seconds: f32,
) -> bool {
    eventually(stage, seconds, || {
        if my_area(player) == Some(destination) {
            return true;
        }
        player.move_to(portal.x.0, portal.y.0);
        false
    })
}

pub fn joining_spawns_a_living_player_in_the_spawn_area(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let view = a.view();
    let me = view.me().expect("client must see the player it controls");
    assert!(
        me.health.expect("player has vitals") > 0.0,
        "player spawns alive"
    );
    assert_eq!(
        me.area,
        Some(world::core::area::spawn_zone()),
        "player spawns in the spawn area"
    );
    assert!(me.actor, "a player is a rendered actor");
}

pub fn a_move_command_walks_the_player_to_the_target(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let start = me_pos(&mut a);
    let arrived = travel(stage, &mut a, walk_target(), 30.0);
    assert!(
        arrived,
        "player should reach the target (start {start:?}, ended {:?})",
        me_pos(&mut a),
    );
}

pub fn attacking_damages_then_kills_the_target(stage: &mut dyn Stage, dead_action: u8) {
    let mut attacker = stage.player();
    let mut victim = stage.player();
    let victim_id = victim.client_id();
    let full = my_health(&mut victim);

    let strike = |attacker: &mut Box<dyn Player>| {
        if let Some(seen) = attacker.view().player_of(victim_id) {
            let entity = seen.entity;
            attacker.attack(entity);
        }
    };
    assert!(
        eventually(stage, 10.0, || {
            strike(&mut attacker);
            my_health(&mut victim) < full
        }),
        "the victim should take damage",
    );
    assert!(
        eventually(stage, 20.0, || {
            strike(&mut attacker);
            my_health(&mut victim) <= 0.0
        }),
        "sustained attacks should kill the victim",
    );
    let view = victim.view();
    assert_eq!(
        view.me().and_then(|me| me.action),
        Some(dead_action),
        "a killed player animates as dead",
    );
}

/// Hits land mid-swing (the attacker shows the attack action on the damage tick), and
/// consecutive hits are separated by a visible recovery where the attacker stands idle.
pub fn swings_are_discrete_with_recovery_between(stage: &mut dyn Stage) {
    let mut attacker = stage.player();
    let mut victim = stage.player();
    let victim_id = victim.client_id();
    let mut last_health = my_health(&mut victim);

    if let Some(seen) = attacker.view().player_of(victim_id) {
        let entity = seen.entity;
        attacker.attack(entity);
    }

    let mut hits = 0;
    let mut idled_since_hit = false;
    for _ in 0..300 {
        stage.step(1.0 / 30.0);
        let action = attacker.view().me().and_then(|me| me.action);
        let health = my_health(&mut victim);
        if health < last_health {
            assert_eq!(
                action,
                Some(world::ACTION_ATTACK),
                "damage must land mid-swing",
            );
            if hits > 0 {
                assert!(
                    idled_since_hit,
                    "consecutive swings must be separated by recovery",
                );
            }
            hits += 1;
            idled_since_hit = false;
            if hits == 2 {
                return;
            }
        }
        idled_since_hit |= action == Some(world::ACTION_IDLE);
        last_health = health;
    }
    panic!("two swings should land within the budget");
}

pub fn a_dead_target_is_not_attackable(stage: &mut dyn Stage, attack_action: u8) {
    let mut attacker = stage.player();
    let mut victim = stage.player();
    let victim_id = victim.client_id();

    assert!(
        eventually(stage, 30.0, || {
            if let Some(seen) = attacker.view().player_of(victim_id) {
                let entity = seen.entity;
                attacker.attack(entity);
            }
            my_health(&mut victim) <= 0.0
        }),
        "the victim dies first",
    );
    stage.step(1.0);

    let corpse = attacker
        .view()
        .player_of(victim_id)
        .map(|seen| seen.entity)
        .expect("the corpse stays visible");
    attacker.attack(corpse);
    for _ in 0..30 {
        stage.step(0.1);
        assert_ne!(
            attacker.view().me().and_then(|me| me.action),
            Some(attack_action),
            "attacking a corpse must not engage",
        );
    }
}

pub fn respawn_restores_a_dead_player_at_spawn(stage: &mut dyn Stage) {
    let mut attacker = stage.player();
    let mut victim = stage.player();
    let victim_id = victim.client_id();

    assert!(
        eventually(stage, 30.0, || {
            if let Some(seen) = attacker.view().player_of(victim_id) {
                let entity = seen.entity;
                attacker.attack(entity);
            }
            my_health(&mut victim) <= 0.0
        }),
        "victim should die first",
    );

    victim.respawn();
    assert!(
        eventually(stage, 5.0, || my_health(&mut victim) > 0.0),
        "respawn should bring the player back to life",
    );
    assert_eq!(
        my_area(&mut victim),
        Some(world::core::area::spawn_zone()),
        "respawn returns to the spawn area"
    );
}

pub fn moving_onto_a_portal_with_intent_crosses_areas(stage: &mut dyn Stage) {
    let mut a = stage.player();
    assert_eq!(my_area(&mut a), Some(world::core::area::spawn_zone()));
    assert!(
        cross_portal(stage, &mut a, home_portal(), away(), 30.0),
        "moving onto the portal should cross to its destination (ended in {:?})",
        my_area(&mut a),
    );
}

pub fn players_in_the_same_area_see_each_other(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let mut b = stage.player();
    let (a_id, b_id) = (a.client_id(), b.client_id());
    assert!(
        eventually(stage, 5.0, || a.view().player_of(b_id).is_some()),
        "a should see b nearby",
    );
    assert!(
        eventually(stage, 5.0, || b.view().player_of(a_id).is_some()),
        "b should see a nearby",
    );
}

pub fn crossing_areas_removes_a_player_from_others_view(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let mut b = stage.player();
    let a_id = a.client_id();
    assert!(
        eventually(stage, 5.0, || b.view().player_of(a_id).is_some()),
        "b sees a before a leaves",
    );
    assert!(
        cross_portal(stage, &mut a, home_portal(), away(), 30.0),
        "a should cross the portal",
    );
    assert!(
        eventually(stage, 5.0, || b.view().player_of(a_id).is_none()),
        "b (at home) must no longer see a (in the destination area)",
    );
}

pub fn disconnecting_removes_a_player_from_others_view(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let mut b = stage.player();
    let a_id = a.client_id();
    assert!(
        eventually(stage, 5.0, || b.view().player_of(a_id).is_some()),
        "b sees a while connected",
    );
    drop(a);
    assert!(
        eventually(stage, 10.0, || b.view().player_of(a_id).is_none()),
        "b must not see a after a disconnects",
    );
}

// The standalone stage proves the server refuses the announcement; the browser stage proves
// the site's role gate never boots the game at all.
pub fn spectating_requires_the_spectate_role(stage: &mut dyn Stage) {
    let _a = stage.player();
    let mut s = stage.unentitled_spectator();
    stage.step(1.0);
    let view = s.view();
    assert!(view.me().is_none(), "no anchor for an unentitled client");
    assert!(
        view.actors.is_empty(),
        "an unentitled client sees nothing, saw {:?}",
        view.actors,
    );
}

pub fn the_spectate_role_grants_spectating(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let a_id = a.client_id();
    let mut s = stage.spectator();
    let view = s.view();
    assert!(view.spectating, "the spectator holds a spectate anchor");
    assert!(
        view.me().is_some_and(|me| !me.actor),
        "the anchor is not a rendered actor",
    );
    assert!(
        eventually(stage, 5.0, || s.view().player_of(a_id).is_some()),
        "a free spectator sees the players in the area",
    );
    assert!(
        s.view()
            .player_of(a_id)
            .is_some_and(|p| p.name.as_deref().is_some_and(|name| !name.is_empty())),
        "the roster lists players by name",
    );
}

pub fn players_never_see_spectators(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let a_id = a.client_id();
    let mut s = stage.spectator();
    s.watch(a_id);
    let s_id = s.view().client.expect("spectator is connected");
    stage.step(1.0);
    let view = a.view();
    assert!(
        view.actors.iter().all(|seen| seen.owner != Some(s_id)),
        "the watched player must not see the spectator's anchor",
    );
    assert!(
        view.actors.iter().all(|seen| !seen.spectate),
        "no spectate marker may leak into a player's world",
    );
}

pub fn a_spectator_follows_its_player_through_a_portal(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let a_id = a.client_id();
    let mut s = stage.spectator();
    s.watch(a_id);
    assert!(
        eventually(stage, 5.0, || s.view().watching == Some(a_id)),
        "spectator locks onto its player",
    );

    assert!(
        cross_portal(stage, &mut a, home_portal(), away(), 30.0),
        "the watched player should cross the portal",
    );
    assert!(
        eventually(stage, 10.0, || my_area(&mut s) == Some(away())),
        "the spectator's anchor should follow through the portal",
    );
    assert!(
        eventually(stage, 5.0, || s.view().player_of(a_id).is_some()),
        "the spectator still sees the watched player after the crossing",
    );
}

pub fn spectators_see_players_beyond_view_distance(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let mut b = stage.player();
    let (a_id, b_id) = (a.client_id(), b.client_id());
    let mut s = stage.spectator();

    assert!(
        cross_portal(stage, &mut b, home_portal(), away(), 30.0),
        "b should cross the portal",
    );
    s.watch(a_id);
    assert!(
        eventually(stage, 5.0, || s.view().watching == Some(a_id)),
        "spectator locks onto a",
    );
    assert!(
        cross_portal(stage, &mut a, home_portal(), away(), 30.0),
        "a should cross the portal",
    );
    assert!(
        eventually(stage, 10.0, || my_area(&mut s) == Some(away())),
        "the spectator should follow a through the portal",
    );

    let far = travel(stage, &mut a, far_from_exit(), 60.0);
    assert!(
        far,
        "a should walk away from the portal exit (stalled at {:?}, health {:?})",
        me_pos(&mut a),
        a.view().me().and_then(|me| me.health),
    );
    let b_seen = s.view().player_of(b_id).map(|seen| seen.pos);
    let a_at = me_pos(&mut a);
    // The anchor rides on a, so anchor↔b separation is a↔b.
    let apart = b_seen.is_some_and(|b_at| a_at.distance(b_at) > world::VIEW_DISTANCE.0);
    assert!(
        apart,
        "the spectator (following a at {a_at:?}) must still see b (at {b_seen:?}) beyond view distance",
    );
}

pub fn aggressive_npcs_attack_a_nearby_player(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let full = my_health(&mut a);
    assert!(
        eventually(stage, 30.0, || my_health(&mut a) < full),
        "hostile NPCs should eventually damage a player standing at spawn",
    );
}

// The spawn-area npc whose reward table guarantees a consumable drop, by display name —
// derived from the content tables so the scenarios follow the data.
fn loot_npc_name() -> &'static str {
    use world::features::items::{self, ItemKind};
    use world::features::npc;
    use world::features::rewards::{RewardKind, rewards_for};
    let spawn = world::spawn_zone();
    let def = npc::spawns()
        .iter()
        .filter(|row| row.area.0 == spawn)
        .map(|row| row.npc)
        .find(|&def| {
            rewards_for(def).any(|reward| {
                matches!(
                    reward.kind,
                    RewardKind::Item { item, chance: None }
                        if matches!(items::item(item).kind, ItemKind::Consumable { .. })
                )
            })
        })
        .expect("a spawn-area npc must guarantee a consumable drop");
    &npc::def(def).display_name
}

fn consumable_in(items: &[world::ItemId]) -> Option<(u32, f32)> {
    use world::features::items::{self, ItemKind};
    items
        .iter()
        .enumerate()
        .find_map(|(slot, &item)| match items::item(item).kind {
            ItemKind::Consumable { health_bonus } => Some((slot as u32, health_bonus)),
            _ => None,
        })
}

// Kill the named npc kind until loot lands in the inventory, respawning if it kills us first.
fn hunt_for_loot(
    stage: &mut dyn Stage,
    player: &mut Box<dyn Player>,
    name: &str,
    seconds: f32,
) -> bool {
    eventually(stage, seconds, || {
        let view = player.view();
        let Some(me) = view.me() else {
            return false;
        };
        if !me.inventory.is_empty() {
            return true;
        }
        if me.health.unwrap_or(0.0) <= 0.0 {
            player.respawn();
            return false;
        }
        let at = me.pos;
        let target = view
            .npcs()
            .filter(|npc| npc.name.as_deref() == Some(name))
            .filter(|npc| npc.health.is_none_or(|health| health > 0.0))
            .min_by(|p, q| p.pos.distance(at).total_cmp(&q.pos.distance(at)))
            .map(|npc| npc.entity);
        if let Some(entity) = target {
            player.attack(entity);
        }
        false
    })
}

pub fn killing_an_npc_grants_xp_and_loot(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let before = a.view().me().and_then(|me| me.xp).unwrap_or(0);
    assert!(
        hunt_for_loot(stage, &mut a, loot_npc_name(), 90.0),
        "killing the loot npc should drop its guaranteed item",
    );
    let view = a.view();
    let me = view.me().expect("player visible");
    assert!(
        me.xp.unwrap_or(0) > before,
        "the kill should grant xp (was {before}, now {:?})",
        me.xp,
    );
}

pub fn consuming_a_potion_heals_and_destroys_it(stage: &mut dyn Stage) {
    let mut a = stage.player();
    assert!(
        hunt_for_loot(stage, &mut a, loot_npc_name(), 90.0),
        "the player should loot a kill first",
    );
    let (slot, bonus) = {
        let view = a.view();
        let me = view.me().expect("player visible");
        consumable_in(&me.inventory).expect("the guaranteed drop is a consumable")
    };

    // Drink inside a health window with full headroom for the heal and margin against the
    // hits still landing: hostile npcs around spawn provide the damage.
    let hurt = eventually(stage, 60.0, || {
        let view = a.view();
        let Some(me) = view.me() else {
            return false;
        };
        let health = me.health.unwrap_or(0.0);
        if health <= 0.0 {
            a.respawn();
            return false;
        }
        health >= bonus && me.max.is_some_and(|max| health <= max - bonus)
    });
    assert!(hurt, "hostile npcs should wear the player into the window");

    let before_len = a.view().me().map_or(0, |me| me.inventory.len());
    let at_use = a.view().me().and_then(|me| me.health).unwrap_or(0.0);
    let consumed = eventually(stage, 10.0, || {
        a.use_item(slot);
        a.view()
            .me()
            .is_some_and(|me| me.inventory.len() < before_len)
    });
    assert!(consumed, "using a consumable destroys the instance");
    let after = a.view().me().and_then(|me| me.health).unwrap_or(0.0);
    assert!(
        after > at_use,
        "the potion should heal ({at_use} -> {after})",
    );
}

pub fn inventories_are_private(stage: &mut dyn Stage) {
    let mut a = stage.player();
    let mut b = stage.player();
    let a_id = a.client_id();
    assert!(
        hunt_for_loot(stage, &mut a, loot_npc_name(), 90.0),
        "a should loot a kill",
    );
    let near_b = me_pos(&mut b);
    assert!(
        travel(stage, &mut a, near_b, 30.0),
        "a should return next to b",
    );
    assert!(
        eventually(stage, 10.0, || b.view().player_of(a_id).is_some()),
        "b should see a nearby",
    );
    let view = b.view();
    let a_through_b = view.player_of(a_id).expect("b sees a");
    assert!(
        a_through_b.inventory.is_empty(),
        "a's inventory must never replicate to b (saw {:?})",
        a_through_b.inventory,
    );
    assert!(
        a.view().me().is_some_and(|me| !me.inventory.is_empty()),
        "a keeps seeing its own loot",
    );
}

pub fn a_player_can_kill_a_nearby_npc(stage: &mut dyn Stage) {
    let mut a = stage.player();
    assert!(
        eventually(stage, 5.0, || a.view().npcs().next().is_some()),
        "a should see an NPC nearby",
    );
    let (npc, full) = {
        let view = a.view();
        let target = {
            let me = view.me().expect("player visible");
            me.pos
        };
        let nearest = view
            .npcs()
            .min_by(|p, q| p.pos.distance(target).total_cmp(&q.pos.distance(target)))
            .expect("an NPC is visible");
        (nearest.entity, nearest.health.expect("npc has vitals"))
    };
    let hurt = eventually(stage, 20.0, || {
        a.attack(npc);
        a.view()
            .actors
            .iter()
            .find(|seen| seen.entity == npc)
            .and_then(|seen| seen.health)
            .is_none_or(|health| health < full)
    });
    assert!(
        hurt,
        "attacking an NPC should reduce its health (or it leaves view as it dies)",
    );
}
