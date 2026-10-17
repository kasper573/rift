use driver::{
    Player, aiming, beside_other_player, id, npc_is, other_player, other_player_within,
    pack_ground, played, played_on, status_on_me,
};
use engine::core::math::{Offset, Pos};
use engine::core::tiling::Tiles;
use engine::core::vfx::VfxSite;
use engine::systems::area::AreaDef;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::Place;

use crate::playtest;

const AREA: &str = "glimmercap";
const PREY: &str = "myconid";

#[test]
fn every_targeting_kind_and_shape_lands_where_it_is_aimed() {
    let mut playtest = playtest();
    let ring = pack_ground(AREA, PREY);
    let mut me = playtest.join_as("arcanist", Some(east_of(ring, 5.0)));
    let mut friend = playtest.join_as("armsman", Some(east_of(ring, 6.0)));
    me.in_world();
    friend.in_world();
    friend.walk_toward(beside_other_player);

    me.act(InputAction::Hotbar6);
    me.wait_until("warded at once", |snapshot| {
        status_on_me(snapshot, "warded") && !aiming(snapshot)
    });

    me.act(InputAction::Hotbar4);
    me.wait_until("Mend armed", aiming);
    let ally = other_player(&me.snapshot())
        .expect("the friend in view")
        .clone();
    me.point_at_tile(ally.at);
    me.click();
    me.wait_until("the friend mended", |snapshot| {
        played_on(snapshot, "mend_glow", ally.id)
    });

    me.fight(
        |_, actor| npc_is(actor, PREY),
        |snapshot| !played(snapshot, "spark_hit").is_empty(),
    );

    cast_at(&mut me, InputAction::Hotbar1, ring, |snapshot| {
        played(snapshot, "fireball_blast")
            .iter()
            .any(|blast| matches!(blast.site, VfxSite::Area(_)))
    });
    cast_at(&mut me, InputAction::Hotbar3, ring, |snapshot| {
        played(snapshot, "chain_lightning")
            .iter()
            .any(|bolt| matches!(&bolt.site, VfxSite::Path(stops) if stops.len() >= 3))
    });
    cast_at(&mut me, InputAction::Hotbar2, ring, |snapshot| {
        played(snapshot, "frost_lance")
            .iter()
            .any(|lance| matches!(lance.site, VfxSite::Area(_)))
    });
    friend.walk_toward(beside_other_player);
    let ally = me.wait_for("the friend beside me", |snapshot| {
        other_player_within(snapshot, 4.5)
    });
    cast_at(&mut me, InputAction::Hotbar5, ally.at, |snapshot| {
        played_on(snapshot, "holy_motes", ally.id)
    });

    friend.act(InputAction::Hotbar1);
    friend.wait_until("Cleave armed", aiming);
    friend.point_at_tile(ring);
    friend.click();
    me.wait_until("the friend's cleave in view", |snapshot| {
        played(snapshot, "cleave_swoosh")
            .iter()
            .any(|sweep| matches!(sweep.site, VfxSite::Area(_)))
    });
}

fn east_of(spot: Pos<Tiles>, by: f32) -> Place {
    Place {
        area: id::<AreaDef>(AREA),
        at: spot + Offset::new(by, 1.0),
    }
}

fn cast_at(
    me: &mut Player,
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
