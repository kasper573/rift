use driver::{
    BUDGET, LONG, Player, aiming, beside_other_player, id, npc_is, other_player_within,
    pack_ground, played, played_on, playing, status_on_me,
};
use engine::core::math::{Offset, Pos};
use engine::core::tiling::Tiles;
use engine::core::vfx::VfxSite;
use engine::systems::area::AreaDef;
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "vfx",
    title: "Visual effects",
    summary: "Shaders that know their geometry and their targets, generic ones with parameters, and effects a status carries",
    play,
};

const AREA: &str = "glimmercap";
const PREY: &str = "myconid";

fn play(shoot: &mut Shoot) {
    let ring = pack_ground(AREA, PREY);
    let player = shoot.join_as("arcanist", Some(east_of(ring, 5.0)));
    let mut friend = shoot.join_as("armsman", Some(east_of(ring, 6.0)));
    friend.in_world();
    let mut me = shoot.roll(player);
    friend.walk_toward(beside_other_player);
    me.caption_top(
        "Every effect is a vfx row: layers of sprite sheets, floating text and shaders, played on a site",
    );
    me.caption_top(
        "A path site threads the targets: Chain Lightning arcs from you through each one it hit, nearest first",
    );
    cast_at(&mut me, InputAction::Hotbar3, ring, |snapshot| {
        played(snapshot, "chain_lightning")
            .iter()
            .any(|bolt| matches!(&bolt.site, VfxSite::Path(stops) if stops.len() >= 3))
    });
    me.hold();
    me.caption_top(
        "A shader on an area reads its geometry: Fireball's flames fill the circle exactly, whatever its radius.",
    );
    let here = me.snapshot().me.expect("the player").at;
    cast_at(&mut me, InputAction::Hotbar1, here, |snapshot| {
        !played(snapshot, "fireball_blast").is_empty()
    });
    me.hold();
    me.caption_top(
        "The Burning it leaves is a status with effects of its own: embers rise from whoever it holds",
    );
    me.wait_until("embers on the pack", |snapshot| {
        !played(snapshot, "embers").is_empty()
    });
    me.hold();
    me.caption_top(
        "A line: Frost Lance thrusts the length you aimed, bursts into ice at the end and crumbles behind",
    );
    cast_at(&mut me, InputAction::Hotbar2, ring, |snapshot| {
        !played(snapshot, "frost_lance").is_empty()
    });
    me.hold();
    me.act(InputAction::Hotbar6);
    me.wait_until("warded", |snapshot| status_on_me(snapshot, "warded"));
    me.caption_top(
        "A site can be an actor: Ward is a shader working on the character's own sprite, an outline and a sheen",
    );
    me.hold();
    me.caption_top(
        "A rectangle: Sanctuary's light stands in its square, and the blessing it leaves carries motes of its own",
    );
    friend.walk_toward(beside_other_player);
    let ally = me.wait_for("the friend beside me", |snapshot| {
        other_player_within(snapshot, 4.5)
    });
    cast_at(&mut me, InputAction::Hotbar5, ally.at, |snapshot| {
        played_on(snapshot, "holy_motes", ally.id)
    });
    me.hold();
    me.caption_top(
        "Generic shaders take parameters. One swoosh is Cleave's white arc wiping across its cone…",
    );
    friend.act(InputAction::Hotbar1);
    friend.wait_until("Cleave armed", aiming);
    friend.point_at_tile(ring);
    friend.click();
    me.wait_until("the cleave", |snapshot| {
        !played(snapshot, "cleave_swoosh").is_empty()
    });
    me.hold();
    me.caption_top(
        "…and, in red, thinner and at an angle, Sunder's slash across its target's own sprite",
    );
    sunder_nearest(&mut friend);
    me.wait_until("the slash", |snapshot| {
        !played(snapshot, "sunder_slash").is_empty()
    });
    me.hold();
    me.caption_top(
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

fn east_of(spot: Pos<Tiles>, by: f32) -> Place {
    Place {
        area: id::<AreaDef>(AREA),
        at: spot + Offset::new(by, 1.0),
    }
}

fn cast_at(
    me: &mut Star,
    slot: InputAction,
    at: Pos<Tiles>,
    done: impl Fn(&ProbeSnapshot) -> bool,
) {
    me.act(slot);
    me.wait_until("the skill armed", aiming);
    me.point_at_tile(at);
    me.click();
    me.wait_until("the cast landed", done);
}

// The friend arms Sunder and lands it on the nearest monster standing clear of the crowd on him,
// so the slash across its sprite is in the open.
fn sunder_nearest(friend: &mut Player) {
    friend.act(InputAction::Hotbar3);
    friend.wait_until("Sunder armed", aiming);
    let target = friend.wait_for("a monster in the open", |snapshot| {
        let me = snapshot.me.as_ref()?.at;
        let standing = || {
            snapshot
                .actors
                .iter()
                .filter(|actor| npc_is(actor, PREY) && actor.health > 0.0)
        };
        let nearest = |mut among: Vec<&ProbeBody>| {
            among.sort_by(|a, b| a.at.distance_to(me).total_cmp(&b.at.distance_to(me)));
            among.first().map(|actor| actor.at)
        };
        let clear = standing()
            .filter(|actor| (2.5..=5.0).contains(&actor.at.distance_to(me)))
            .collect();
        nearest(clear).or_else(|| nearest(standing().collect()))
    });
    friend.point_at_tile(target);
    friend.click();
}
