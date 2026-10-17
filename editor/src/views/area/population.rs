use egui::{Ui, Vec2};
use engine::core::content::Content;
use engine::core::math::{Direction, Pos, Rng};
use engine::core::tiling::Tiles;
use engine::core::time::{PlaybackRate, Seconds};
use engine::systems::actor::model::ModelId;
use engine::systems::actor::{Action, ActorModel};
use engine::systems::area;
use engine::systems::npc::NpcDef;
use engine::systems::player::PlayerDef;
use serde_json::Value;

use crate::app::Editor;
use crate::forms::{FormAction, FormCx};
use crate::project::row_file;
use crate::thumbs;
use crate::ui::{theme, widgets};

pub(super) fn from_npc(ui: &mut Ui, cx: &mut FormCx, module: &Value) {
    let Some(npc) = module["npc"].as_str().map(str::to_owned) else {
        return;
    };
    let Some(row) = cx.files.json(&row_file("npc", &npc)) else {
        return;
    };
    widgets::section(ui, &format!("From npc/{npc}"));
    let modules = row["modules"].as_array().cloned().unwrap_or_default();
    let respawn = modules
        .iter()
        .find(|m| m["type"] == "respawn")
        .and_then(|m| m["after"].as_f64());
    widgets::kv_text(
        ui,
        "respawn",
        respawn.map_or("never".to_owned(), |after| format!("{} s", after)),
    );
    let scripts: Vec<String> = modules
        .iter()
        .filter(|m| m["type"].as_str().is_some_and(|t| t.ends_with("_script")))
        .map(|m| match m["script"].as_str() {
            Some(script) => {
                let values: Vec<String> = m["values"]
                    .as_object()
                    .into_iter()
                    .flat_map(|v| v.iter())
                    .map(|(k, v)| {
                        format!(
                            "{k} {}",
                            v.as_f64()
                                .map(|number| number.to_string())
                                .unwrap_or_else(|| v
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| v.to_string()))
                        )
                    })
                    .collect();
                if values.is_empty() {
                    script.to_owned()
                } else {
                    format!("{script} · {}", values.join(" · "))
                }
            }
            None => "inline script".to_owned(),
        })
        .collect();
    widgets::kv_text(
        ui,
        "script",
        if scripts.is_empty() {
            "none".to_owned()
        } else {
            scripts.join(", ")
        },
    );
    if let Some(health) = row["stats"]["health"].as_f64() {
        widgets::kv_text(ui, "health", health.to_string());
    }
    ui.add_space(4.0);
    if widgets::chip_colored(ui, &format!("npc/ {npc} →"), theme::INPUT, theme::CODE).clicked() {
        cx.actions.push(FormAction::OpenRow {
            table: "npc".to_owned(),
            key: npc.clone(),
        });
    }
}

pub fn population_places(area: &area::Area, module: &Value, seed: u64) -> Vec<Pos<Tiles>> {
    let count = module["count"].as_u64().unwrap_or(0) as usize;
    let nodes: Vec<Pos<Tiles>> = match super::shapes::rect_of(&module["roams"]) {
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
            .wrapping_add(module["count"].as_u64().unwrap_or(0)),
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
    let def = content.by_key::<NpcDef>(npc)?.get(content);
    model_sprite(editor, ctx, content, def.model)
}

pub fn player_sprite(
    editor: &mut Editor,
    ctx: &egui::Context,
    content: &Content,
) -> Option<NpcSprite> {
    let player = content.table::<PlayerDef>().keys().first().cloned()?;
    let def = content.by_key::<PlayerDef>(&player)?.get(content);
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
