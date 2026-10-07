use crate::core::babble::BabbleId;
use crate::core::time::Seconds;
use crate::data::job::Id as JobId;
use crate::data::model::Id as ModelId;
use crate::systems::actor::Rgba;
use crate::systems::combat::HealthRegen;
use crate::systems::player::PlayerDef;
use crate::systems::stat::StatKind;

crate::table! {
    Adventurer: PlayerDef {
        model: ModelId::Adventurer,
        babble: BabbleId::Adventurer,
        job: JobId::Adventurer,
        tint: Rgba(0xffffffff),
        stats: &[
            StatKind::Health.of(30.0),
            StatKind::MaxHealth.of(30.0),
            StatKind::Damage.of(6.0),
            StatKind::AttackSpeed.of(1.2),
            StatKind::AttackDelay.of(200.0),
            StatKind::Range.of(1.5),
            StatKind::MovementSpeed.of(4.0),
        ],
        regen: HealthRegen { every: Seconds(10.0), health: 5.0 },
    },
}

/// The archetype every player plays as.
pub const DEFAULT_ID: Id = Id::Adventurer;
