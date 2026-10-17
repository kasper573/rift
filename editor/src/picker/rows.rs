use std::sync::Arc;

use egui::Ui;
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::npc::{self, NpcDef};
use engine::systems::script::{OwnerKind, RiftScripts};
use serde_json::Value;

use super::{Candidate, Category, Dragged, Pick, PickSource};
use crate::forms::{FormAction, FormCx};
use crate::project::{ProjectFiles, row_file, row_of};
use crate::thumbs;
use crate::value::RowValue;

const PICTURE_TABLES: [&str; 6] = [
    "model",
    "item",
    "tileset",
    "attention",
    "interface_icon",
    "cursor",
];

/// Rows of one or more tables. Several tables become the popover's category chips; a single
/// script table offers its owner kinds instead.
pub struct RowSource {
    tables: Vec<String>,
    path: String,
}

impl RowSource {
    pub fn table(table: &str, path: &str) -> RowSource {
        RowSource::tables(&[table], path)
    }

    pub fn tables(tables: &[&str], path: &str) -> RowSource {
        RowSource {
            tables: tables.iter().map(|table| (*table).to_string()).collect(),
            path: path.to_owned(),
        }
    }

    fn several(&self) -> bool {
        self.tables.len() > 1
    }

    fn spoken(&self) -> String {
        self.tables
            .iter()
            .map(|table| table.replace('_', " "))
            .collect::<Vec<_>>()
            .join(" or ")
    }

    fn only_scripts(&self) -> bool {
        self.tables.as_slice() == ["script"]
    }
}

impl PickSource for RowSource {
    fn placeholder(&self) -> String {
        self.spoken()
    }

    fn categories(&self, cx: &FormCx) -> Vec<Category> {
        if self.several() {
            return self
                .tables
                .iter()
                .map(|table| Category {
                    id: table.clone(),
                    label: table.replace('_', " "),
                })
                .collect();
        }
        if self.only_scripts() {
            let mut owners: Vec<String> = cx
                .files
                .paths_under("tables/script/")
                .filter_map(|file| cx.files.json(file))
                .map(|row| row.text("owner").to_owned())
                .filter(|owner| !owner.is_empty())
                .collect();
            owners.sort();
            owners.dedup();
            return owners
                .into_iter()
                .map(|owner| Category {
                    label: owner.clone(),
                    id: owner,
                })
                .collect();
        }
        Vec::new()
    }

    fn candidates(&self, cx: &FormCx) -> Vec<Candidate> {
        let mut listed = Vec::new();
        for table in &self.tables {
            let block = pick_block(cx, table, &self.path);
            for file in cx.files.paths_under(&format!("tables/{table}/")) {
                let Some((_, key)) = row_of(file) else {
                    continue;
                };
                let category = if self.only_scripts() {
                    cx.files
                        .json(file)
                        .map(|row| row.text("owner").to_owned())
                        .unwrap_or_default()
                } else {
                    table.clone()
                };
                listed.push(Candidate {
                    pick: Pick::new(table.as_str(), key),
                    category,
                    name: display_name(cx, table, key),
                    hint: pick_hint(cx, table, key),
                    blocked: block.as_ref().and_then(|block| (block.reason)(cx, key)),
                });
            }
        }
        listed
    }

    fn accepts(&self, cx: &FormCx, dragged: &Dragged) -> Result<Pick, String> {
        let Dragged::Row { table, key } = dragged else {
            return Err(format!("this field takes {}, not a file", self.spoken()));
        };
        if !self.tables.iter().any(|listed| listed == table) {
            return Err(format!("this field takes {}, not {table}", self.spoken()));
        }
        if let Some(reason) =
            pick_block(cx, table, &self.path).and_then(|block| (block.reason)(cx, key))
        {
            return Err(format!("{table}/{key}: {reason}"));
        }
        Ok(Pick::new(table.as_str(), key.as_str()))
    }

