use std::collections::BTreeMap;

use engine::field;
use engine::systems::class::SkillPoints;
use engine::systems::skill::{SkillDef, SkillLevel, SkillStat};
use serde_json::Value;
use ui::egui::Ui;

use crate::forms::{FormChange, FormCx};
use crate::levels::{self, Level};

/// The rank cards of a skill row: each rank's point cost and the stats it adds, with the totals a
/// player at that rank sees.
pub fn ranks(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let mut totals: BTreeMap<SkillStat, f32> = BTreeMap::new();
    levels::row_cards(
        ui,
        cx,
        value,
        field!(SkillDef, levels),
        skill_ranks,
        |_, rank| {
            for (stat, amount) in &rank.stats {
                *totals.entry(*stat).or_default() += amount;
            }
            let summed: Vec<String> = totals
                .iter()
                .map(|(stat, total)| format!("{} {total}", <&str>::from(*stat)))
                .collect();
            (!summed.is_empty()).then(|| format!("at this rank: {}", summed.join(" · ")))
        },
    )
}

fn skill_ranks(skill: &mut SkillDef) -> &mut Vec<SkillLevel> {
    &mut skill.levels
}

impl Level for SkillLevel {
    type Stat = SkillStat;
    const NAME: &'static str = "Rank";
    const STATS: &'static str = field!(SkillLevel, stats);

    fn stats(&mut self) -> &mut BTreeMap<SkillStat, f32> {
        &mut self.stats
    }

    fn unit(stat: SkillStat) -> Option<&'static str> {
        match stat {
            SkillStat::Cooldown | SkillStat::Duration => Some("s"),
            SkillStat::Range | SkillStat::Radius => Some("m"),
            SkillStat::Power | SkillStat::Mana => None,
        }
    }

    fn next(_: Option<&SkillLevel>) -> SkillLevel {
        SkillLevel {
            cost: SkillPoints(1),
            stats: BTreeMap::new(),
        }
    }
}
