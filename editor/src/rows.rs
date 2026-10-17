use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use egui::{RichText, Ui};
use engine::core::content::ModuleRule;
use engine::core::content::id::LeafKind;
use engine::core::vfx::shader_file;
use serde_json::Value;

use crate::app::{DeleteDialog, Editor, ModalDialog, NewRowDialog, registry};
use crate::project::{ContentEdit, Project, ProjectFiles, RowSelection, row_file, row_of};
use crate::schema::SchemaShapes;
use crate::tabs::EditorTab;
use crate::ui::{theme, widgets};
use crate::usages::ContentUsage;
use crate::views::area::placement;
use crate::views::{vfx, world};
use crate::{schema, usages};

pub const KEY_MAX: usize = 64;

#[derive(Clone, Debug)]
pub struct UsageGroup {
    pub table: String,
    pub kind: String,
    pub usages: Vec<ContentUsage>,
    pub removable: bool,
    pub whole_modules: bool,
    pub decision: UsageDecision,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UsageDecision {
    Undecided,
    Remove,
    Repoint(String),
    Unpack,
}

pub fn key_problem(files: &ProjectFiles, table: &str, key: &str) -> Option<String> {
    if key.is_empty() || key.len() > KEY_MAX {
        return Some(format!("a key is 1 to {KEY_MAX} characters"));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Some("a key uses letters, digits, _ and - only".to_owned());
    }
    let lower = key.to_lowercase();
    let clash = files
        .paths_under(&format!("tables/{table}/"))
        .filter_map(|file| row_of(file).map(|(_, other)| other.to_owned()))
        .find(|other| other.to_lowercase() == lower);
    clash.map(|other| format!("{table}/{other} already exists (keys ignore case)"))
}

pub fn key_from_name(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "_")
}

pub fn free_key(files: &ProjectFiles, table: &str, wanted: &str) -> String {
    let taken: HashSet<String> = files.keys_of(table).map(str::to_owned).collect();
    free_name(wanted, &taken)
}

pub fn free_name(name: &str, taken: &HashSet<String>) -> String {
    let is_taken = |candidate: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    if !is_taken(name) {
        return name.to_owned();
    }
    let base = match name.rfind('_') {
        Some(cut) if cut > 0 && name[cut + 1..].chars().all(|c| c.is_ascii_digit()) => &name[..cut],
        _ => name,
    };
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| !is_taken(candidate))
        .expect("a free name")
}

pub fn new_row_dialog(editor: &mut Editor, table: &str) {
    new_row_into(editor, table, None);
}

pub fn new_row_into(editor: &mut Editor, table: &str, into: Option<(String, String)>) {
    let Some(project) = &editor.project else {
        return;
    };
    let Some(shape) = editor.shapes.table(table) else {
        return;
    };
    let value = schema::template(&shape.shape, &shape.shape);
    let key = free_key(&project.files, table, &format!("new_{table}"));
    editor.dialogs.push(ModalDialog::NewRow(NewRowDialog {
        table: table.to_owned(),
        key,
        value,
        error: None,
        into,
    }));
}

pub fn new_row_body(editor: &mut Editor, ui: &mut Ui, dialog: &mut NewRowDialog) {
    ui.horizontal(|ui| {
        ui.label("Key");
        ui.text_edit_singleline(&mut dialog.key);
    });
    if let Some(project) = &editor.project
        && let Some(problem) = key_problem(&project.files, &dialog.table, dialog.key.trim())
    {
        ui.label(RichText::new(problem).weak());
    }
    if let Some(name) = dialog
        .value
        .get_mut("name")
        .and_then(|name| name.as_str().map(str::to_owned))
    {
        let mut draft = name;
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui.text_edit_singleline(&mut draft).changed() {
                dialog.value["name"] = Value::String(draft);
            }
        });
    }
    if let Some((file, pointer)) = &dialog.into {
        ui.label(
            RichText::new(format!("Written into {file} at {pointer}"))
                .weak()
                .small(),
        );
    }
    if let Some(error) = &dialog.error {
        ui.colored_label(theme::DANGER, error);
    }
}

