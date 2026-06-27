use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MovementSpeed(pub f32);

#[derive(Clone, Copy)]
pub struct MovementSpeedStat;

inventory::submit! {
    &MovementSpeedStat as &dyn Stat
}

impl Scalar for MovementSpeedStat {
    type Component = MovementSpeed;
    const NAME: &'static str = "MovementSpeed";
    const LABEL: &'static str = "Move Speed";
    fn read(stat: &MovementSpeed) -> f32 {
        stat.0
    }
    fn make(value: f32) -> MovementSpeed {
        MovementSpeed(value)
    }
}
