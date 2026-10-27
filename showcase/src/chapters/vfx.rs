use driver::{
    BUDGET, LONG, Player, aiming, beside_other_player, clickable_point, ground_near, id,
    nearest_npc, npc_is, other_player_within, pack_ground, played, played_on, playing,
    status_on_me,
};
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::core::vfx::VfxSite;
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "vfx",
    title: "Visual effects",
    summary: "Shaders that know their geometry and their targets, generic ones with parameters, and effects a status carries",
    play,
};

const AREA: &str = "island";
const PREY: &str = "skeleton_rogue";

fn play(shoot: &mut Shoot) {
    let ring = pack_ground(AREA, PREY);
    // Beside the pack rather than above or below it, so the pack stays in the wider half of the view.
    let ground: Vec<_> = ground_near(AREA, ring, Meters(4.0)..=Meters(6.5))
        .into_iter()
        .filter(|ground| (ground.y - ring.y).abs() <= 1.5)
        .collect();
    let mine = *ground.first().expect("ground beside the pack");
    let theirs = *ground
        .iter()
        .find(|ground| ground.distance(mine) >= Meters(1.0))
        .expect("room for two beside the pack");
    let player = shoot.join_as("arcanist", Some(place(mine)));
    let mut friend = shoot.join_as("armsman", Some(place(theirs)));
    friend.in_world();
    let mut me = shoot.roll(player);
    friend.walk_toward(beside_other_player);
    me.caption("Every effect is a vfx row: layers of sprite sheets and shaders, played on a site");
    me.caption(
        "A path site threads the targets: Chain Lightning arcs from you through each one it hit, nearest first",
    );
    let pack = me.wait_for("the pack closing in", closing_pack);
    me.cast_at(InputAction::Hotbar3, pack, |snapshot| {
        played(snapshot, "chain_lightning")
            .iter()
            .any(|bolt| matches!(&bolt.site, VfxSite::Path(stops) if stops.len() >= 3))
    });
    me.hold();
    me.caption(
        "A shader on an area reads its geometry: Fireball's flames fill the circle exactly, and whoever stands in it burns bright inside the fire",
    );
    let healthiest = me.wait_for("the pack close by", healthiest_close_prey);
    me.cast_at(InputAction::Hotbar1, healthiest, |snapshot| {
        !played(snapshot, "fireball_blast").is_empty()
    });
    me.hold();
    me.caption(
        "The Burning it leaves is a status with effects of its own: embers rise from whoever it holds",
    );
    me.wait_until("embers on the pack", |snapshot| {
        !played(snapshot, "embers").is_empty()
    });
    me.hold();
    me.act(InputAction::Hotbar6);
    me.wait_until("warded", |snapshot| status_on_me(snapshot, "warded"));
    me.caption(
        "A site can be an actor: Ward is a shader working on the character's own sprite, an outline and a sheen",
    );
    me.hold();
    me.caption(
        "A rectangle: Sanctuary's light stands in its square, and the blessing it leaves carries motes of its own",
    );
    friend.walk_toward(beside_other_player);
    let ally = me.wait_for("the friend standing beside me", |snapshot| {
        other_player_within(snapshot, 4.5).filter(|ally| ally.resting)
    });
    me.cast_at(InputAction::Hotbar5, ally.at, |snapshot| {
        played_on(snapshot, "holy_motes", ally.id)
    });
    me.hold();
    me.caption(
        "Generic shaders take parameters. One swoosh is Cleave's white arc wiping across its cone…",
    );
    friend.act(InputAction::Hotbar1);
    friend.wait_until("Cleave armed", aiming);
    let foe = friend.wait_for("a rogue in the friend's view", foe_in_view);
    friend.point_at_ground(foe);
    friend.click();
    me.wait_until("the cleave", |snapshot| {
        !played(snapshot, "cleave_swoosh").is_empty()
    });
    me.hold();
    me.caption(
        "…and, in red, thinner and at an angle, Sunder's slash across its target's own sprite",
    );
    sunder_nearest(&mut friend);
    me.wait_until("the slash", |snapshot| {
        !played(snapshot, "sunder_slash").is_empty()
    });
    me.hold();
    me.caption(
        "A status ending is an effect too: the moment the ward's time runs out, it shatters",
    );
    me.wait_within("the ward broken", LONG, |snapshot| {
        (!status_on_me(snapshot, "warded")).then_some(())
    });
    if me
        .try_wait_for(BUDGET, |snapshot| {
            (!played(snapshot, "ward_break").is_empty()).then_some(())
        })
        .is_some()
    {
        me.wait_until("the shards settled", |snapshot| {
            playing(snapshot, "ward_break").is_empty()
        });
    }
    me.hold();
}

// Where the pack is converging on the player, once two or more have come close, so the chain's
// circle still holds them when they arrive.
fn closing_pack(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    let me = snapshot.me.as_ref()?.at;
    let close: Vec<_> = snapshot
        .actors
        .iter()
        .filter(|actor| npc_is(actor, PREY) && actor.health > 0.0)
        .filter(|actor| actor.at.distance_to(me) <= 4.0)
        .map(|actor| actor.at)
        .collect();
    let count = close.len() as f32;
    (count >= 2.0).then(|| {
        let sum = close
            .iter()
            .fold(Pos::zero(), |sum, at| sum + at.to_vector());
        Pos::new(sum.x / count, sum.y / count)
    })
}

// The healthiest of the pack close by, so a blast centred on it leaves someone burning instead of
// felling everyone it holds.
fn healthiest_close_prey(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    let me = snapshot.me.as_ref()?.at;
    snapshot
        .actors
        .iter()
        .filter(|actor| npc_is(actor, PREY) && actor.health > 0.0)
        .filter(|actor| actor.at.distance_to(me) <= 4.0)
        .max_by(|a, b| a.health.total_cmp(&b.health))
        .map(|actor| actor.at)
}

// The friend arms Sunder and lands it on the nearest monster standing clear of the crowd on him,
// so the slash across its sprite is in the open.
fn sunder_nearest(friend: &mut Player) {
    friend.cast_on(
        InputAction::Hotbar3,
        |snapshot| {
            let me = snapshot.me.as_ref()?.at;
            let standing = || {
                snapshot
                    .actors
                    .iter()
                    .filter(|actor| npc_is(actor, PREY) && actor.health > 0.0)
            };
            let nearest = |mut among: Vec<&ProbeBody>| {
                among.sort_by(|a, b| a.at.distance_to(me).total_cmp(&b.at.distance_to(me)));
                among.first().map(|actor| actor.id)
            };
            let clear = standing()
                .filter(|actor| (2.5..=5.0).contains(&actor.at.distance_to(me)))
                .collect();
            nearest(clear).or_else(|| nearest(standing().collect()))
        },
        |snapshot| !played(snapshot, "sunder_slash").is_empty(),
    );
}

fn place(at: Pos<Meters>) -> Place {
    Place { area: id(AREA), at }
}

/// The nearest living prey on the friend's screen: where the pack is now, which the cone sweeps
/// at, rather than where it roams, which may lie out of the friend's view.
fn foe_in_view(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    let foe = nearest_npc(snapshot, PREY)?.at;
    clickable_point(snapshot, foe).map(|_| foe)
}