pub fn create(editor: &mut Editor, dialog: &NewRowDialog) -> Result<(), String> {
    let key = dialog.key.trim().to_owned();
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    if let Some(problem) = key_problem(&project.files, &dialog.table, &key) {
        return Err(problem);
    }
    let mut edit = ContentEdit::new(format!("New {} {key}", dialog.table));
    edit.write_json(row_file(&dialog.table, &key), &dialog.value);
    if let Some(file) = companion_file(&dialog.table, &key) {
        edit.write(file, Arc::from(NEW_SHADER.as_bytes()));
    }
    if let Some((file, pointer)) = &dialog.into
        && let Some(mut target) = project.files.json_owned(file)
    {
        if let Some(slot) = target.pointer_mut(pointer) {
            *slot = Value::String(key.clone());
        }
        edit.write_json(file, &target);
    }
    editor.try_apply(edit)?;
    editor.select(RowSelection::row(&dialog.table, &key));
    editor.open_tab(EditorTab::for_row(&dialog.table, &key));
    Ok(())
}

pub fn duplicate(editor: &mut Editor, table: &str, key: &str) {
    let Some(project) = &editor.project else {
        return;
    };
    let Some(value) = project.files.json(&row_file(table, key)) else {
        return;
    };
    let copy = free_key(&project.files, table, &format!("{key}_2"));
    let mut edit = ContentEdit::new(format!("Duplicate {table}/{key} as {copy}"));
    edit.write_json(row_file(table, &copy), &value);
    if let Some((from, to)) = companion_file(table, key).zip(companion_file(table, &copy))
        && let Some(bytes) = project.files.get(&from)
    {
        edit.write(to, bytes.clone());
    }
    for file in folder_files(project, table, key) {
        if let Some(bytes) = project.files.get(&file) {
            edit.write(
                file.replacen(
                    &format!("tables/{table}/{key}/"),
                    &format!("tables/{table}/{copy}/"),
                    1,
                ),
                bytes.clone(),
            );
        }
    }
    if table == "area" {
        world::card_beside(project, &mut edit, key, &copy);
    }
    if editor.apply(edit) {
        editor.select(RowSelection::row(table, &copy));
        editor.open_tab(EditorTab::for_row(table, &copy));
    }
}

pub fn rename(editor: &mut Editor, table: &str, key: &str, new_key: &str) -> Result<(), String> {
    if new_key == key {
        return Ok(());
    }
    if editor.shapes.is_fixture(table, key) {
        return Err("fixtures keep their key".to_owned());
    }
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    if let Some(problem) = key_problem(&project.files, table, new_key)
        && !(new_key.to_lowercase() == key.to_lowercase() && new_key.len() <= KEY_MAX)
    {
        return Err(problem);
    }
    let mut edit = ContentEdit::new(format!("Rename {table}/{key} to {new_key}"));
    let usages = project.usages.of_row(table, key).to_vec();
    let mut by_file: BTreeMap<String, Vec<ContentUsage>> = Default::default();
    for usage in usages {
        by_file.entry(usage.file.clone()).or_default().push(usage);
    }
    let own = row_file(table, key);
    for (file, usages) in by_file {
        let Some(mut value) = project.files.json_owned(&file) else {
            continue;
        };
        for usage in usages {
            rewrite(&mut value, &usage.pointer, &project.usages, &file, new_key);
        }
        if file == own {
            edit.write_json(row_file(table, new_key), &value);
        } else {
            edit.write_json(file, &value);
        }
    }
    if !edit
        .changes
        .iter()
        .any(|change| change.file == row_file(table, new_key))
    {
        let value = project.files.json(&own).ok_or("the row is gone")?;
        edit.write_json(row_file(table, new_key), &value);
    }
    edit.remove(own);
    if let Some((from, to)) = companion_file(table, key).zip(companion_file(table, new_key))
        && let Some(bytes) = project.files.get(&from)
    {
        edit.write(to, bytes.clone());
        edit.remove(from);
    }
    for file in folder_files(project, table, key) {
        if let Some(bytes) = project.files.get(&file) {
            edit.write(
                file.replacen(
                    &format!("tables/{table}/{key}/"),
                    &format!("tables/{table}/{new_key}/"),
                    1,
                ),
                bytes.clone(),
            );
            edit.remove(file);
        }
    }
    let count = edit.changes.len();
    if !editor.apply(edit) {
        return Err("the rename was refused; see the output".to_owned());
    }
    editor.toast(format!(
        "Renamed {table}/{key} to {new_key}: {count} files rewritten"
    ));
    retarget_tabs(editor, table, key, new_key);
    if editor.selection.table == table {
        for selected in &mut editor.selection.keys {
            if selected == key {
                *selected = new_key.to_owned();
            }
        }
    }
    Ok(())
}

