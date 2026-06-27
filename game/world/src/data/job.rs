use crate::systems::effect::Effect;
use crate::systems::job::{JobDef, JobLevel};
use crate::systems::stat::{Stat, StatKind};

crate::table! {
    Adventurer: JobDef {
        name: "Adventurer",
        levels: &[
            JobLevel {
                exp: 0,
                effects: &[],
            },
            JobLevel {
                exp: 30,
                effects: &[Effect::StatModifier(Stat { kind: StatKind::MaxHealth, value: 10.0 })],
            },
            JobLevel {
                exp: 90,
                effects: &[Effect::StatModifier(Stat { kind: StatKind::Damage, value: 2.0 })],
            },
            JobLevel {
                exp: 200,
                effects: &[
                    Effect::StatModifier(Stat { kind: StatKind::MaxHealth, value: 15.0 }),
                    Effect::StatModifier(Stat { kind: StatKind::Damage, value: 2.0 }),
                ],
            },
        ],
    },
}
