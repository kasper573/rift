use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Range(pub f32);

#[derive(Clone, Copy)]
pub struct RangeStat;

inventory::submit! {
    &RangeStat as &dyn Stat
}

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