fn rewrite(
    value: &mut Value,
    pointer: &str,
    usages: &usages::ContentUsages,
    file: &str,
    new_key: &str,
) {
    let is_key = usages
        .in_file(file)
        .iter()
        .any(|reference| reference.pointer == pointer && reference.key);
    if is_key {
        let (parent, name) = split_pointer(pointer);
        if let Some(map) = value.pointer_mut(parent).and_then(Value::as_object_mut)
            && let Some(entry) = map.remove(&name)
        {
            map.insert(new_key.to_owned(), entry);
        }
    } else if let Some(slot) = value.pointer_mut(pointer) {
        *slot = Value::String(new_key.to_owned());
    }
}

fn split_pointer(pointer: &str) -> (&str, String) {
    let cut = pointer.rfind('/').unwrap_or(0);
    let name = pointer[cut + 1..].replace("~1", "/").replace("~0", "~");
    (&pointer[..cut], name)
}

fn retarget_tabs(editor: &mut Editor, table: &str, key: &str, new_key: &str) {
    for (_, tab) in editor.dock.iter_all_tabs_mut() {
        let fresh = match &*tab {
            EditorTab::Script(owner) => owner.renamed(table, key, new_key).map(EditorTab::Script),
            other if other.is_row(table, key) => Some(EditorTab::for_row(table, new_key)),
            _ => None,
        };
        if let Some(fresh) = fresh {
            *tab = fresh;
        }
    }
}

/// The file a row owns by convention besides its own, which follows the row through a copy, a
/// rename and a deletion: a shader's WGSL.
pub fn companion_file(table: &str, key: &str) -> Option<String> {
    (table == vfx::SHADER_TABLE).then(|| shader_file(key))
}

const NEW_SHADER: &str =
    "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return sample(site_uv(uv));\n}\n";

pub fn folder_files(project: &Project, table: &str, key: &str) -> Vec<String> {
    project
        .files
        .paths_under(&format!("tables/{table}/{key}/"))
        .cloned()
        .collect()
}

pub fn delete_dialog(editor: &mut Editor, table: &str, keys: Vec<String>) {
    let keys: Vec<String> = keys
        .into_iter()
        .filter(|key| {
            if editor.shapes.is_fixture(table, key) {
                editor
                    .output
                    .say("editor", format!("{table}/{key} is a fixture and stays"));
                false
            } else {
                true
            }
        })
        .collect();
    if keys.is_empty() {
        editor.toast("Fixtures can't be deleted");
        return;
    }
    let Some(project) = &editor.project else {
        return;
    };
    let own: Vec<String> = keys.iter().map(|key| row_file(table, key)).collect();
    let mut groups: Vec<UsageGroup> = Vec::new();
    for key in &keys {
        for usage in project.usages.of_row(table, key) {
            if own.contains(&usage.file) {
                continue;
            }
            let UsageKind {
                table: other_table,
                kind,
                entry,
            } = kind_of(project, &usage.file, &usage.pointer);
            let removable = entry.is_some();
            let same_group = |group: &&mut UsageGroup| {
                group.table == other_table
                    && group.removable == removable
                    && if removable {
                        group.kind == kind
                    } else {
                        group.usages.iter().any(|known| known.file == usage.file)
                    }
            };
            match groups.iter_mut().find(same_group) {
                Some(group) => group.usages.push(usage.clone()),
                None => groups.push(UsageGroup {
                    table: other_table,
                    kind,
                    usages: vec![usage.clone()],
                    removable,
                    whole_modules: entry == Some(Entry::Module),
                    decision: if removable {
                        UsageDecision::Remove
                    } else {
                        UsageDecision::Repoint(String::new())
                    },
                }),
            }
        }
    }
    editor.dialogs.push(ModalDialog::Delete(DeleteDialog {
        table: table.to_owned(),
        keys,
        groups,
        error: None,
    }));
}

