use crate::core::time::Seconds;
use crate::data::job::Id as JobId;
use crate::data::model::Id as ModelId;
use crate::systems::combat::HealthRegen;
use crate::systems::player::{PlayerDef, PlayerModule};
use crate::systems::stat::StatKind;

crate::table! {
    Adventurer: PlayerDef {
        model: ModelId::Adventurer,
        job: JobId::Adventurer,
        stats: &[
            StatKind::Health.of(30.0),
            StatKind::MaxHealth.of(30.0),
            StatKind::Damage.of(6.0),
            StatKind::AttackSpeed.of(1.2),
            StatKind::AttackDelay.of(200.0),
            StatKind::Range.of(1.5),
            StatKind::MovementSpeed.of(4.0),
        ],
        modules: &[PlayerModule::Regen(HealthRegen { every: Seconds(10.0), health: 5.0 })],
    },
}
