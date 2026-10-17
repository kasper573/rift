use std::cell::Cell;
use std::collections::HashSet;
use std::time::Duration;

use bevy_ecs::entity::Entity;
use driver::{
    Player, UiTarget, WaitBudget, area_key, area_named, carried, clickable_point, holding, npc_is,
    resident_home,
};
use engine::core::math::{Offset, Pos};
use engine::core::tiling::{TilePos, Tiles};
use engine::systems::actor::Action;
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeGroundItem, ProbeSnapshot};

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "combat",
    title: "Combat",
    summary: "Fight monsters, loot what they drop, grow stronger and run for town",
    play,
};

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as(
        "swordsman",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    let mut me = shoot.roll(player);
    me.caption("Town is a safe zone: monsters roam the wilds beyond it");
    // Read in town: out there, monsters don't wait for the viewer.
    me.hold();
    me.look_out(PREY, |snapshot| nearest_prey(snapshot).is_some());
    me.caption(
        "Hover a monster to target it; a click uses the basic attack in the slot by your hotbar",
    );
    let loot = slay(&mut me);
    me.wait_until("the kill's XP in the feed", |snapshot| {
        snapshot
            .history
            .records
            .iter()
            .any(|record| record.text.ends_with(" XP"))
    });
    me.caption("Small news collects in the corner: experience, loot, gone in a few seconds");
    me.caption(
        "Monsters drop loot — click it to pick it up. The same thing again counts up on one row",
    );
    let before = carried(&me.snapshot());
    for item in &loot {
        me.pick_up(item.id, LOOTING);
    }
    me.wait_until("some of the loot in the bag", |snapshot| {
        carried(snapshot) > before
    });

    me.caption("Press I to open the inventory");
    me.act(InputAction::ToggleInventory);
    me.find_ui("Inventory");
    me.find_ui(UiTarget::image(GREATER_POTION));
    let potions = holding(&me.snapshot(), "greater_health_potion");
    me.caption("Double-click a potion to drink it");
    me.act_on_ui(InputAction::UseItem, UiTarget::image(GREATER_POTION));
    me.wait_until("the potion drunk", |snapshot| {
        holding(snapshot, "greater_health_potion") < potions
    });
    me.caption("Kills earn experience — see the xp under your name");
    me.hold();
    me.act(InputAction::ToggleInventory);

    me.caption("Pick a fight, then run for town");
    let chaser = provoke(&mut me);
    run_to_town(&mut me);
    me.caption("It gives up the chase at the edge: monsters never set foot in a safe zone");
    chase_ends(&mut me, &chaser);
}

const GREATER_POTION: &str = "icons/potion/red_potion_3.png";

// Loot lying under a monster can't be clicked until it moves or falls, so it's given a while.
const LOOTING: WaitBudget = WaitBudget {
    time: Duration::from_secs(20),
    frames: 1200,
};
// Loot lands the moment a monster falls, give or take a round trip.
// They fight only once attacked, so they don't crowd the loot they always drop.
const PREY: &str = "myconid";
const DROPPING: WaitBudget = WaitBudget {
    time: Duration::from_secs(2),
    frames: 120,
};

// Fights until a monster falls, and returns what it dropped.
fn slay(me: &mut Player) -> Vec<ProbeGroundItem> {
    let fallen = fallen(&me.snapshot());
    let fell = Cell::new(None);
    me.fight(
        |_, actor| is_prey(actor),
        |snapshot| {
            let felled = snapshot
                .actors
                .iter()
                .find(|actor| actor.health <= 0.0 && !actor.player && !fallen.contains(&actor.id));
            fell.set(felled.map(|it| it.at));
            felled.is_some()
        },
    );
    let fell = fell.get().expect("a monster fell");
    me.wait_within("what it dropped", DROPPING, |snapshot| {
        let near: Vec<ProbeGroundItem> = snapshot
            .items
            .iter()
            .filter(|item| item.at.distance(fell) < Tiles(2.5))
            .cloned()
            .collect();
        (!near.is_empty()).then_some(near)
    })
}

