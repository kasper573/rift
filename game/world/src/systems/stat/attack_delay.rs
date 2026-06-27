use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AttackDelay(pub f32);

#[derive(Clone, Copy)]
pub struct AttackDelayStat;

inventory::submit! {
    &AttackDelayStat as &dyn Stat
}

impl Scalar for AttackDelayStat {
    type Component = AttackDelay;
    const NAME: &'static str = "AttackDelay";
    const LABEL: &'static str = "Attack Delay";
    fn read(stat: &AttackDelay) -> f32 {
        stat.0
    }
    fn make(value: f32) -> AttackDelay {
        AttackDelay(value)
    }
}
