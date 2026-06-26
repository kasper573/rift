//! The plain scalar stats — just declarations, no special handling. (Health and max_health, which
//! come with utility fns, live in health.rs; a stat graduates to its own file once it grows those.)

use super::scalar_stat;

scalar_stat!(Damage, DamageStat, "Damage");
scalar_stat!(AttackSpeed, AttackSpeedStat, "Attack Speed");
scalar_stat!(AttackDelay, AttackDelayStat, "Attack Delay");
scalar_stat!(Range, RangeStat, "Range");
scalar_stat!(MovementSpeed, MovementSpeedStat, "Move Speed");
