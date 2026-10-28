//! The body page: what the skeleton's bodies are put together from, each list made row by row. The
//! builds its models and parts are cut for, the first the one every other falls back to; the
//! regions parts cover and hide; the slots worn items lay their parts in, in the order they lay,
//! each with the items worn there to dress the other pages' stage in; and the tints a body is
//! painted in, in the order they paint.

use std::collections::BTreeMap;

use engine::core::content::{Content, NamedContent};
use engine::core::math::Rgb;
use engine::systems::actor::ModelPartDef;
use engine::systems::actor::skeleton::{
    BodyRegion, BuildName, EquipmentSlot, SkeletonDef, SkeletonTint, TintName,
};
use engine::systems::item::{ItemDef, ItemId, Wear};
use ui::egui::{self, Ui};
use ui::{controls, form, theme, words};

use crate::app::Editor;

/// A list of names the page edits: its rows in order, each renamed, moved and taken away in place.
struct Names<'a> {
    title: &'a str,
    hint: &'a str,
    names: Vec<String>,
    notes: Vec<String>,
}

/// What became of a list this frame.
enum Edited {
    Renamed(usize, String),
    Moved(usize, usize),
    Removed(usize),
    Added(String),
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef) {
    let content = editor.loaded();
    let parts: Vec<&ModelPartDef> = content
        .as_ref()
        .and_then(|content| content.by_path::<SkeletonDef>(key).map(|id| (content, id)))
        .map(|(content, id)| {
            content
                .table::<ModelPartDef>()
                .iter()
                .filter(|(_, part)| part.skeleton == id)
                .map(|(_, part)| part)
                .collect()
        })
        .unwrap_or_default();
    egui::ScrollArea::vertical()
        .id_salt(("body", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.columns(2, |columns| {
                let builds = Names {
                    title: "Builds",
                    hint: "The body types its models and parts are cut for; the first is the one \
                           every other plays the clips of where its own cells name none.",
                    names: def.builds.iter().map(|build| build.0.clone()).collect(),
                    notes: def
                        .builds
                        .iter()
                        .map(|build| {
                            let cut = parts
                                .iter()
                                .filter(|part| part.builds.contains_key(build))
                                .count();
                            format!("{} cut for it", words::counted(cut, "part"))
                        })
                        .collect(),
                };
                if let Some(edit) = list(&mut columns[0], key, builds) {
                    apply(
                        editor,
                        key,
                        "builds",
                        edit,
                        |skeleton| &mut skeleton.builds,
                        BuildName::new,
                    );
                }
                columns[0].add_space(10.0);
                let regions = Names {
                    title: "Regions",
                    hint: "The places of the body parts cover and hide: a part laid over another \
                           that covers one of its regions replaces it.",
                    names: def.regions.iter().map(|region| region.0.clone()).collect(),
                    notes: def
                        .regions
                        .iter()
                        .map(|region| {
                            let covering = parts
                                .iter()
                                .filter(|part| part.covers.contains(region))
                                .count();
                            format!("covered by {}", words::counted(covering, "part"))
                        })
                        .collect(),
                };
                if let Some(edit) = list(&mut columns[0], key, regions) {
                    apply(
                        editor,
                        key,
                        "regions",
                        edit,
                        |skeleton| &mut skeleton.regions,
                        BodyRegion::new,
                    );
                }
                let slots = Names {
                    title: "Slots · in the order they lay",
                    hint: "The places items are worn besides the hands, in the order their parts \
                           are laid over the body.",
                    names: def.slots.iter().map(|slot| slot.0.clone()).collect(),
                    notes: vec![String::new(); def.slots.len()],
                };
                if let Some(edit) = list(&mut columns[1], key, slots) {
                    apply(
                        editor,
                        key,
                        "slots",
                        edit,
                        |skeleton| &mut skeleton.slots,
                        EquipmentSlot::new,
                    );
                }
                if let Some(content) = &content {
                    wearing(editor, &mut columns[1], key, content, def);
                }
                columns[1].add_space(10.0);
                tints(editor, &mut columns[1], key, def);
            });
        });
}

/// The rows of `names`, each to rename, move and take away, and a row to add one more.
fn list(ui: &mut Ui, key: &str, names: Names) -> Option<Edited> {
    form::section_count(ui, names.title, names.names.len());
    let mut edited = None;
    for (index, name) in names.names.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut renamed = name.clone();
            let response = ui.add(egui::TextEdit::singleline(&mut renamed).desired_width(140.0));
            if response.lost_focus() && renamed.trim() != name && !renamed.trim().is_empty() {
                edited = Some(Edited::Renamed(index, renamed.trim().to_owned()));
            }
            if index > 0 && ui.small_button("↑").on_hover_text("earlier").clicked() {
                edited = Some(Edited::Moved(index, index - 1));
            }
            if index + 1 < names.names.len()
                && ui.small_button("↓").on_hover_text("later").clicked()
            {
                edited = Some(Edited::Moved(index, index + 1));
            }
            if controls::remove(ui, names.title).clicked() {
                edited = Some(Edited::Removed(index));
            }
            if let Some(note) = names.notes.get(index).filter(|note| !note.is_empty()) {
                ui.label(theme::faint(note));
            }
        });
    }
    let id = ui.make_persistent_id(("body-new", key, names.title));
    let mut fresh: String = ui.data_mut(|data| data.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut fresh)
                .hint_text("name")
                .desired_width(140.0),
        );
        let name = fresh.trim().to_owned();
        if ui
            .add_enabled(
                !name.is_empty() && !names.names.contains(&name),
                egui::Button::new("+ add"),
            )
            .clicked()
        {
            edited = Some(Edited::Added(name));
            fresh.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(id, fresh));
    form::hint(ui, names.hint);
    edited
}

