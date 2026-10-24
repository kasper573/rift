use driver::{
    UiTarget, aiming, beside_other_player, clickable_point, content, east_of, id, input_name,
    nearest_npc, npc_is, other_player, other_player_within, pack_ground, played, played_on,
    status_on_me,
};
use engine::core::geometry::Shape;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::core::vfx::VfxSite;
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};
use engine::systems::skill::SkillDef;

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "casting",
    title: "Casting",
    summary: "Every way a skill can be aimed: yourself, one ally, one enemy, and areas of every shape",
    play,
};

const AREA: &str = "glimmercap";
const PREY: &str = "myconid";

fn play(shoot: &mut Shoot) {
    let ring = pack_ground(AREA, PREY);
    let player = shoot.join_as("arcanist", Some(east_of(AREA, ring, 5.0)));
    let mut friend = shoot.join_as("armsman", Some(east_of(AREA, ring, 6.0)));
    friend.in_world();
    let mut me = shoot.roll(player);
    friend.walk_toward(beside_other_player);
    me.caption(
        "Every skill has one targeting kind. On yourself: Ward fires the moment its key is pressed",
    );
    me.act(InputAction::Hotbar6);
    me.wait_until("warded", |snapshot| status_on_me(snapshot, "warded"));
    me.hold();
    me.caption(&format!(
        "One ally: Mend arms, and {} must land on a friend. Allies are the other players",
        input_name(InputAction::Interact)
    ));
    me.act(InputAction::Hotbar4);
    me.wait_until("Mend armed", aiming);
    let ally = me.wait_for("the friend standing still", |snapshot| {
        other_player(snapshot).cloned().filter(still)
    });
    me.point_at_tile(ally.aim);
    me.hold();
    me.click();
    me.wait_until("the friend mended", |snapshot| {
        played_on(snapshot, "mend_glow", ally.id)
    });
    me.hold();
    me.caption(&format!(
        "One enemy: {} on a monster uses the basic attack, Spark; any other single-target skill is aimed the same way",
        input_name(InputAction::Interact)
    ));
    me.fight(
        |_, actor| npc_is(actor, PREY),
        |snapshot| !played(snapshot, "spark_hit").is_empty(),
    );
    me.hold();
    me.caption(
        "An area of enemies, a circle: Fireball lands on the point you aim at, walking into range first if it must",
    );
    aim_then_cast(&mut me, InputAction::Hotbar1, ring, |snapshot| {
        !played(snapshot, "fireball_blast").is_empty()
    });
    me.hold();
    me.caption(
        "Chain Lightning's circle picks out everything hostile inside it, and the bolt runs through each one",
    );
    let crowd = me.wait_for("myconids close enough to chain", |snapshot| {
        crowd(snapshot, chain_radius())
    });
    aim_then_cast(
        &mut me,
        InputAction::Hotbar3,
        crowd,
        chained_through_several,
    );
    me.hold();
    me.caption(
        "An area of allies, a rectangle: Sanctuary blesses everyone standing in it, the caster included",
    );
    friend.walk_toward(beside_other_player);
    let ally = me.wait_for("the friend standing beside me", |snapshot| {
        other_player_within(snapshot, 4.5).filter(still)
    });
    aim_then_cast(&mut me, InputAction::Hotbar5, ally.at, |snapshot| {
        played_on(snapshot, "holy_motes", ally.id)
    });
    me.hold();
    me.caption("And a cone: the armsman's Cleave sweeps the arc in front of him");
    friend.act(InputAction::Hotbar1);
    friend.wait_until("Cleave armed", aiming);
    let foe = friend.wait_for("a myconid in the friend's view", foe_in_view);
    friend.point_at_tile(foe);
    friend.click();
    me.wait_until("the cleave", |snapshot| {
        !played(snapshot, "cleave_swoosh").is_empty()
    });
    me.hold();
    me.caption("A passive, Arcane Mind, is aimed at nobody: it holds for as long as it is known");
    me.act(InputAction::ToggleSkills);
    me.hover_ui(UiTarget::starting("Arcane Mind node"));
    me.hold();
    me.act(InputAction::ToggleSkills);
}

// The viewer sees where the skill is aimed before the click lands it.
fn aim_then_cast(
    me: &mut Star,
    slot: InputAction,
    at: Pos<Meters>,
    done: impl Fn(&ProbeSnapshot) -> bool,
) {
    me.aim_skill(slot, at);
    me.hold();
    me.cast(done);
}

/// The prey with the most others inside a circle of `radius` around it, once that is at least one
/// other: the pack fights back after the fireball and runs at the caster, so the circle goes where
/// it is now rather than where it roams.
fn crowd(snapshot: &ProbeSnapshot, radius: Meters) -> Option<Pos<Meters>> {
    let prey: Vec<Pos<Meters>> = snapshot
        .actors
        .iter()
        .filter(|actor| actor.health > 0.0 && npc_is(actor, PREY))
        .map(|actor| actor.at)
        .collect();
    let around = |at: Pos<Meters>| {
        prey.iter()
            .filter(|other| other.distance_to(at) <= radius.0)
            .count()
    };
    prey.iter()
        .copied()
        .max_by_key(|at| around(*at))
        .filter(|at| around(*at) >= 2)
}

/// A friend aimed at stands where the click lands, rather than walking out from under it.
fn still(body: &ProbeBody) -> bool {
    body.resting.is_some()
}

fn chain_radius() -> Meters {
    match id::<SkillDef>("chain_lightning").get(content()).area {
        Some(Shape::Circle { radius }) => radius,
        area => panic!("Chain Lightning strikes a circle, not {area:?}"),
    }
}

fn chained_through_several(snapshot: &ProbeSnapshot) -> bool {
    played(snapshot, "chain_lightning")
        .iter()
        .any(|bolt| matches!(&bolt.site, VfxSite::Path(stops) if stops.len() >= 3))
}

/// The nearest living prey on the friend's screen: where the pack is now, which the cone sweeps
/// at, rather than where it roams, which may lie out of the friend's view.
fn foe_in_view(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    let foe = nearest_npc(snapshot, PREY)?.at;
    clickable_point(snapshot, foe).map(|_| foe)
}