pub struct UsageSummary {
    pub table: String,
    pub field: String,
    pub keys: Vec<String>,
}

pub fn usage_groups(project: &Project, table: &str, key: &str) -> Vec<UsageSummary> {
    let mut groups: Vec<UsageSummary> = Vec::new();
    for usage in project.usages.of_row(table, key) {
        let Some((other_table, other_key)) = row_of(&usage.file) else {
            continue;
        };
        let field = field_of(project, &usage.file, &usage.pointer);
        match groups
            .iter_mut()
            .find(|group| group.table == other_table && group.field == field)
        {
            Some(group) => {
                if !group.keys.iter().any(|k| k == other_key) {
                    group.keys.push(other_key.to_owned());
                }
            }
            None => groups.push(UsageSummary {
                table: other_table.to_owned(),
                field,
                keys: vec![other_key.to_owned()],
            }),
        }
    }
    groups
}

pub fn field_of(project: &Project, file: &str, pointer: &str) -> String {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    if segments.first() == Some(&"modules")
        && let Some(index) = segments.get(1)
        && let Some(row) = project.files.json(file)
        && let Some(kind) =
            row["modules"][index.parse::<usize>().unwrap_or(usize::MAX)]["type"].as_str()
    {
        return kind.to_owned();
    }
    segments
        .first()
        .map(|s| (*s).to_owned())
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    List,
    Module,
}

struct UsageKind {
    table: String,
    kind: String,
    entry: Option<Entry>,
}

fn kind_of(project: &Project, file: &str, pointer: &str) -> UsageKind {
    let table = row_of(file)
        .map(|(table, _)| table)
        .unwrap_or("game")
        .to_owned();
    let kind = field_of(project, file, pointer);
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let slot = (!segments.contains(&"nodes"))
        .then(|| list_slot(pointer))
        .flatten();
    let entry = match slot {
        None => None,
        Some((parent, index)) if parent.ends_with("/modules") => {
            let any_number = project
                .files
                .json(file)
                .and_then(|row| row["modules"].get(index).cloned())
                .and_then(|module| registry().module_rule(&table, &module))
                == Some(ModuleRule::AnyNumber);
            any_number.then_some(Entry::Module)
        }
        Some(_) => Some(Entry::List),
    };
    UsageKind { table, kind, entry }
}

fn list_slot(pointer: &str) -> Option<(String, usize)> {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let at = segments
        .iter()
        .rposition(|segment| segment.parse::<usize>().is_ok())?;
    let parent = if at == 0 {
        String::new()
    } else {
        format!("/{}", segments[..at].join("/"))
    };
    Some((parent, segments[at].parse().ok()?))
}

fn usage_detail(project: &Project, file: &str, pointer: &str) -> String {
    let (Some(row), Some((table, _))) = (project.files.json(file), row_of(file)) else {
        return String::new();
    };
    let parent = pointer
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let Some(at) = row.pointer(parent) else {
        return String::new();
    };
    let Some(entry) = at.as_object() else {
        return String::new();
    };
    let root = project.shapes.shape_of(table);
    let shape = schema::at_pointer(root, root, &row, parent).unwrap_or(&Value::Null);
    let mut parts: Vec<String> = Vec::new();
    for (name, value) in entry {
        if format!("{parent}/{name}") == pointer || name == "type" {
            continue;
        }
        let text = match value {
            Value::Number(number) => {
                let number = number.as_f64().unwrap_or(0.0);
                schema::number_shorthand(root, &shape["properties"][name], number)
                    .unwrap_or_else(|| format!("{name} {number}"))
            }
            Value::String(text) => format!("{name} {text}"),
            Value::Bool(true) => name.clone(),
            _ => continue,
        };
        parts.push(text);
    }
    parts.extend(
        schema::blank_meaning(shape, at)
            .into_iter()
            .map(str::to_owned),
    );
    parts.join(" · ")
}

