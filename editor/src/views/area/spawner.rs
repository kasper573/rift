use engine::core::content::{ContentFile, FolderReader};
use engine::core::math::{Pos, Rng};
use engine::core::units::Meters;
use engine::field;
use engine::systems::area::{self, AreaModule, AreaModuleKind};
use engine::systems::item::ToolTypeDef;
use engine::systems::npc::{NpcDef, NpcModule};
use engine::systems::resource::{Extraction, ResourceDef};
use engine::systems::spawner::{SpawnShare, Spawner, Spawns};
use engine::systems::stat::StatKind;
use ui::chip;
use ui::egui::{self, Align, Layout, Ui};
use ui::form;
use ui::glyph::{self, Mark};
use ui::sheet;
use ui::theme;

use super::placement::short_value;
use super::{AreaFrame, nodes, pick};
use crate::app::Editor;
use crate::forms::{FormAction, FormCx};
use crate::picker;
use crate::tabs::{file_title, shown_path};

/// What `what` brings to the spawner spawning it, read from its file, and a way to open it.
pub(super) fn from_spawned(ui: &mut Ui, cx: &mut FormCx, what: Spawns) {
    let reader = cx.project.reader();
    match what {
        Spawns::Npc { npc } => {
            if let Some(npc) = reader.path_of(npc) {
                from_npc(ui, cx, npc);
            }
        }
        Spawns::Resource { resource } => {
            if let Some(resource) = reader.path_of(resource) {
                from_resource(ui, cx, resource);
            }
        }
    }
}

/// What the resource at `resource` brings to every spawner spawning it: how much it holds, how
/// many work it at once and what working it takes.
fn from_resource(ui: &mut Ui, cx: &mut FormCx, resource: &str) {
    let Ok(row) = cx.project.read::<ResourceDef>(resource) else {
        return;
    };
    let reader = cx.project.reader();
    sheet::section(ui, &format!("From {}", shown_path(resource)));
    form::kv_text(ui, "holds", row.quantity.to_string());
    form::kv_text(ui, "room for", row.max_occupants.to_string());
    let worked = match row.extraction {
        Extraction::ByHand { takes } => format!("by hand · {} s", takes.0),
        Extraction::Tool { tool, takes } => {
            let kind = reader
                .path_of(tool)
                .and_then(|tool| cx.project.read::<ToolTypeDef>(tool).ok())
                .map_or_else(|| "a tool".to_owned(), |tool| tool.name.clone());
            format!("{kind} · {} s", takes.0)
        }
    };
    form::kv_text(ui, "worked", worked);
    ui.add_space(4.0);
    if chip::colored(
        ui,
        format!("{} →", shown_path(resource)),
        theme::INPUT,
        theme::CODE,
    )
    .clicked()
    {
        cx.actions.push(FormAction::OpenRow {
            table: ResourceDef::TABLE.to_owned(),
            key: resource.to_owned(),
        });
    }
}

/// What the npc at `npc` brings to every module placing it: its scripts and its health, and a way
/// to open it.
pub(super) fn from_npc(ui: &mut Ui, cx: &mut FormCx, npc: &str) {
    let Ok(row) = cx.project.read::<NpcDef>(npc) else {
        return;
    };
    let reader = cx.project.reader();
    sheet::section(ui, &format!("From {}", shown_path(npc)));
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

/// The tables of the files a spawner can spawn.
pub(crate) const SPAWNED_TABLES: [&str; 2] = [NpcDef::TABLE, ResourceDef::TABLE];

/// Whether a spawner can spawn the files of `table`.
pub(super) fn spawns_table(table: &str) -> bool {
    SPAWNED_TABLES.contains(&table)
}

/// What a spawner spawns of the file at `key`, when it can spawn it.
pub(crate) fn spawned(reader: &FolderReader, key: &str) -> Option<Spawns> {
    reader
        .id_of::<NpcDef>(key)
        .map(|npc| Spawns::Npc { npc })
        .or_else(|| {
            reader
                .id_of::<ResourceDef>(key)
                .map(|resource| Spawns::Resource { resource })
        })
}

fn table_of(what: Spawns) -> &'static str {
    match what {
        Spawns::Npc { .. } => NpcDef::TABLE,
        Spawns::Resource { .. } => ResourceDef::TABLE,
    }
}

