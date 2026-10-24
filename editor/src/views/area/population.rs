use engine::core::content::{Content, ContentFile};
use engine::core::math::{Direction, Pos, Rng};
use engine::core::time::{PlaybackRate, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::ModelId;
use engine::systems::actor::{Action, ActorModel};
use engine::systems::area::{self, Population};
use engine::systems::npc::{NpcDef, NpcModule};
use engine::systems::stat::StatKind;
use ui::chip;
use ui::egui;
use ui::egui::{Ui, Vec2};
use ui::form;
use ui::sheet;
use ui::theme;

use super::placement::short_value;
use crate::app::Editor;
use crate::forms::{FormAction, FormCx};
use crate::tabs::shown_path;
use crate::thumbs;

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
    let nodes: Vec<Pos<Meters>> = match population.roams {
        Some(bounds) => area
            .wild_grid
            .nodes()
            .iter()
            .copied()
            .filter(|p| bounds.contains(*p))
            .collect(),
        None => area.wild_grid.nodes().to_vec(),
    };
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut rng = Rng::new(
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(population.count as u64),
    );
    (0..count)
        .map(|_| nodes[rng.rand_range(0..nodes.len() as u32) as usize])
        .collect()
}

pub struct NpcSprite {
    pub texture: egui::TextureId,
    pub uv: egui::Rect,
    pub drawn: Vec2,
}

pub fn npc_sprite(
    editor: &mut Editor,
    ctx: &egui::Context,
    content: &Content,
    npc: &str,
) -> Option<NpcSprite> {
    let def = content.by_path::<NpcDef>(npc)?.get(content);
    model_sprite(editor, ctx, content, def.model)
}

pub fn model_sprite(
    editor: &mut Editor,
    ctx: &egui::Context,
    content: &Content,
    model: ModelId,
) -> Option<NpcSprite> {
    let model = ActorModel::get(content, model);
    let project = editor.project.as_ref()?;
    let thumb = editor
        .form
        .thumbnails
        .image(ctx, &project.files, model.sheet().as_str())?;
    let region = model.frame(Action::Idle, Direction::S, Seconds(0.0), PlaybackRate(1.0));
    let drawn = model.drawn_size(region);
    let uv = thumbs::uv(
        thumb.size,
        region.min().x,
        region.min().y,
        region.size.width,
        region.size.height,
    );
    Some(NpcSprite {
        texture: thumb.texture.id(),
        uv,
        drawn: Vec2::new(drawn.width, drawn.height),
    })
}
