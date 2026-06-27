use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Damage(pub f32);

#[derive(Clone, Copy)]
pub struct DamageStat;

inventory::submit! {
    &DamageStat as &dyn Stat
}

impl Scalar for DamageStat {
    type Component = Damage;
    const NAME: &'static str = "Damage";
    const LABEL: &'static str = "Damage";
    fn read(stat: &Damage) -> f32 {
        stat.0
    }
    fn make(value: f32) -> Damage {
        Damage(value)
    }
}
