use crate::systems::effect::Effect;
use crate::systems::job::{JobDef, JobLevel};
use crate::systems::stat::Stat;

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
                effects: &[Effect::StatModifier(Stat::MaxHealth(10.0))],
            },
            JobLevel {
                exp: 90,
                effects: &[Effect::StatModifier(Stat::Damage(2.0))],
            },
            JobLevel {
                exp: 200,
                effects: &[
                    Effect::StatModifier(Stat::MaxHealth(15.0)),
                    Effect::StatModifier(Stat::Damage(2.0)),
                ],
            },
        ],
    },
}
