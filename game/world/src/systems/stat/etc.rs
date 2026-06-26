//! The plain scalar stats — just declarations, no special handling. (Health and max_health, which
//! come with utility fns, live in health.rs; a stat graduates to its own file once it grows those.)

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::Scalar;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Damage(pub f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DamageStat;

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

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AttackSpeed(pub f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AttackSpeedStat;

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

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AttackDelay(pub f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AttackDelayStat;

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

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Range(pub f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RangeStat;

impl Scalar for RangeStat {
    type Component = Range;
    const NAME: &'static str = "Range";
    const LABEL: &'static str = "Range";
    fn read(stat: &Range) -> f32 {
        stat.0
    }
    fn make(value: f32) -> Range {
        Range(value)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MovementSpeed(pub f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MovementSpeedStat;

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