    fn shown(&self, pick: &Pick) -> (String, String) {
        (format!("{}/", pick.category), pick.key.clone())
    }

    fn thumbnail(&self, ui: &mut Ui, cx: &mut FormCx, pick: &Pick, size: f32) {
        thumbnail(ui, cx, &pick.category, &pick.key, size);
    }

    fn footer(&self, cx: &FormCx, pick: &Pick) -> String {
        let uses = cx.usages.of_row(&pick.category, &pick.key).len();
        let mut footer = format!("{}/{}", pick.category, pick.key);
        if let Some(picture) = picture_of(cx.files, &pick.category, &pick.key) {
            footer.push_str(&format!(" · {}", picture.path));
        }
        footer.push_str(&format!(" · used by {uses}"));
        footer
    }

    fn grid(&self) -> bool {
        !self.several() && PICTURE_TABLES.contains(&self.tables[0].as_str())
    }

    fn opens(&self, pick: &Pick) -> Option<FormAction> {
        Some(FormAction::OpenRow {
            table: pick.category.clone(),
            key: pick.key.clone(),
        })
    }

    fn creates(&self, cx: &FormCx) -> Option<(String, FormAction)> {
        if self.several() || cx.shapes.has_fixtures(&self.tables[0]) {
            return None;
        }
        let table = &self.tables[0];
        Some((
            format!("New {}…", table.replace('_', " ")),
            FormAction::NewRow {
                table: table.clone(),
                path: self.path.clone(),
            },
        ))
    }
}

type BlockReason = Box<dyn Fn(&FormCx, &str) -> Option<String>>;

struct Block {
    reason: BlockReason,
}

fn pick_block(cx: &FormCx, table: &str, path: &str) -> Option<Block> {
    if table == "script" {
        let owner = cx.owner_table.clone()?;
        let kind = owner.parse::<OwnerKind>().unwrap_or(OwnerKind::Any);
        return Some(Block {
            reason: Box::new(move |cx, key| {
                let row = cx.files.json(&row_file("script", key))?;
                let script_owner = row.text("owner");
                (script_owner.parse::<OwnerKind>().unwrap_or(OwnerKind::Any) != kind)
                    .then(|| format!("owner {script_owner}"))
            }),
        });
    }
    if table == "skill" && path.ends_with("/basic_attack") {
        return Some(Block {
            reason: Box::new(|cx, key| {
                let row = cx.files.json(&row_file("skill", key))?;
                (row["basic_attack"] != true).then(|| "not flagged as a basic attack".to_owned())
            }),
        });
    }
    if table == "skill"
        && cx.owner_table.as_deref() == Some("class")
        && let Some(node) = path
            .strip_prefix("/tree/")
            .and_then(|rest| rest.strip_suffix("/skill"))
    {
        let class = cx.owner_key.clone()?;
        let node = node.to_owned();
        return Some(Block {
            reason: Box::new(move |cx, key| {
                let row = cx.files.json(&row_file("class", &class))?;
                let holder = row["tree"]
                    .as_object()?
                    .iter()
                    .find(|(other, held)| **other != node && held["skill"] == key)?;
                Some(format!("already the node {}", holder.0))
            }),
        });
    }
    if table == "npc" && cx.owner_table.as_deref() == Some("quest") && path.ends_with("/npc") {
        let content = cx.content.clone()?;
        if !content.has_built::<RiftScripts>() {
            return None;
        }
        return Some(Block {
            reason: Box::new(move |_, key| {
                let id = content.by_key::<NpcDef>(key)?;
                (!npc::can_be_fought(content.built::<RiftScripts>(), id))
                    .then(|| "no script sets it hostile".to_owned())
            }),
        });
    }
    None
}

