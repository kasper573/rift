use driver::{
    Player, UiTarget, aiming, clickable_point, content, east_of, ground_near, hunting_ground, id,
    input_name, nearest_npc, npc_is, pack_ground, played, played_on, player_named, status_on_me,
};
use engine::core::geometry::Shape;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::core::vfx::VfxSite;
use engine::systems::actor::skeleton::Rung;
use engine::systems::actor::{Motion, Pose};
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};
use engine::systems::script::Place;
use engine::systems::skill::SkillDef;

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "casting",
    title: "Casting",
    summary: "Every way a skill can be aimed: yourself, one ally, one enemy, and areas of every shape",
    play,
};

const AREA: &str = "island";
const PREY: &str = "skeleton";
const ARCANIST: &str = "Arcanist";
const ARMSMAN: &str = "Armsman";
const ARCHER: &str = "Archer";
/// How near the caster the fireball lands: whatever it catches is near enough to see him and fight
/// back, rather than losing him and wandering off, so the camp gathers where Chain Lightning finds
/// it.
const PROVOKED_WITHIN: Meters = Meters(4.0);
/// A monster that never fights back: nothing sets on the archer while she shoots at it, and the
/// skeletons' camp swarms with those that do.
const QUARRY: &str = "slime";

fn play(shoot: &mut Shoot) {
    let ring = pack_ground(AREA, PREY);
    let (caster_at, friend_at, hunter_at) = quiet_ground();
    let player = shoot.join_as("arcanist", Some(at(caster_at)));
    let mut friend = shoot.join_as("armsman", Some(at(friend_at)));
    friend.in_world();
    let mut hunter = shoot.join_as("archer", Some(at(hunter_at)));
    hunter.in_world();
    let mut me = shoot.roll(player);
    friend.walk_toward(beside_me);
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
        player_named(snapshot, ARMSMAN).cloned().filter(still)
    });
    me.point_at_ground(ally.aim);
    me.hold();
    me.click();
    me.wait_until("the friend mended", |snapshot| {
        played_on(snapshot, "mend_glow", ally.id)
    });
    me.hold();
    me.caption("A skill asks for a move, and the hands decide how it looks: the archer's Volley draws her bow");
    volley(&mut me, &mut hunter);
    me.walk_near(east_of(AREA, ring, 5.0).at);
    me.come_to_rest();
    friend.walk_near(east_of(AREA, ring, 6.0).at);
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
    me.hold();
    let blast = me.wait_for("foes near enough to provoke", |snapshot| {
        let me = snapshot.me.as_ref()?.at;
        densest(snapshot, fireball_radius(), |body| {
            !body.friendly && body.at.distance_to(me) <= PROVOKED_WITHIN.0
        })
        .map(|(at, _)| at)
    });
    aim_then_cast(&mut me, InputAction::Hotbar1, blast, |snapshot| {
        !played(snapshot, "fireball_blast").is_empty()
    });
    me.hold();
    me.caption(
        "Chain Lightning's circle picks out everything hostile inside it, and the bolt runs through each one",
    );
    let crowd = me.wait_for("skeletons close enough to chain", |snapshot| {
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
    friend.walk_toward(beside_me);
    let ally = me.wait_for("the friend standing beside me", |snapshot| {
        let me = snapshot.me.as_ref()?.at;
        player_named(snapshot, ARMSMAN)
            .filter(|friend| still(friend) && friend.at.distance_to(me) <= 4.5)
            .cloned()
    });
    aim_then_cast(&mut me, InputAction::Hotbar5, ally.at, |snapshot| {
        played_on(snapshot, "holy_motes", ally.id)
    });
    me.hold();
    me.caption("And a cone: the armsman's Cleave sweeps the arc in front of him");
    friend.act(InputAction::Hotbar1);
    friend.wait_until("Cleave armed", aiming);
    let foe = friend.wait_for("a skeleton in the friend's view", foe_in_view);
    friend.point_at_ground(foe);
    friend.click();
    me.wait_until("the cleave, swung as his sword's cleave", |snapshot| {
        !played(snapshot, "cleave_swoosh").is_empty()
            && player_named(snapshot, ARMSMAN).is_some_and(moves_as_its_stance_names)
    });
    me.hold();
    me.caption("A passive, Arcane Mind, is aimed at nobody: it holds for as long as it is known");
    me.act(InputAction::ToggleSkills);
    me.hover_ui(UiTarget::starting("Arcane Mind node"));
    me.hold();
    me.act(InputAction::ToggleSkills);
}

/// The archer looses at a quarry already in Volley's range, standing beside the caster, who watches
/// close up but from overhead: tilted toward the horizon, the canopy beyond his own clearing hides
/// her.
fn volley(me: &mut Star, hunter: &mut Player) {
    let range = volley_range();
    let in_range = |snapshot: &ProbeSnapshot| {
        let me = snapshot.me.as_ref()?.at;
        nearest_npc(snapshot, QUARRY)
            .map(|quarry| quarry.at)
            .filter(|at| at.distance_to(me) <= range.0 && clickable_point(snapshot, *at).is_some())
    };
    hunter.act(InputAction::Hotbar3);
    hunter.wait_until("Volley armed", aiming);
    let quarry = hunter.wait_for("a quarry in Volley's range", in_range);
    hunter.point_at_ground(quarry);
    hunter.click();
    me.wait_until("the volley drawn from the bow, in sight", |snapshot| {
        player_named(snapshot, ARCHER).is_some_and(|archer| {
            moves_as_its_stance_names(archer) && clickable_point(snapshot, archer.at).is_some()
        })
    });
    me.hold();
    me.close_in();
}

/// Where the caster, his friend and the archer start: beside the quarry's ground and far from the
/// camp, so the skills aimed at oneself and at friends, and the bow, show before anything hostile
/// comes at them. The archer stands about Volley's range from the middle of the quarry's ground,
/// the caster a step east or west of her, never between her and the camera looking north, and the
/// friend a little further off.
fn quiet_ground() -> (Pos<Meters>, Pos<Meters>, Pos<Meters>) {
    let quarry = hunting_ground(AREA, QUARRY);
    let ground = ground_near(
        AREA,
        quarry + Offset::new(0.0, -volley_range().0),
        Meters(0.0)..=Meters(5.0),
    );
    let hunter = *ground.first().expect("open ground by the quarry");
    let caster = *ground
        .iter()
        .find(|at| {
            let apart = **at - hunter;
            (1.5..=2.5).contains(&apart.x.abs()) && apart.y.abs() <= 0.5
        })
        .expect("open ground beside the archer");
    let friend = *ground
        .iter()
        .find(|at| (3.0..=4.5).contains(&at.distance_to(caster)) && at.distance_to(hunter) >= 3.0)
        .expect("open ground for the friend");
    (caster, friend, hunter)
}

fn at(ground: Pos<Meters>) -> Place {
    Place {
        area: id(AREA),
        at: ground,
    }
}

/// Whether the player named `name` plays the clip `clip`.
/// Where the friend stands to be beside the caster, seen from the friend's screen.
fn beside_me(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    player_named(snapshot, ARCANIST).map(|caster| caster.at + Offset::new(2.0, 0.0))
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

/// The foe in view with the most other foes in view within half of `radius` around it, once that
/// makes three: a mob that fights back stands still, but a camp the fireball left unprovoked
/// wanders on, and half the radius keeps a wandering three inside the circle for as long as the
/// cast takes to land.
fn crowd(snapshot: &ProbeSnapshot, radius: Meters) -> Option<Pos<Meters>> {
    densest(snapshot, Meters(radius.0 / 2.0), |body| {
        !body.friendly && clickable_point(snapshot, body.at).is_some()
    })
    .filter(|(_, count)| *count >= 3)
    .map(|(at, _)| at)
}

/// Among living monsters for which `counts` holds, the one with the most of them inside a circle
/// of `radius` around it, and how many that is, itself included.
fn densest(
    snapshot: &ProbeSnapshot,
    radius: Meters,
    counts: impl Fn(&ProbeBody) -> bool,
) -> Option<(Pos<Meters>, usize)> {
    let counted: Vec<Pos<Meters>> = snapshot
        .actors
        .iter()
        .filter(|actor| actor.health > 0.0 && actor.npc.is_some() && counts(actor))
        .map(|actor| actor.at)
        .collect();
    counted
        .iter()
        .map(|at| {
            let around = counted
                .iter()
                .filter(|other| other.distance_to(*at) <= radius.0)
                .count();
            (*at, around)
        })
        .max_by_key(|(_, around)| *around)
}

/// A friend aimed at stands where the click lands, rather than walking out from under it.
fn still(body: &ProbeBody) -> bool {
    body.resting
}

fn fireball_radius() -> Meters {
    match id::<SkillDef>("fireball").get(content()).area {
        Some(Shape::Circle { radius }) => radius,
        area => panic!("Fireball blasts a circle, not {area:?}"),
    }
}

fn chain_radius() -> Meters {
    match id::<SkillDef>("chain_lightning").get(content()).area {
        Some(Shape::Circle { radius }) => radius,
        area => panic!("Chain Lightning strikes a circle, not {area:?}"),
    }
}

fn volley_range() -> Meters {
    id::<SkillDef>("volley")
        .get(content())
        .range
        .expect("Volley reaches as far as its range")
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

/// Whether `body` plays a move in the clip its stance's own set names for it: the hands, not the
/// skill, deciding how the move looks.
fn moves_as_its_stance_names(body: &ProbeBody) -> bool {
    let Some(motion @ Motion::Move(_)) = &body.motion else {
        return false;
    };
    let build = body.appearance.model.get(content()).build.as_ref();
    body.skeleton
        .get(content())
        .named(build, body.stance, motion, Pose::Idle)
        .is_some_and(|(cell, rung)| {
            rung == Rung::Stance && cell.clips().iter().any(|named| *named.clip() == body.clip)
        })
}