/// Writes `edit` into the list `field` reads of the skeleton, each name made as `name` makes it.
fn apply<N: Clone>(
    editor: &mut Editor,
    key: &str,
    what: &str,
    edit: Edited,
    field: impl FnOnce(&mut SkeletonDef) -> &mut Vec<N>,
    name: impl Fn(String) -> N,
) {
    super::change(editor, key, &format!("Change the {what}"), |skeleton| {
        let list = field(skeleton);
        match edit {
            Edited::Renamed(index, to) => list[index] = name(to),
            Edited::Moved(from, to) => list.swap(from, to),
            Edited::Removed(index) => {
                list.remove(index);
            }
            Edited::Added(new) => list.push(name(new)),
        }
        Ok(())
    });
}

/// Each slot with the items worn in it that have a look on the skeleton, each picked to dress the
/// stage in it and picked again to take it off.
fn wearing(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, def: &SkeletonDef) {
    let Some(id) = content.by_path::<SkeletonDef>(key) else {
        return;
    };
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
    if worn.is_empty() {
        return;
    }
    ui.label(theme::muted("dress the stage in"));
    let mut dressed = editor.views.skeleton.view(key).wearing.clone();
    for slot in &def.slots {
        for (_, path, name) in worn.iter().filter(|(worn, _, _)| *worn == slot) {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let on = dressed.get(slot) == Some(path);
                if ui
                    .add(egui::Button::selectable(on, format!("{slot} · {name}")))
                    .clicked()
                {
                    if on {
                        dressed.remove(slot);
                    } else {
                        dressed.insert(slot.clone(), path.clone());
                    }
                }
            });
        }
    }
    editor.views.skeleton.view_mut(key).wearing = dressed;
}

fn tints(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef) {
    form::section_count(ui, "Tints · in the order they paint", def.tints.len());
    let mut tints = def.tints.clone();
    let mut removed = None;
    let mut moved = None;
    for (index, tint) in tints.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut colour = egui::Color32::from(tint.colour);
            if ui.color_edit_button_srgba(&mut colour).changed() {
                tint.colour = Rgb([colour.r(), colour.g(), colour.b()]);
            }
            let mut name = tint.name.0.clone();
            let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
            if response.lost_focus() && !name.trim().is_empty() {
                tint.name = TintName::new(name.trim());
            }
            if index > 0 && ui.small_button("↑").on_hover_text("earlier").clicked() {
                moved = Some((index, index - 1));
            }
            if ui.small_button("↓").on_hover_text("later").clicked() {
                moved = Some((index, index + 1));
            }
            if controls::remove(ui, "tint").clicked() {
                removed = Some(index);
            }
        });
    }
    let id = ui.make_persistent_id(("body-new-tint", key));
    let mut fresh: String = ui.data_mut(|data| data.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut fresh)
                .hint_text("name")
                .desired_width(140.0),
        );
        let name = fresh.trim().to_owned();
        if ui
            .add_enabled(
                !name.is_empty() && !tints.iter().any(|tint| tint.name.0 == name),
                egui::Button::new("+ add"),
            )
            .clicked()
        {
            tints.push(SkeletonTint {
                name: TintName::new(name),
                colour: Rgb([200, 200, 200]),
            });
            fresh.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(id, fresh));
    form::hint(
        ui,
        "The colours a body's parts are painted in where nothing laid over it says otherwise, in the \
         order they paint where their masks overlap.",
    );
    if let Some(index) = removed {
        tints.remove(index);
    }
    if let Some((from, to)) = moved.filter(|(_, to)| *to < tints.len()) {
        tints.swap(from, to);
    }
    if tints != def.tints {
        super::change(editor, key, "Change the tints", |skeleton| {
            skeleton.tints = tints;
            Ok(())
        });
    }
}

/// The items the stage's body is dressed in, by the slot each is worn in.
pub(super) fn wearing_items(
    editor: &Editor,
    key: &str,
    content: &Content,
) -> BTreeMap<EquipmentSlot, ItemId> {
    editor
        .views
        .skeleton
        .view(key)
        .wearing
        .iter()
        .filter_map(|(slot, path)| Some((slot.clone(), content.by_path::<ItemDef>(path)?)))
        .collect()
}