fn pick_hint(cx: &FormCx, table: &str, key: &str) -> String {
    match table {
        "npc" => {
            let Some(content) = cx.content.clone() else {
                return String::new();
            };
            let Some(id) = content.by_key::<NpcDef>(key) else {
                return String::new();
            };
            let mut places: Vec<String> = Vec::new();
            for area in content.ids::<AreaDef>() {
                let count: u32 = area::placed(&content, area)
                    .iter()
                    .map(|placed| match &placed.module {
                        AreaModule::Resident(resident) if resident.npc == id => 1,
                        AreaModule::Population(population) if population.npc == id => {
                            population.count
                        }
                        _ => 0,
                    })
                    .sum();
                if count > 0 {
                    places.push(format!("{} × {count}", area.key(&content)));
                }
            }
            if places.is_empty() {
                "placed nowhere".to_owned()
            } else {
                places.join(" · ")
            }
        }
        "script" => {
            let owner = cx
                .files
                .json(&row_file("script", key))
                .and_then(|row| row["owner"].as_str().map(str::to_owned))
                .unwrap_or_default();
            let uses = cx.usages.of_row("script", key).len();
            format!("{owner} · used by {uses}")
        }
        _ => {
            let uses = cx.usages.of_row(table, key).len();
            format!("used by {uses}")
        }
    }
}

fn display_name(cx: &FormCx, table: &str, key: &str) -> String {
    let Some(row) = cx.files.json(&row_file(table, key)) else {
        return String::new();
    };
    ["name", "title", "label"]
        .iter()
        .find_map(|field| row[field].as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub struct Picture {
    pub path: String,
    pub frame: PictureFrame,
}

pub enum PictureFrame {
    Sprite(Arc<Value>),
    Whole,
    Tile(f32),
}

pub fn picture_of(files: &ProjectFiles, table: &str, key: &str) -> Option<Picture> {
    let row = files.json(&row_file(table, key))?;
    let (path, frame) = match table {
        "model" => (sprite_image(&row)?, PictureFrame::Sprite(row)),
        "npc" | "player" => {
            let model = files.json(&row_file("model", row["model"].as_str()?))?;
            (sprite_image(&model)?, PictureFrame::Sprite(model))
        }
        "item" | "skill" | "attention" | "interface_icon" | "cursor" => {
            let path = row["icon"].as_str().or(row["image"].as_str())?;
            (path.to_owned(), PictureFrame::Whole)
        }
        "tileset" => {
            let tile = row["tile"].as_f64().unwrap_or(16.0) as f32;
            (row["image"].as_str()?.to_owned(), PictureFrame::Tile(tile))
        }
        _ => return None,
    };
    Some(Picture { path, frame })
}

fn sprite_image(model: &Value) -> Option<String> {
    model["sprites"]["image"].as_str().map(str::to_owned)
}

fn thumbnail(ui: &mut Ui, cx: &mut FormCx, table: &str, key: &str, size: f32) {
    let Some(picture) = picture_of(cx.files, table, key) else {
        return;
    };
    let ctx = ui.ctx().clone();
    let Some(thumb) = cx.state.thumbnails.image(&ctx, cx.files, &picture.path) else {
        return;
    };
    match picture.frame {
        PictureFrame::Sprite(model) => {
            let frame = thumbs::model_frame(&model, &thumb);
            let scale = size / frame.size.y.max(1.0);
            ui.add(
                egui::Image::from_texture((thumb.texture.id(), frame.size * scale)).uv(frame.uv),
            );
        }
        PictureFrame::Whole => {
            ui.add(
                egui::Image::from_texture((thumb.texture.id(), egui::vec2(size, size)))
                    .fit_to_exact_size(egui::vec2(size, size)),
            );
        }
        PictureFrame::Tile(tile) => {
            let uv = thumbs::uv(thumb.size, 0.0, 0.0, tile * 2.0, tile * 2.0);
            ui.add(egui::Image::from_texture((thumb.texture.id(), egui::vec2(size, size))).uv(uv));
        }
    }
}
