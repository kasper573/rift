use std::collections::BTreeMap;

use engine::core::geometry::Shape;
use engine::core::units::Meters;
use engine::field;
use engine::systems::class::SkillPoints;
use engine::systems::skill::{SkillDef, SkillLevel, SkillStat, Targeting};
use serde_json::Value;
use ui::egui::Ui;

use crate::forms::{FormChange, FormCx};
use crate::shell::levels::{self, Level};

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
                .map(|(stat, total)| format!("{} {total}", stat.label()))
                .collect();
            (!summed.is_empty()).then(|| format!("at this rank: {}", summed.join(" · ")))
        },
    )
}

/// `skill` as an edit from `before` leaves it, with its target and area kept in step: a target
/// aimed at a point takes an area and a single one gives its area up, and an area added or removed
/// turns the target into its area or single kind. The game takes neither half without the other.
pub fn paired_targeting(before: &SkillDef, skill: &mut SkillDef) {
    let target_changed = before.target != skill.target;
    match (skill.target.aimed_at_point(), skill.area.is_some()) {
        (true, false) if target_changed => {
            skill.area = Some(Shape::Circle {
                radius: Meters(2.0),
            })
        }
        (true, false) => skill.target = single_kind(skill.target),
        (false, true) if target_changed => skill.area = None,
        (false, true) => skill.target = area_kind(skill.target),
        _ => {}
    }
}

fn single_kind(target: Targeting) -> Targeting {
    match target {
        Targeting::AreaAlly => Targeting::SingleAlly,
        Targeting::AreaEnemy => Targeting::SingleEnemy,
        single => single,
    }
}

fn area_kind(target: Targeting) -> Targeting {
    match target {
        Targeting::SingleEnemy => Targeting::AreaEnemy,
        Targeting::SingleAlly | Targeting::Caster => Targeting::AreaAlly,
        area => area,
    }
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
        stat.unit()
    }

    fn next(_: Option<&SkillLevel>) -> SkillLevel {
        SkillLevel {
            cost: SkillPoints(1),
            stats: BTreeMap::new(),
        }
    }
}