pub fn delete_body(editor: &mut Editor, ui: &mut Ui, dialog: &mut DeleteDialog) {
    let Some(project) = &editor.project else {
        return;
    };
    let total: usize = dialog.groups.iter().map(|group| group.usages.len()).sum();
    let rows: usize = {
        let mut files: Vec<&String> = dialog
            .groups
            .iter()
            .flat_map(|group| group.usages.iter().map(|usage| &usage.file))
            .collect();
        files.sort();
        files.dedup();
        files.len()
    };
    if dialog.groups.is_empty() {
        ui.label(theme::muted(
            "Nothing names it. The file goes; its assets stay until nothing names them.",
        ));
    } else {
        let used = format!(
            "Used in {total} place{} in {rows} row{}. Nothing is deleted until every group has an answer.",
            widgets::plural(total),
            widgets::plural(rows)
        );
        ui.label(theme::muted(used));
    }
    let candidates: Vec<String> = project
        .files
        .paths_under(&format!("tables/{}/", dialog.table))
        .filter_map(|file| row_of(file).map(|(_, key)| key.to_owned()))
        .filter(|key| !dialog.keys.contains(key))
        .collect();
    let details: Vec<Vec<(String, String)>> = dialog
        .groups
        .iter()
        .map(|group| {
            let mut lines: Vec<(String, String, usize)> = Vec::new();
            for usage in &group.usages {
                let row = row_of(&usage.file)
                    .map(|(t, k)| format!("{t}/{k}"))
                    .unwrap_or_else(|| usage.file.clone());
                let mut detail = usage_detail(project, &usage.file, &usage.pointer);
                let line = if !group.removable {
                    let field = field_of(project, &usage.file, &usage.pointer);
                    detail = if detail.is_empty() {
                        field
                    } else {
                        format!("{field}  {detail}")
                    };
                    (String::new(), detail)
                } else {
                    (row, detail)
                };
                match lines
                    .iter_mut()
                    .find(|(r, d, _)| *r == line.0 && *d == line.1)
                {
                    Some((_, _, n)) => *n += 1,
                    None => lines.push((line.0, line.1, 1)),
                }
            }
            lines
                .into_iter()
                .map(|(row, detail, n)| {
                    if n > 1 {
                        (
                            row,
                            if detail.is_empty() {
                                format!("× {n}")
                            } else {
                                format!("{detail} × {n}")
                            },
                        )
                    } else {
                        (row, detail)
                    }
                })
                .collect()
        })
        .collect();
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .max_height(440.0)
        .show(ui, |ui| {
            for (index, group) in dialog.groups.iter_mut().enumerate() {
                let title = group_title(project, group);
                widgets::card_frame(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let badge = if group.kind.is_empty() {
                            &group.table
                        } else {
                            &group.kind
                        };
                        widgets::badge(ui, badge);
                        ui.label(theme::strong(&title));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            decision_picker(
                                ui,
                                &project.shapes,
                                &dialog.table,
                                index,
                                group,
                                &candidates,
                            );
                        });
                    });
                    ui.add_space(2.0);
                    usage_lines(ui, index, &details[index]);
                });
                ui.add_space(4.0);
            }
        });
    let files_removed = dialog.keys.len();
    let assets_named = dialog.keys.iter().any(|key| {
        project
            .usages
            .in_file(&row_file(&dialog.table, key))
            .iter()
            .any(|reference| reference.kind == LeafKind::Asset)
    });
    ui.add_space(4.0);
    let mut summary = format!(
        "{total} edit{} in {rows} row{} and {files_removed} deleted file{}, one undo step.",
        widgets::plural(total),
        widgets::plural(rows),
        widgets::plural(files_removed)
    );
    if dialog.groups.is_empty() {
        summary = format!(
            "{files_removed} deleted file{}, one undo step.",
            widgets::plural(files_removed)
        );
    }
    if assets_named {
        summary.push_str(" Its files stay in assets/ until nothing names them.");
    }
    ui.label(theme::muted(summary).size(12.5));
    if let Some(error) = &dialog.error {
        ui.colored_label(theme::DANGER, error);
    }
}

