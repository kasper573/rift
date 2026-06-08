use rift::{Builder, Ctx};

use crate::core::math::Seconds;
use crate::core::protocol::{Owner, Vitals, is_dead};

const HP_REGEN_INTERVAL: Seconds = Seconds(10.0);
const HP_REGEN_AMOUNT: f32 = 5.0;

struct RegenAt(Seconds);

pub fn feature(b: &mut Builder) {
    b.start(|ctx| ctx.res.insert(RegenAt(Seconds(0.0))));
    b.system(regen);
}

fn regen(ctx: &mut Ctx) {
    let time = Seconds(ctx.time);
    let last = ctx.res.get::<RegenAt>().map_or(Seconds(0.0), |r| r.0);
    if time - last < HP_REGEN_INTERVAL {
        return;
    }
    ctx.res.insert(RegenAt(time));
    let world = &mut ctx.server.world;
    // Players are exactly the owned entities; NPCs regenerate on respawn instead.
    for entity in world.ids::<Owner>() {
        if !is_dead(world, entity) {
            world.modify::<Vitals>(entity, |v| {
                v.health = (v.health + HP_REGEN_AMOUNT).min(v.max);
            });
        }
    }
}
