use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AttackSpeed(pub f32);

#[derive(Clone, Copy)]
pub struct AttackSpeedStat;

inventory::submit! {
    &AttackSpeedStat as &dyn Stat
}

impl Scalar for AttackSpeedStat {
    type Component = AttackSpeed;
    const NAME: &'static str = "AttackSpeed";
    const LABEL: &'static str = "Attack Speed";
    fn read(stat: &AttackSpeed) -> f32 {
        stat.0
    }
    fn make(value: f32) -> AttackSpeed {
        AttackSpeed(value)
    }
}