fn group_title(project: &Project, group: &UsageGroup) -> String {
    let mut files: Vec<&String> = group.usages.iter().map(|usage| &usage.file).collect();
    files.sort();
    files.dedup();
    let mut kinds: Vec<String> = group
        .usages
        .iter()
        .map(|usage| field_of(project, &usage.file, &usage.pointer))
        .collect();
    kinds.sort();
    kinds.dedup();
    if files.len() != 1 {
        return format!(
            "{} · {} {}s",
            group.kind.replace('_', " "),
            files.len(),
            group.table
        );
    }
    let row = row_of(files[0])
        .map(|(t, k)| format!("{t}/{k}"))
        .unwrap_or_else(|| files[0].clone());
    match kinds.as_slice() {
        [kind] if !kind.is_empty() && *kind != group.table => {
            format!("{row} · {}", kind.replace('_', " "))
        }
        _ => row,
    }
}

fn decision_picker(
    ui: &mut Ui,
    shapes: &SchemaShapes,
    table: &str,
    index: usize,
    group: &mut UsageGroup,
    candidates: &[String],
) {
    let mut repoint = matches!(group.decision, UsageDecision::Repoint(_)) || !group.removable;
    if group.removable {
        let remove_label = if group.whole_modules {
            format!("Remove the {} modules", group.kind.replace('_', " "))
        } else {
            format!("Remove {}", usage_phrase(shapes, &group.table, &group.kind))
        };
        let unpackable = table == "blueprint" && group.whole_modules;
        let label = match group.decision {
            UsageDecision::Repoint(_) => "Replace with…".to_owned(),
            UsageDecision::Unpack => "Unpack into scenery".to_owned(),
            _ => remove_label.clone(),
        };
        widgets::combo_box(("decision", index))
            .selected_text(theme::mono(label).size(13.0))
            .show_ui(ui, |ui| {
                if unpackable
                    && ui
                        .selectable_label(
                            group.decision == UsageDecision::Unpack,
                            "Unpack into scenery",
                        )
                        .on_hover_text(
                            "Each placement becomes its parts, placed, and looks exactly the same",
                        )
                        .clicked()
                {
                    group.decision = UsageDecision::Unpack;
                    repoint = false;
                }
                if ui
                    .selectable_label(
                        matches!(group.decision, UsageDecision::Remove),
                        &remove_label,
                    )
                    .clicked()
                {
                    group.decision = UsageDecision::Remove;
                    repoint = false;
                }
                if ui.selectable_label(repoint, "Replace with…").clicked() && !repoint {
                    group.decision = UsageDecision::Repoint(String::new());
                    repoint = true;
                }
            });
    }
    if !repoint {
        return;
    }
    let mut picked = match &group.decision {
        UsageDecision::Repoint(key) => key.clone(),
        _ => String::new(),
    };
    let selected = if picked.is_empty() {
        theme::muted("pick a replacement…").size(13.0)
    } else {
        theme::code(format!("{table}/ {picked}")).size(13.0)
    };
    ui.allocate_ui_with_layout(
        egui::vec2(170.0, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            widgets::combo_box(("repoint", index))
                .width(160.0)
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for candidate in candidates {
                        if ui
                            .selectable_label(picked == *candidate, theme::mono(candidate))
                            .clicked()
                        {
                            picked = candidate.clone();
                        }
                    }
                });
        },
    );
    if !picked.is_empty() && group.decision != UsageDecision::Repoint(picked.clone()) {
        group.decision = UsageDecision::Repoint(picked);
    }
    ui.label(theme::muted("Replace with").size(12.5));
}

