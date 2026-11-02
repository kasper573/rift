use std::collections::{BTreeMap, BTreeSet};
use ui::controls;
use ui::egui;
use ui::face;
use ui::theme;
use ui::words;

use ui::egui::{Context, Ui};
use ui::glyph::{self, Mark};

use crate::app::Editor;
use crate::modal::{FindUnusedDialog, ModalDialog};
use crate::rows;
use crate::unused::{self, UnusedItem, UnusedRow};
use ui::dialog::{self, answer, card, keys, title};
use ui::form::error_line;

pub(crate) fn find_unused(editor: &mut Editor) {
    editor
        .dialogs
        .push(ModalDialog::FindUnused(FindUnusedDialog {
            unticked: BTreeSet::new(),
            open: BTreeSet::new(),
            error: None,
        }));
}

/// The unused things of one table, or the unused files of one folder.
struct Group {
    id: String,
    label: String,
    members: Vec<Member>,
}

struct Member {
    item: UnusedItem,
    name: String,
    note: String,
}

pub(crate) fn find_unused_dialog(
    editor: &mut Editor,
    ctx: &Context,
    mut dialog: FindUnusedDialog,
) -> Option<ModalDialog> {
    let project = editor.project.as_ref()?;
    let loads = project.loads();
    let groups = groups(editor);
    let ticked: Vec<&UnusedItem> = groups
        .iter()
        .flat_map(|group| &group.members)
        .map(|member| &member.item)
        .filter(|item| !dialog.unticked.contains(*item))
        .collect();
    let rows_ticked = ticked
        .iter()
        .filter(|item| matches!(item, UnusedItem::Row(_)))
        .count();
    let files_ticked = ticked.len() - rows_ticked;
    let mut toggles: Vec<(Vec<UnusedItem>, bool)> = Vec::new();
    let mut opened: Option<UnusedItem> = None;
    let mut outcome = None;
    let mut pressed = dialog::Keys::default();
    card(ctx, "find-unused", 860.0, |ui| {
        pressed = keys(ui);
        title(ui, "Find unused content");
        if !loads {
            ui.label(theme::muted(
                "The game doesn't load, so what it uses can't be worked out. Fix its load errors \
                 first.",
            ));
        } else if groups.is_empty() {
            ui.label(theme::muted(
                "Nothing is unused: every row and media file is reached from what the game uses.",
            ));
        } else {
            ui.label(theme::muted(
                "Nothing the game uses reaches these: no fixture, setting, test character or row \
                 in use names them, directly or through other rows. Ticked ones are deleted \
                 together, with the rows local to them.",
            ));
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(440.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for group in &groups {
                        group_lines(ui, group, &mut dialog, &mut toggles, &mut opened);
                    }
                });
        }
        error_line(ui, dialog.error.as_deref());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                theme::faint(format!(
                    "{} row{} · {} file{} ticked · one undo step",
                    words::thousands(rows_ticked),
                    words::plural(rows_ticked),
                    words::thousands(files_ticked),
                    words::plural(files_ticked),
                ))
                .size(12.5),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!ticked.is_empty(), |ui| {
                    if controls::primary(ui, format!("Delete {}", words::thousands(ticked.len())))
                        .clicked()
                    {
                        outcome = Some(true);
                    }
                });
                if ui.button("Close").clicked() {
                    outcome = Some(false);
                }
            });
        });
    });
    for (items, on) in toggles {
        for item in items {
            if on {
                dialog.unticked.remove(&item);
            } else {
                dialog.unticked.insert(item);
            }
        }
        dialog.error = None;
    }
    if let Some(item) = opened {
        match item {
            UnusedItem::Row(row) => editor.open_path(&row),
            UnusedItem::Asset(path) => editor.open_media(&path),
        }
        return None;
    }
    match answer(outcome, &pressed) {
        Some(true) if !ticked.is_empty() => {
            let mut rows = Vec::new();
            let mut assets = Vec::new();
            for item in ticked {
                match item {
                    UnusedItem::Row(row) => rows.push(row.clone()),
                    UnusedItem::Asset(path) => assets.push(path.clone()),
                }
            }
            match delete(editor, &rows, &assets) {
                Ok(()) => None,
                Err(error) => {
                    dialog.error = Some(error);
                    Some(ModalDialog::FindUnused(dialog))
                }
            }
        }
        Some(false) => None,
        _ => Some(ModalDialog::FindUnused(dialog)),
    }
}

