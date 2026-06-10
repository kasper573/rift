use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::math::Seconds;
use crate::protocol::{Owner, Vitals};

const HP_REGEN_INTERVAL: Seconds = Seconds(10.0);
const HP_REGEN_AMOUNT: f32 = 5.0;

#[derive(Resource, Default)]
pub struct RegenAt(Seconds);

// Players are exactly the owned entities; NPCs regenerate on respawn instead.
pub fn regen(
    time: Res<Time>,
    mut last: ResMut<RegenAt>,
    mut players: Query<&mut Vitals, With<Owner>>,
) {
    let now = Seconds(time.elapsed_secs());
    if now - last.0 < HP_REGEN_INTERVAL {
        return;
    }
    last.0 = now;
    for mut vitals in &mut players {
        if vitals.health > 0.0 {
            vitals.health = (vitals.health + HP_REGEN_AMOUNT).min(vitals.max);
        }
    }
}