/// A file a spawner can spawn, dragged from the content tree and let go of over the map at
/// `ground`: one more line of the spawner whose rect it lands in, or a spawner of its own there.
pub(super) fn drop_file(editor: &mut Editor, frame: &AreaFrame, key: &str, ground: Pos<Meters>) {
    let Some(reader) = editor.reader() else {
        return;
    };
    let Some(what) = spawned(&reader, key) else {
        return;
    };
    let onto = frame
        .doc
        .of_kind(AreaModuleKind::Spawner)
        .find(|node| {
            matches!(
                node.module(),
                AreaModule::Spawner(spawner)
                    if spawner.within.is_some_and(|within| within.contains(ground))
                        && spawner.spawns.iter().all(|share| share.what != what)
            )
        })
        .map(|node| node.pointer.to_owned());
    match onto {
        Some(pointer) => {
            nodes::change_scene(editor, &frame.bundle, |scene| {
                let AreaModule::Spawner(spawner) = &mut scene.module_mut(&pointer)?.module else {
                    return None;
                };
                spawner.spawns.push(SpawnShare { what, weight: 1 });
                Some(format!(
                    "Add {} to spawner · {}",
                    shown_path(key),
                    file_title(&frame.bundle)
                ))
            });
        }
        None => pick::add_picked(
            editor,
            &frame.bundle,
            AreaModuleKind::Spawner,
            Some(crate::views::snap(ground, false)),
            key,
        ),
    }
}

/// The lines of what `spawner` may spawn, each with its share of the entries, and a line to add
/// another.
pub(super) fn spawns_card(ui: &mut Ui, cx: &mut FormCx, spawner: &mut Spawner, path: &str) -> bool {
    let reader = cx.project.reader();
    let mut changed = false;
    let mut remove = None;
    let alone = spawner.spawns.len() == 1;
    sheet::section(ui, "Spawns");
    for (index, share) in spawner.spawns.iter_mut().enumerate() {
        let share_path = format!("{path}{}/{index}", field!(Spawner, spawns));
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !alone
                    && glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                        .on_hover_text("Stop spawning this")
                        .clicked()
                {
                    remove = Some(index);
                }
                changed |= ui
                    .add(egui::DragValue::new(&mut share.weight).range(1..=u32::MAX))
                    .on_hover_text("Its share: how many entries in this many of all the shares")
                    .changed();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let mut table = table_of(share.what).to_owned();
                    let mut key = match share.what {
                        Spawns::Npc { npc } => reader.path_of(npc),
                        Spawns::Resource { resource } => reader.path_of(resource),
                    }
                    .unwrap_or_default()
                    .to_owned();
                    if picker::row_among(ui, cx, &SPAWNED_TABLES, &mut table, &mut key, &share_path)
                        && let Some(picked) = spawned(&reader, &key)
                    {
                        share.what = picked;
                        changed = true;
                    }
                });
            });
        });
    }
    if let Some(index) = remove {
        spawner.spawns.remove(index);
        changed = true;
    }
    form::kv(ui, "+ spawn", |ui| {
        let new_path = format!("{path}{}/{}", field!(Spawner, spawns), spawner.spawns.len());
        if let Some((_, key)) = picker::row_added_among(ui, cx, &SPAWNED_TABLES, &new_path)
            && let Some(what) = spawned(&reader, &key)
        {
            spawner.spawns.push(SpawnShare { what, weight: 1 });
            changed = true;
        }
    });
    changed
}

/// What a preview of `spawner` stands in `area` and where, rolled from `seed`.
pub(crate) fn preview_places(
    area: &area::Area,
    spawner: &Spawner,
    seed: u64,
) -> Vec<(Spawns, Pos<Meters>)> {
    let mut rng = Rng::new(
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(spawner.count as u64),
    );
    (0..spawner.count)
        .filter_map(|_| {
            let what = spawner.draw(&mut rng);
            let mesh = match what {
                Spawns::Npc { .. } => area.mesh_for(true, false),
                Spawns::Resource { .. } => area.mesh_for(false, false),
            };
            let at = match spawner.within {
                Some(bounds) => mesh.random_point_within(&mut rng, bounds, |_| true),
                None => mesh.random_point(&mut rng, |_| true),
            }?;
            Some((what, at))
        })
        .collect()
}