fn usage_lines(ui: &mut Ui, index: usize, lines: &[(String, String)]) {
    egui::Grid::new(("usages", index))
        .num_columns(2)
        .spacing([14.0, 2.0])
        .show(ui, |ui| {
            for (row, detail) in lines {
                if row.is_empty() {
                    ui.label(theme::mono(detail).size(12.5).color(theme::TEXT));
                    ui.label("");
                } else {
                    ui.label(theme::mono(row).size(12.5).color(theme::TEXT));
                    ui.label(theme::muted(detail).monospace().size(12.5));
                }
                ui.end_row();
            }
        });
}

fn usage_phrase(shapes: &SchemaShapes, table: &str, field: &str) -> String {
    shapes
        .tables
        .get(table)
        .and_then(|table| table.shape["properties"][field]["x-usage"].as_str())
        .unwrap_or("the entries")
        .to_owned()
}

pub fn delete(editor: &mut Editor, dialog: &DeleteDialog) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    if let Some(group) = dialog
        .groups
        .iter()
        .find(|group| group.decision == UsageDecision::Undecided)
    {
        return Err(format!("decide what happens to {}", group.kind));
    }
    let mut edit = ContentEdit::new(format!(
        "Delete {} {}",
        dialog.table,
        dialog.keys.join(", ")
    ));
    let mut by_file: BTreeMap<String, Vec<(&ContentUsage, &UsageDecision)>> = Default::default();
    for group in &dialog.groups {
        for usage in &group.usages {
            by_file
                .entry(usage.file.clone())
                .or_default()
                .push((usage, &group.decision));
        }
    }
    for (file, mut entries) in by_file {
        let Some(mut value) = project.files.json_owned(&file) else {
            continue;
        };
        entries.sort_by(|a, b| b.0.pointer.cmp(&a.0.pointer));
        for (usage, decision) in entries {
            match decision {
                UsageDecision::Repoint(key) => {
                    rewrite(&mut value, &usage.pointer, &project.usages, &file, key)
                }
                UsageDecision::Remove => remove_entry(&mut value, &usage.pointer),
                UsageDecision::Unpack => unpack_entry(project, &mut value, &usage.pointer),
                UsageDecision::Undecided => {}
            }
        }
        edit.write_json(file, &value);
    }
    for key in &dialog.keys {
        edit.remove(row_file(&dialog.table, key));
        if let Some(file) = companion_file(&dialog.table, key)
            && project.files.contains(&file)
        {
            edit.remove(file);
        }
        for file in folder_files(project, &dialog.table, key) {
            edit.remove(file);
        }
    }
    if !editor.apply(edit) {
        return Err("the deletion was refused; see the output".to_owned());
    }
    for key in &dialog.keys {
        close_tabs(editor, &dialog.table, key);
    }
    if editor.selection.table == dialog.table {
        editor
            .selection
            .keys
            .retain(|key| !dialog.keys.contains(key));
    }
    editor.toast(format!(
        "Deleted {} {}",
        dialog.table,
        dialog.keys.join(", ")
    ));
    Ok(())
}

fn unpack_entry(project: &Project, value: &mut Value, pointer: &str) {
    let Some(content) = project.content.clone() else {
        return;
    };
    let Some((parent, index)) = list_slot(pointer) else {
        return;
    };
    let Some(list) = value.pointer_mut(&parent).and_then(Value::as_array_mut) else {
        return;
    };
    if index >= list.len() {
        return;
    }
    let Some(parts) = placement::unpacked(&content, &list[index]) else {
        return;
    };
    list.remove(index);
    for (offset, part) in parts.into_iter().enumerate() {
        list.insert(index + offset, part);
    }
}

fn remove_entry(value: &mut Value, pointer: &str) {
    if let Some((parent, index)) = list_slot(pointer)
        && let Some(list) = value.pointer_mut(&parent).and_then(Value::as_array_mut)
        && index < list.len()
    {
        list.remove(index);
    }
}

pub fn close_tabs(editor: &mut Editor, table: &str, key: &str) {
    let gone: Vec<EditorTab> = editor
        .dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .filter(|tab| tab.is_row(table, key))
        .collect();
    for tab in gone {
        if let Some(path) = editor.dock.find_tab(&tab) {
            editor.dock.remove_tab(path);
        }
    }
}
