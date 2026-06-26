//! One file per stat. Scalar stats use the [`scalar_stat!`](super::scalar_stat) macro (a component +
//! marker + [`Stat`](super::Stat) impl); a computed stat would instead hand-write a marker whose
//! `base` derives from the entity.

mod attack_delay;
mod attack_speed;
mod damage;
mod health;
mod max_health;
mod movement_speed;
mod range;

pub use attack_delay::{AttackDelay, AttackDelayStat};
pub use attack_speed::{AttackSpeed, AttackSpeedStat};
pub use damage::{Damage, DamageStat};
pub use health::{Health, HealthStat};
pub use max_health::{MaxHealth, MaxHealthStat};
pub use movement_speed::{MovementSpeed, MovementSpeedStat};
pub use range::{Range, RangeStat};
