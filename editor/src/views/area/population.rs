use engine::core::content::ContentFile;
use engine::core::math::{Pos, Rng};
use engine::core::units::Meters;
use engine::systems::area::{self, Population};
use engine::systems::npc::{NpcDef, NpcModule};
use engine::systems::stat::StatKind;
use ui::chip;
use ui::egui::Ui;
use ui::form;
use ui::sheet;
use ui::theme;

use super::placement::short_value;
use crate::forms::{FormAction, FormCx};
use crate::tabs::shown_path;

/// What the npc at `npc` brings to every module placing it: when it respawns, its scripts and its
/// health, and a way to open it.
pub(super) fn from_npc(ui: &mut Ui, cx: &mut FormCx, npc: &str) {
    let Ok(row) = cx.project.read::<NpcDef>(npc) else {
        return;
    };
    let reader = cx.project.reader();
    sheet::section(ui, &format!("From {}", shown_path(npc)));
    let respawn = row.modules.iter().find_map(|module| match module {
        NpcModule::Respawn(respawn) => Some(respawn.after),
        _ => None,
    });
    form::kv_text(
        ui,
        "respawn",
        respawn.map_or("never".to_owned(), |after| format!("{} s", after.0)),
    );
    let scripts: Vec<String> = row
        .modules
        .iter()
        .filter_map(|module| match module {
            NpcModule::Script(script) => Some(script),
            _ => None,
        })
        .map(|script| {
            let named = shown_path(reader.path_of(script.script).unwrap_or_default());
            let values: Vec<String> = script
                .values
                .iter()
                .map(|(name, value)| format!("{name} {}", short_value(value, cx.project)))
                .collect();
            if values.is_empty() {
                named.to_owned()
            } else {
                format!("{named} · {}", values.join(" · "))
            }
        })
        .collect();
    form::kv_text(
        ui,
        "script",
        if scripts.is_empty() {
            "none".to_owned()
        } else {
            scripts.join(", ")
        },
    );
    if let Some(health) = row.stats.0.get(&StatKind::Health) {
        form::kv_text(ui, "health", health.to_string());
    }
    ui.add_space(4.0);
    if chip::colored(
        ui,
        format!("{} →", shown_path(npc)),
        theme::INPUT,
        theme::CODE,
    )
    .clicked()
    {
        cx.actions.push(FormAction::OpenRow {
            table: NpcDef::TABLE.to_owned(),
            key: npc.to_owned(),
        });
    }
}

/// Where a preview of `population` stands its npcs in `area`, rolled from `seed`.
pub fn population_places(
    area: &area::Area,
    population: &Population,
    seed: u64,
) -> Vec<Pos<Meters>> {
    let count = population.count as usize;
    let mut rng = Rng::new(
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(population.count as u64),
    );
    (0..count)
        .filter_map(|_| match population.roams {
            Some(bounds) => area
                .wild_mesh
                .random_point_within(&mut rng, bounds, |_| true),
            None => area.wild_mesh.random_point(&mut rng, |_| true),
        })
        .collect()
}