// Swings at the nearest monster once, so it gives chase, and returns the one that does.
fn provoke(me: &mut Player) -> ProbeBody {
    me.look_out(PREY, |snapshot| nearest_prey(snapshot).is_some());
    me.fight(
        |_, actor| is_prey(actor),
        |snapshot| {
            snapshot
                .me
                .as_ref()
                .is_some_and(|me| me.action == Action::Attack)
        },
    );
    me.wait_for("the monster that gives chase", |snapshot| {
        nearest_prey(snapshot).cloned()
    })
}

fn run_to_town(me: &mut Player) {
    let seen = me.snapshot();
    let refuge = in_town(&seen);
    me.travel_to(|_| Some(refuge), safe_here);
    me.wait_until("the player back in town", safe_here);
}

// Watches the chaser until it turns away at the edge, failing if it ever sets foot in town.
fn chase_ends(me: &mut Player, chaser: &ProbeBody) {
    let mut closest = f32::INFINITY;
    me.wait_within(
        "the chaser to give up",
        WaitBudget {
            time: Duration::from_secs(30),
            frames: 1800,
        },
        |snapshot| {
            let Some(it) = snapshot.actors.iter().find(|actor| actor.id == chaser.id) else {
                return Some(());
            };
            assert!(!safe(snapshot, it.at), "{} followed you into town", it.name);
            let away = snapshot
                .me
                .as_ref()
                .map_or(f32::INFINITY, |me| it.at.distance_to(me.at));
            closest = closest.min(away);
            let chasing = matches!(it.action, Action::Walk | Action::Run);
            (clickable_point(snapshot, it.at).is_none() || !chasing || away > closest + 2.0)
                .then_some(())
        },
    );
}

fn prey(snapshot: &ProbeSnapshot) -> Vec<&ProbeBody> {
    snapshot
        .actors
        .iter()
        .filter(|actor| is_prey(actor) && clickable_point(snapshot, actor.aim).is_some())
        .collect()
}

fn is_prey(actor: &ProbeBody) -> bool {
    npc_is(actor, PREY) && actor.health > 0.0
}

fn fallen(snapshot: &ProbeSnapshot) -> HashSet<Entity> {
    snapshot
        .actors
        .iter()
        .filter(|actor| actor.health <= 0.0)
        .map(|actor| actor.id)
        .collect()
}

fn nearest_prey(snapshot: &ProbeSnapshot) -> Option<&ProbeBody> {
    let me = snapshot.me.as_ref()?.at;
    prey(snapshot)
        .into_iter()
        .min_by(|a, b| a.at.distance_to(me).total_cmp(&b.at.distance_to(me)))
}

fn safe(snapshot: &ProbeSnapshot, at: Pos<Tiles>) -> bool {
    area_key(snapshot).is_some_and(|area| area_named(area).safe(at))
}

fn safe_here(snapshot: &ProbeSnapshot) -> bool {
    snapshot.me.as_ref().is_some_and(|me| safe(snapshot, me.at))
}

// The ground in town nearest the player, a couple of tiles in from every edge.
fn in_town(snapshot: &ProbeSnapshot) -> Pos<Tiles> {
    let me = snapshot.me.as_ref().expect("the player").at;
    let area = area_named(area_key(snapshot).expect("an area"));
    let grid = &area.grid;
    let deep = |at: Pos<Tiles>| {
        [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0)]
            .into_iter()
            .all(|(x, y)| area.safe(at + Offset::new(x, y)))
    };
    grid.nodes()
        .iter()
        .copied()
        .filter(|node| grid.component(*node) == grid.component(me) && deep(*node))
        .min_by(|a, b| a.distance_to(me).total_cmp(&b.distance_to(me)))
        .expect("ground in town")
}