fn delete(editor: &mut Editor, rows: &[String], assets: &[String]) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let deletion = unused::delete(project, rows, assets)?;
    let label = deletion.edit.label.clone();
    editor.try_apply(deletion.edit)?;
    for row in &deletion.rows {
        rows::close_tabs(editor, row);
    }
    editor
        .selection
        .keys
        .retain(|key| !deletion.rows.contains(key));
    editor.toast(label.replacen("Delete", "Deleted", 1));
    Ok(())
}

/// The unused rows by table, in the content list's order, then the unused files by folder.
fn groups(editor: &Editor) -> Vec<Group> {
    let Some(project) = &editor.project else {
        return Vec::new();
    };
    let mut by_table: BTreeMap<&str, Vec<Member>> = BTreeMap::new();
    for (row, why) in project.unused.rows() {
        let note = match why {
            UnusedRow::Unnamed => "nothing names it".to_owned(),
            UnusedRow::NamedByUnused(by) => format!(
                "named only by unused {}",
                by.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        by_table
            .entry(engine::core::content::table_of(row).unwrap_or_default())
            .or_default()
            .push(Member {
                item: UnusedItem::Row(row.clone()),
                name: row.clone(),
                note,
            });
    }
    let mut groups: Vec<Group> = editor
        .shapes
        .order
        .iter()
        .filter_map(|table| {
            let members = by_table.remove(table.as_str())?;
            Some(Group {
                id: format!("table:{table}"),
                label: table.clone(),
                members,
            })
        })
        .collect();
    let mut by_folder: BTreeMap<&str, Vec<Member>> = BTreeMap::new();
    for path in project.unused.assets() {
        let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
        by_folder.entry(folder).or_default().push(Member {
            item: UnusedItem::Asset(path.to_owned()),
            name: name.to_owned(),
            note: String::new(),
        });
    }
    groups.extend(by_folder.into_iter().map(|(folder, members)| Group {
        id: format!("folder:{folder}"),
        label: format!("{folder}/"),
        members,
    }));
    groups
}

fn group_lines(
    ui: &mut Ui,
    group: &Group,
    dialog: &mut FindUnusedDialog,
    toggles: &mut Vec<(Vec<UnusedItem>, bool)>,
    opened: &mut Option<UnusedItem>,
) {
    let ticked = group
        .members
        .iter()
        .filter(|member| !dialog.unticked.contains(&member.item))
        .count();
    let open = dialog.open.contains(&group.id);
    ui.horizontal(|ui| {
        let mut on = ticked > 0;
        let mixed = on && ticked < group.members.len();
        if ui
            .add(egui::Checkbox::without_text(&mut on).indeterminate(mixed))
            .changed()
        {
            toggles.push((
                group
                    .members
                    .iter()
                    .map(|member| member.item.clone())
                    .collect(),
                on,
            ));
        }
        let fold = ui
            .horizontal(|ui| {
                let mark = if open {
                    Mark::CaretDown
                } else {
                    Mark::CaretRight
                };
                glyph::show(ui, mark, 11.0, theme::FAINT);
                ui.label(face::mono(&group.label).color(theme::TEXT));
            })
            .response
            .interact(egui::Sense::click());
        if fold.clicked() {
            if open {
                dialog.open.remove(&group.id);
            } else {
                dialog.open.insert(group.id.clone());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(face::mono(group.members.len().to_string()).color(theme::MUTED));
        });
    });
    if !open {
        return;
    }
    for member in &group.members {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            let mut on = !dialog.unticked.contains(&member.item);
            if ui.checkbox(&mut on, face::mono(&member.name)).changed() {
                toggles.push((vec![member.item.clone()], on));
            }
            if !member.note.is_empty() {
                ui.label(theme::muted(&member.note).size(12.5));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.link("Open").clicked() {
                    *opened = Some(member.item.clone());
                }
            });
        });
    }
}
