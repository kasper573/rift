use crate::systems::job::{JobDef, JobLevel};
use crate::systems::stat::{Stat, StatKind};

crate::table! {
    Adventurer: JobDef {
        name: "Adventurer",
        levels: &[
            JobLevel {
                xp: 0,
                stats: &[],
                chasing: false,
            },
            JobLevel {
                xp: 30,
                stats: &[Stat { kind: StatKind::MaxHealth, value: 10.0 }],
                chasing: false,
            },
            JobLevel {
                xp: 90,
                stats: &[Stat { kind: StatKind::Damage, value: 2.0 }],
                chasing: false,
            },
            JobLevel {
                xp: 200,
                stats: &[Stat { kind: StatKind::MaxHealth, value: 15.0 }, Stat { kind: StatKind::Damage, value: 2.0 }],
                chasing: false,
            },
        ],
    },
}
