//! The body page: what the skeleton's bodies are put together from. The builds its parts are cut
//! for, the regions parts cover and hide, the slots worn items lay their parts in, in the order
//! they lay, and the tints a body is painted in, in the order they paint. The stage dresses its
//! wearer in the items picked slot by slot.

use std::collections::BTreeMap;

use engine::core::content::{Content, NamedContent};
use engine::systems::actor::ModelPartDef;
use engine::systems::actor::skeleton::{BuiltSkeleton, EquipmentSlot, SkeletonDef, SkeletonId};
use engine::systems::item::{ItemDef, Wear};
use ui::egui::{self, Ui, Vec2};
use ui::{chip, form, panel, theme, words};

use crate::app::Editor;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, built: &BuiltSkeleton) {
    let Some(id) = content.by_path::<SkeletonDef>(key) else {
        return;
    };
    let parts: Vec<&ModelPartDef> = content
        .table::<ModelPartDef>()
        .iter()
        .filter(|(_, part)| part.skeleton == id)
        .map(|(_, part)| part)
        .collect();
    let width = ui.available_width();
    let list_width = (width * 0.36).clamp(240.0, 380.0);
    let [left, right] = panel::split(ui, list_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("body", key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                builds(ui, &built.def, &parts);
                ui.add_space(10.0);
                regions(ui, &built.def, &parts);
                ui.add_space(10.0);
                slots(editor, ui, key, content, id, &built.def);
                ui.add_space(10.0);
                tints(ui, &built.def);
            });
    });
    panel::body(ui, right, |ui| {
        super::played_clip(editor, key, built);
        super::stage_pickers(editor, ui, key, content, built, false);
        let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
        super::stage_rig(editor, ui, key, content, built, size, Vec::new());
        form::hint(
            ui,
            "An item picked under a slot dresses the stage's body in it.",
        );
    });
}

fn builds(ui: &mut Ui, def: &SkeletonDef, parts: &[&ModelPartDef]) {
    form::section_count(ui, "Builds", def.builds.len());
    if def.builds.is_empty() {
        form::hint(ui, "One build: every part is a single mesh.");
    }
    for build in &def.builds {
        let cut = parts
            .iter()
            .filter(|part| part.builds.contains_key(build))
            .count();
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::mono(build.as_str()).size(13.0));
            ui.label(theme::faint(format!(
                "{} cut for it",
                words::counted(cut, "part")
            )));
        });
    }
}

fn regions(ui: &mut Ui, def: &SkeletonDef, parts: &[&ModelPartDef]) {
    form::section_count(ui, "Regions", def.regions.len());
    if def.regions.is_empty() {
        form::hint(
            ui,
            "No regions: each part a body lays is drawn, none replacing another.",
        );
    }
    for region in &def.regions {
        let covering = parts
            .iter()
            .filter(|part| part.covers.contains(region))
            .count();
        let hiding = parts
            .iter()
            .filter(|part| part.hides.contains(region))
            .count();
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::mono(region.as_str()).size(13.0));
            let mut said = format!("covered by {}", words::counted(covering, "part"));
            if hiding > 0 {
                said.push_str(&format!(" · hidden by {hiding}"));
            }
            ui.label(theme::faint(said));
        });
    }
}

/// Each slot with the items worn in it that have a look on the skeleton, each picked to dress the
/// stage in it and picked again to take it off.
fn slots(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    id: SkeletonId,
    def: &SkeletonDef,
) {
    form::section_count(ui, "Slots · in the order they lay", def.slots.len());
    if def.slots.is_empty() {
        form::hint(ui, "No slots: nothing is worn on the body.");
    }
    let worn: Vec<(&EquipmentSlot, String, &str)> = content
        .table::<ItemDef>()
        .iter()
        .filter_map(|(item, row)| match row.wear() {
            Some(Wear::Worn { slot, looks }) if looks.iter().any(|look| look.skeleton == id) => {
                Some((slot, item.path(content).to_owned(), row.shown_name()))
            }
            _ => None,
        })
        .collect();
    let mut wearing = editor.views.skeleton.view(key).wearing.clone();
    for slot in &def.slots {
        ui.label(theme::mono(slot.as_str()).size(13.0));
        let here: Vec<&(&EquipmentSlot, String, &str)> =
            worn.iter().filter(|(worn, _, _)| *worn == slot).collect();
        if here.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(theme::faint("nothing with a look on this skeleton"));
            });
        }
        for (_, path, name) in here {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let on = wearing.get(slot) == Some(path);
                if ui.add(egui::Button::selectable(on, *name)).clicked() {
                    if on {
                        wearing.remove(slot);
                    } else {
                        wearing.insert(slot.clone(), path.clone());
                    }
                }
            });
        }
    }
    editor.views.skeleton.view_mut(key).wearing = wearing;
}

fn tints(ui: &mut Ui, def: &SkeletonDef) {
    form::section_count(ui, "Tints · in the order they paint", def.tints.len());
    if def.tints.is_empty() {
        form::hint(ui, "No tints: every part is drawn in its own texture.");
    }
    for tint in &def.tints {
        let [r, g, b] = tint.colour.0;
        ui.horizontal(|ui| {
            chip::swatch(
                ui,
                theme::mono(tint.name.as_str()).size(13.0),
                tint.colour.into(),
            );
            ui.label(theme::faint(format!("#{r:02x}{g:02x}{b:02x}")));
        });
    }
}

/// The items the stage's body is dressed in, by the slot each is worn in.
pub(super) fn wearing(
    editor: &Editor,
    key: &str,
    content: &Content,
) -> BTreeMap<EquipmentSlot, engine::systems::item::ItemId> {
    editor
        .views
        .skeleton
        .view(key)
        .wearing
        .iter()
        .filter_map(|(slot, path)| Some((slot.clone(), content.by_path::<ItemDef>(path)?)))
        .collect()
}
