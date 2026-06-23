use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::time::Seconds;
use crate::protocol::{Owner, Vitals};

const HP_REGEN_INTERVAL: Seconds = Seconds(10.0);
const HP_REGEN_AMOUNT: f32 = 5.0;

#[derive(Resource, Default)]
pub struct RegenAt(Seconds);

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
        if !vitals.is_dead() {
            vitals.heal(HP_REGEN_AMOUNT);
        }
    }
}
