use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use ui::chip;
use ui::egui;
use ui::face;
use ui::form;
use ui::input;
use ui::menu;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::audio::playback::SfxDef;
use engine::core::content::serial::TypeTag;
use engine::core::content::{ContentFile, MODULES, fields_file, name_of, parent_of};
use engine::field;
use engine::systems::input::map::{BoundAction, InputDef};
use engine::systems::npc::{self, NpcDef};
use engine::systems::registry;
use engine::systems::script::RiftScripts;
use serde_json::Value;
use ui::egui::{Color32, Sense, Ui, Vec2};

use crate::app::Editor;
use crate::forms::{self, FormCx, FormState};
use crate::icons::{self, IconTone};
use crate::keymap::EditorKey;
use crate::project::{ContentEdit, RowSelection};
use crate::row_form::tune::{self, SfxTuneField, sfx_tune_field};
use crate::schema::{self, SchemaKind};
use crate::tabs::shown_path;
use crate::tabs::{EditorTab, file_title};
use crate::user::{FilterComparison, SheetFilter, SpreadsheetView};
use strum::VariantArray;
use ui::controls;

mod bindings;

const ROW_H: f32 = 29.0;
const FILE_W: f32 = 110.0;
const CELL_W: f32 = 84.0;
const MAX_DEFAULT_COLUMNS: usize = 9;

#[derive(Default)]
pub struct SpreadsheetState {
    pub search: String,
    pub selected: Option<(String, String)>,
    pub anchor: Option<(String, String)>,
    pub editing: Option<String>,
    pub form: FormState,
    pub last_edit: Option<(String, String, String)>,
    focus_editor: bool,
    columns: Option<(u64, Arc<Vec<SpreadsheetColumn>>)>,
    rows: Option<(u64, Arc<Vec<SpreadsheetRow>>)>,
}

impl SpreadsheetState {
    fn open_cell(&mut self, column: String) {
        self.editing = Some(column);
        self.form = FormState::default();
        self.focus_editor = true;
    }
}

/// Where a sheet's rows come from.
#[derive(Clone, Debug, PartialEq)]
enum SheetRows {
    /// Every file of the table, a row each.
    Files,
    /// Every entry of the map at `field` in the table's one file: rows the engine fixes, which
    /// the sheet edits but never adds, removes or renames.
    Fixed { file: String, field: String },
}

impl SheetRows {
    pub fn of(editor: &Editor, table: &str) -> SheetRows {
        let field = editor.shapes.table(table).and_then(|shape| {
            schema::properties(&shape.shape)
                .into_iter()
                .find(|(_, field)| schema::fixed_rows(field))
                .map(|(name, _)| name.clone())
        });
        let file = editor
            .project
            .as_ref()
            .and_then(|project| project.files.paths_of(table).into_iter().next());
        match (field, file) {
            (Some(field), Some(file)) => SheetRows::Fixed { file, field },
            _ => SheetRows::Files,
        }
    }

    /// The columns every sheet leads with, whatever else it shows, so its rows are told apart:
    /// by their files, or by the entries they are.
    fn pinned(&self) -> Vec<SpreadsheetColumn> {
        let place = |id: &str, title: &str, source| SpreadsheetColumn {
            id: id.to_owned(),
            title: title.to_owned(),
            sub: String::new(),
            source,
            schema: Value::Null,
            numeric: false,
            cell: CellKind::Form,
        };
        match self {
            SheetRows::Files => vec![
                place("#file", "file", ColumnSource::File),
                place("#folder", "folder", ColumnSource::Folder),
            ],
            SheetRows::Fixed { field, .. } => {
                vec![place("#entry", singular(field), ColumnSource::Entry)]
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ColumnSource {
    File,
    Folder,
    Entry,
    First(Vec<String>),
    Module { tag: String, path: Vec<String> },
}

/// How a column's cells read and edit: as a form of their schema, or with an editor of their own.
#[derive(Clone, Copy, Debug)]
pub enum CellKind {
    Form,
    /// A sound's volume or pitch, on a slider.
    Tune(&'static SfxTuneField),
    /// An action's bindings, each captured from the key or click itself.
    Bindings,
}

impl CellKind {
    fn of(table: &str, path: &[String]) -> CellKind {
        let [name] = path else {
            return CellKind::Form;
        };
        match table {
            SfxDef::TABLE => sfx_tune_field(name).map_or(CellKind::Form, CellKind::Tune),
            InputDef::TABLE if name == &field!(BoundAction, bindings)[1..] => CellKind::Bindings,
            _ => CellKind::Form,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpreadsheetColumn {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub source: ColumnSource,
    pub schema: Value,
    pub numeric: bool,
    pub cell: CellKind,
}

/// A row as its sheet reads it: its fields, and its modules each as its type writes it, or why
/// they don't read. A row is a file of the table, or an entry of the one file, at `pointer`.
#[derive(Clone)]
pub struct SpreadsheetRow {
    pub key: String,
    pub file: String,
    pub pointer: String,
    pub value: Arc<Value>,
    pub modules: Arc<Vec<Value>>,
    pub unread: Option<String>,
}

impl SpreadsheetRow {
    /// Where the row is, as the editor names it to the designer.
    pub fn place(&self) -> String {
        match self.pointer.as_str() {
            "" => shown_path(&self.key).to_owned(),
            _ => format!("{} › {}", file_title(&self.file), self.key),
        }
    }

    fn has_module(&self, tag: &str) -> bool {
        self.module(tag).is_some()
    }

    fn module(&self, tag: &str) -> Option<&Value> {
        self.modules
            .iter()
            .find(|module| TypeTag::of_json(module).as_deref() == Some(tag))
    }
}

impl SpreadsheetColumn {
    pub fn get<'a>(&self, row: &'a SpreadsheetRow) -> Option<&'a Value> {
        match &self.source {
            ColumnSource::File | ColumnSource::Folder | ColumnSource::Entry => None,
            ColumnSource::First(path) => dig(&row.value, path),
            ColumnSource::Module { tag, path } => dig(row.module(tag)?, path),
        }
    }

    pub fn text(&self, row: &SpreadsheetRow) -> Option<String> {
        match self.source {
            ColumnSource::File => Some(name_of(&row.key).to_owned()),
            ColumnSource::Folder => Some(parent_of(&row.key).to_owned()),
            ColumnSource::Entry => Some(row.key.clone()),
            _ => match self.cell {
                CellKind::Bindings => self.get(row).map(bindings::names),
                _ => self.get(row).map(cell_text),
            },
        }
    }

    /// Whether a cell of the column is written into its row, rather than read off the row's place.
    pub fn editable(&self) -> bool {
        !matches!(
            self.source,
            ColumnSource::File | ColumnSource::Folder | ColumnSource::Entry
        )
    }

    pub fn number(&self, row: &SpreadsheetRow) -> Option<f64> {
        self.get(row).and_then(Value::as_f64)
    }

    fn lacks_module(&self, row: &SpreadsheetRow) -> bool {
        match &self.source {
            ColumnSource::Module { tag, .. } => row.unread.is_none() && !row.has_module(tag),
            _ => false,
        }
    }

    /// Why the cell can't show the row's value: its modules don't read.
    pub fn unread<'r>(&self, row: &'r SpreadsheetRow) -> Option<&'r str> {
        match &self.source {
            ColumnSource::Module { .. } => row.unread.as_deref(),
            _ => None,
        }
    }

    pub fn origin(&self) -> String {
        match &self.source {
            ColumnSource::File => "the file's name".to_owned(),
            ColumnSource::Folder => "the folder the file sits in".to_owned(),
            ColumnSource::Entry => "the entry the row is".to_owned(),
            ColumnSource::First(path) if path.len() == 1 => "the row itself".to_owned(),
            ColumnSource::First(path) => {
                format!("the {} list", path[..path.len() - 1].join("."))
            }
            ColumnSource::Module { tag, path } if path.len() == 1 => format!("the {tag} module"),
            ColumnSource::Module { tag, path } => {
                format!("{} in the {tag} module", path[..path.len() - 1].join("."))
            }
        }
    }
}

fn dig<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = value;
    for step in path {
        current = current.get(step)?;
    }
    Some(current)
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => "none".to_owned(),
        Value::Bool(flag) => if *flag { "yes" } else { "no" }.to_owned(),
        Value::Number(number) => number
            .as_f64()
            .map(|number| number.to_string())
            .unwrap_or_else(|| number.to_string()),
        Value::String(text) => text.clone(),
        Value::Array(items) => format!("{} items", items.len()),
        Value::Object(fields) => format!("{} fields", fields.len()),
    }
}

fn columns_of(editor: &Editor, table: &str, rows: &[SpreadsheetRow]) -> Vec<SpreadsheetColumn> {
    let mut columns = Vec::new();
    let Some(shape) = editor.shapes.table(table) else {
        return columns;
    };
    let root = &shape.shape;
    if let SheetRows::Fixed { field, .. } = SheetRows::of(editor, table) {
        let entry = schema::properties(root)
            .into_iter()
            .find(|(name, _)| **name == field)
            .and_then(|(_, map)| match schema::kind(root, map) {
                SchemaKind::Map(entry) => Some(schema::resolve(root, entry)),
                _ => None,
            });
        for (name, schema) in entry.map(schema::properties).unwrap_or_default() {
            let path = vec![name.clone()];
            columns.push(SpreadsheetColumn {
                id: name.clone(),
                title: name.clone(),
                sub: singular(&field).to_owned(),
                numeric: is_numeric(root, schema),
                cell: CellKind::of(table, &path),
                source: ColumnSource::First(path),
                schema: schema.clone(),
            });
        }
        return columns;
    }
    let mut path = Vec::new();
    leaves(root, root, &mut path, &mut |path, schema| {
        if path.first().is_some_and(|first| first == MODULES) {
            return;
        }
        let sub = match path.len() {
            1 => "row".to_owned(),
            _ => singular(&path[path.len() - 2]).to_owned(),
        };
        columns.push(SpreadsheetColumn {
            id: path.join("."),
            title: path.last().cloned().unwrap_or_default(),
            sub,
            cell: CellKind::of(table, path),
            source: ColumnSource::First(path.to_vec()),
            numeric: is_numeric(root, schema),
            schema: schema.clone(),
        });
    });
    for variant in editor.shapes.module_variants(table) {
        let mut path = Vec::new();
        leaves(root, variant.schema, &mut path, &mut |path, schema| {
            if path.first().is_some_and(|first| first == &variant.field) {
                return;
            }
            let sub = match path.len() {
                1 => variant.tag.clone(),
                _ => singular(&path[path.len() - 2]).to_owned(),
            };
            columns.push(SpreadsheetColumn {
                id: format!("{}.{}", variant.tag, path.join(".")),
                title: path.last().cloned().unwrap_or_default(),
                sub,
                cell: CellKind::Form,
                source: ColumnSource::Module {
                    tag: variant.tag.clone(),
                    path: path.to_vec(),
                },
                numeric: is_numeric(root, schema),
                schema: schema.clone(),
            });
        });
        let mut keyed: BTreeMap<(String, String), bool> = BTreeMap::new();
        for row in rows {
            for module in row
                .modules
                .iter()
                .filter(|m| TypeTag::of_json(m).as_ref() == Some(&variant.tag))
            {
                for (name, field) in schema::properties(variant.schema) {
                    if !matches!(schema::kind(root, field), SchemaKind::Map(_)) {
                        continue;
                    }
                    if let Some(map) = module[name].as_object() {
                        for (entry, value) in map {
                            keyed
                                .entry((name.clone(), entry.clone()))
                                .or_insert(value.is_number());
                        }
                    }
                }
            }
        }
        for ((map_name, entry), numeric) in keyed {
            columns.push(SpreadsheetColumn {
                id: format!("{}.{map_name}.{entry}", variant.tag),
                title: entry.clone(),
                sub: singular(&map_name).to_owned(),
                cell: CellKind::Form,
                source: ColumnSource::Module {
                    tag: variant.tag.clone(),
                    path: vec![map_name, entry],
                },
                schema: if numeric {
                    schema::of_type::<f64>().clone()
                } else {
                    schema::of_type::<String>().clone()
                },
                numeric,
            });
        }
    }
    columns
}

fn singular(plural: &str) -> &str {
    plural.trim_end_matches('s')
}

fn is_numeric(root: &Value, schema: &Value) -> bool {
    matches!(
        schema::kind(root, schema),
        SchemaKind::Integer | SchemaKind::Number
    )
}

fn leaves(
    root: &Value,
    schema: &Value,
    path: &mut Vec<String>,
    out: &mut dyn FnMut(&[String], &Value),
) {
    if path.len() > 3 {
        return;
    }
    let schema = schema::resolve(root, schema);
    match schema::kind(root, schema) {
        SchemaKind::Object if schema::marker(schema).is_none() => {
            for (name, field) in schema::properties(schema) {
                path.push(name.clone());
                leaves(root, field, path, out);
                path.pop();
            }
        }
        SchemaKind::Text
        | SchemaKind::Integer
        | SchemaKind::Number
        | SchemaKind::Boolean
        | SchemaKind::Enum(_)
        | SchemaKind::Object => {
            if !path.is_empty() {
                out(path, schema);
            }
        }
        SchemaKind::Choice(alternatives)
            if !path.is_empty()
                && alternatives.iter().any(|alt| {
                    matches!(
                        schema::kind(root, alt),
                        SchemaKind::Text
                            | SchemaKind::Integer
                            | SchemaKind::Number
                            | SchemaKind::Boolean
                            | SchemaKind::Enum(_)
                    )
                }) =>
        {
            out(path, schema);
        }
        _ => {}
    }
}

fn rows_of(editor: &Editor, table: &str) -> Vec<SpreadsheetRow> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    if let SheetRows::Fixed { file, field } = SheetRows::of(editor, table) {
        let Some(value) = project.files.json(&fields_file(&file)) else {
            return Vec::new();
        };
        let entries = value.get(&field).and_then(Value::as_object);
        return entries
            .into_iter()
            .flatten()
            .map(|(key, entry)| SpreadsheetRow {
                key: key.clone(),
                file: file.clone(),
                pointer: format!("/{field}/{key}"),
                value: Arc::new(entry.clone()),
                modules: Arc::new(Vec::new()),
                unread: None,
            })
            .collect();
    }
    let reader = project.reader();
    let holds_modules = registry().holds_modules(table);
    let mut rows: Vec<SpreadsheetRow> = project
        .files
        .paths_of(table)
        .into_iter()
        .filter_map(|key| {
            let value = project.files.json(&fields_file(&key))?;
            let read = if holds_modules {
                reader.modules_in(table, &value)
            } else {
                Ok(Vec::new())
            };
            let (modules, unread) = match read {
                Ok(modules) => (modules, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            Some(SpreadsheetRow {
                value,
                modules: Arc::new(modules),
                unread,
                file: key.clone(),
                pointer: String::new(),
                key,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        parent_of(&a.key)
            .cmp(parent_of(&b.key))
            .then_with(|| file_title(&a.key).cmp(file_title(&b.key)))
    });
    rows
}

fn view_of(editor: &Editor, table: &str, columns: &[SpreadsheetColumn]) -> SpreadsheetView {
    let mut view = editor
        .settings
        .views
        .get(table)
        .cloned()
        .unwrap_or_default();
    if view.columns.is_empty() {
        view.columns = columns
            .iter()
            .take(MAX_DEFAULT_COLUMNS)
            .map(|column| column.id.clone())
            .collect();
    }
    view
}

fn save_view(editor: &mut Editor, table: &str, view: SpreadsheetView) {
    editor.settings.views.insert(table.to_owned(), view);
    editor.save_settings();
}

fn can_be_fought(editor: &Editor) -> BTreeSet<String> {
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
    else {
        return BTreeSet::new();
    };
    if !content.has_built::<RiftScripts>() {
        return BTreeSet::new();
    }
    let scripts = content.built::<RiftScripts>();
    content
        .ids::<NpcDef>()
        .filter(|&id| npc::can_be_fought(scripts, id))
        .map(|id| id.path(&content).to_owned())
        .collect()
}

fn passes(
    row: &SpreadsheetRow,
    filter: &SheetFilter,
    fought: &BTreeSet<String>,
    columns: &[SpreadsheetColumn],
) -> bool {
    match filter {
        SheetFilter::CanBeFought => fought.contains(&row.key),
        SheetFilter::Has(tag) => row.has_module(tag),
        SheetFilter::Lacks(tag) => !row.has_module(tag),
        SheetFilter::Compare {
            column,
            test,
            value,
        } => columns
            .iter()
            .find(|known| known.id == *column)
            .and_then(|column| column.number(row))
            .is_none_or(|have| test.holds(have, *value)),
    }
}

fn matches_search(row: &SpreadsheetRow, search: &str, columns: &[SpreadsheetColumn]) -> bool {
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    row.key.to_lowercase().contains(&needle)
        || columns
            .iter()
            .filter_map(|column| column.text(row))
            .any(|text| text.to_lowercase().contains(&needle))
}

fn sort_rows(rows: &mut [SpreadsheetRow], column: &SpreadsheetColumn, descending: bool) {
    rows.sort_by(|a, b| {
        let order = if column.numeric {
            ordered(column.number(a), column.number(b), |x, y| {
                x.partial_cmp(y).unwrap_or(Ordering::Equal)
            })
        } else {
            ordered(column.text(a), column.text(b), |x, y| {
                x.to_lowercase().cmp(&y.to_lowercase())
            })
        };
        if descending { order.reverse() } else { order }.then_with(|| a.key.cmp(&b.key))
    });
}

fn ordered<T>(a: Option<T>, b: Option<T>, compare: impl Fn(&T, &T) -> Ordering) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => compare(&x, &y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn shown_rows(
    editor: &mut Editor,
    table: &str,
) -> (
    Vec<SpreadsheetRow>,
    Arc<Vec<SpreadsheetColumn>>,
    SpreadsheetView,
    usize,
) {
    let all = rows_cached(editor, table);
    let columns = columns_cached(editor, table, &all);
    let view = view_of(editor, table, &columns);
    let fought = if view.filters.contains(&SheetFilter::CanBeFought) {
        can_be_fought(editor)
    } else {
        BTreeSet::new()
    };
    let search = editor
        .views
        .spreadsheet
        .get(table)
        .map(|state| state.search.clone())
        .unwrap_or_default();
    let total = all.len();
    let mut rows: Vec<SpreadsheetRow> = all
        .iter()
        .filter(|row| {
            view.filters
                .iter()
                .all(|filter| passes(row, filter, &fought, &columns))
        })
        .filter(|row| matches_search(row, &search, &columns))
        .cloned()
        .collect();
    let pinned = SheetRows::of(editor, table).pinned();
    if let Some((id, descending)) = &view.sort
        && let Some(column) = pinned.iter().chain(columns.iter()).find(|c| c.id == *id)
    {
        sort_rows(&mut rows, column, *descending);
    }
    (rows, columns, view, total)
}

fn rows_cached(editor: &mut Editor, table: &str) -> Arc<Vec<SpreadsheetRow>> {
    let revision = files_revision(editor);
    if let Some((built_for, rows)) = &editor.views.spreadsheet.view_or_default(table).rows
        && *built_for == revision
    {
        return rows.clone();
    }
    let rows = Arc::new(rows_of(editor, table));
    editor.views.spreadsheet.view_or_default(table).rows = Some((revision, rows.clone()));
    rows
}

fn columns_cached(
    editor: &mut Editor,
    table: &str,
    rows: &[SpreadsheetRow],
) -> Arc<Vec<SpreadsheetColumn>> {
    let revision = files_revision(editor);
    if let Some((built_for, columns)) = &editor.views.spreadsheet.view_or_default(table).columns
        && *built_for == revision
    {
        return columns.clone();
    }
    let columns = Arc::new(columns_of(editor, table, rows));
    editor.views.spreadsheet.view_or_default(table).columns = Some((revision, columns.clone()));
    columns
}

fn files_revision(editor: &Editor) -> u64 {
    editor
        .project
        .as_ref()
        .map_or(0, |project| project.files.revision())
}

pub fn show(editor: &mut Editor, ui: &mut Ui, table: &str) {
    let selection = editor.selection.clone();
    ui::panel::inspector(ui, editor, "sheet-column", |editor, ui| {
        if selection.table == table
            && selection
                .parts
                .first()
                .is_some_and(|part| sheet_column(part).is_some())
        {
            inspect(editor, ui, &selection);
        } else {
            form::nothing_selected(ui, "Select a column to inspect it here.");
        }
    });
    egui::Frame::new()
        .inner_margin(crate::widgets::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            sheet(editor, ui, table);
        });
}

fn sheet(editor: &mut Editor, ui: &mut Ui, table: &str) {
    let (rows, columns, mut view, total) = shown_rows(editor, table);
    let pinned = SheetRows::of(editor, table).pinned();
    let shown: Vec<&SpreadsheetColumn> = pinned
        .iter()
        .chain(
            view.columns
                .iter()
                .filter_map(|id| columns.iter().find(|c| c.id == *id)),
        )
        .collect();
    let mut view_changed = false;
    view_changed |= toolbar(editor, ui, table, &columns, &mut view, rows.len(), total);
    ui.add_space(6.0);
    let shown: Vec<SpreadsheetColumn> = shown.into_iter().cloned().collect();
    view_changed |= table_body(editor, ui, table, &rows, &shown, &mut view);
    if view_changed {
        save_view(editor, table, view);
    }
}

fn toolbar(
    editor: &mut Editor,
    ui: &mut Ui,
    table: &str,
    columns: &[SpreadsheetColumn],
    view: &mut SpreadsheetView,
    shown: usize,
    total: usize,
) -> bool {
    let mut changed = false;
    let mut filters = view.filters.clone();
    let module_tags: Vec<String> = editor
        .shapes
        .module_variants(table)
        .into_iter()
        .map(|v| v.tag)
        .collect();
    ui.horizontal(|ui| {
        let state = editor.views.spreadsheet.view_or_default(table);
        ui.allocate_ui(Vec2::new(220.0, 26.0), |ui| {
            input::search(ui, &mut state.search, "Filter rows", None);
        });
        let mut removed = None;
        for (index, filter) in filters.iter().enumerate() {
            let (_, closed) = chip::closable(ui, &filter.label());
            if closed {
                removed = Some(index);
            }
        }
        if let Some(index) = removed {
            filters.remove(index);
            changed = true;
        }
        menu::button(ui, "+ filter", |ui| {
            let mut offered: Vec<SheetFilter> = Vec::new();
            if table == NpcDef::TABLE {
                offered.push(SheetFilter::CanBeFought);
            }
            offered.extend(module_tags.iter().cloned().map(SheetFilter::Has));
            offered.extend(module_tags.iter().cloned().map(SheetFilter::Lacks));
            for filter in offered {
                if ui.button(filter.label()).clicked() {
                    filters.push(filter);
                    changed = true;
                    ui.close();
                }
            }
            ui.separator();
            if let Some(compare) = comparison_picker(ui, columns) {
                filters.push(compare);
                changed = true;
                ui.close();
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(theme::muted(format!("{shown} of {total} rows")));
            menu::button(ui, "columns", |ui| {
                ui.set_min_width(220.0);
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for column in columns.iter() {
                            let mut on = view.columns.contains(&column.id);
                            let label = if column.sub.is_empty() {
                                column.title.clone()
                            } else {
                                format!("{} · {}", column.title, column.sub)
                            };
                            if ui.checkbox(&mut on, label).changed() {
                                if on {
                                    view.columns.push(column.id.clone())
                                } else {
                                    view.columns.retain(|id| *id != column.id)
                                }
                                changed = true;
                            }
                        }
                    });
            });
        });
    });
    if changed {
        view.filters = filters;
    }
    changed
}

struct CellCommit {
    row: SpreadsheetRow,
    column: SpreadsheetColumn,
    value: Value,
    label: String,
}

struct Hit {
    key: String,
    column: String,
    extend: bool,
    double: bool,
}

fn table_body(
    editor: &mut Editor,
    ui: &mut Ui,
    table: &str,
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    view: &mut SpreadsheetView,
) -> bool {
    let mut view_changed = false;
    let mut hit: Option<Hit> = None;
    let mut sort_click: Option<String> = None;
    let grid = egui::Id::new(("spreadsheet-grid", table));
    let grid_focused = ui.memory(|m| m.has_focus(grid));
    let keyboard_free = grid_focused || !ui.ctx().egui_wants_keyboard_input();
    let (selected, anchor, editing) = {
        let state = editor.views.spreadsheet.view_or_default(table);
        (
            state.selected.clone(),
            state.anchor.clone(),
            state.editing.clone(),
        )
    };
    let range = selection_range(rows, shown, &selected, &anchor);
    let mut commit: Option<CellCommit> = None;
    let mut stop_editing = false;
    let widths: Vec<f32> = shown
        .iter()
        .map(|column| match column.source {
            ColumnSource::File | ColumnSource::Entry => FILE_W,
            ColumnSource::Folder => 220.0,
            _ if matches!(column.cell, CellKind::Bindings) => 300.0,
            _ if column.title == "label" => 300.0,
            _ if column.title == "name" => 120.0,
            _ if column.title == "script" => 150.0,
            _ => CELL_W,
        })
        .collect();
    let header_h = 36.0;
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        for (column, width) in shown.iter().zip(&widths) {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(*width, header_h), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &column.title)
            });
            let painter = ui.painter();
            let sorted = view.sort.as_ref().filter(|(id, _)| *id == column.id);
            let title = match sorted {
                Some((_, true)) => format!("{} ↓", column.title.to_uppercase()),
                Some((_, false)) => format!("{} ↑", column.title.to_uppercase()),
                None => column.title.to_uppercase(),
            };
            let align = if column.numeric {
                egui::Align2::RIGHT_TOP
            } else {
                egui::Align2::LEFT_TOP
            };
            let x = if column.numeric {
                rect.max.x - 6.0
            } else {
                rect.min.x + 4.0
            };
            paint::text_within(
                painter,
                egui::pos2(x, rect.min.y + 2.0),
                align,
                &title,
                egui::FontId::new(11.5, theme::medium()),
                theme::MUTED,
                *width - 10.0,
            );
            painter.text(
                egui::pos2(x, rect.min.y + 18.0),
                align,
                &column.sub,
                egui::FontId::proportional(10.5),
                theme::FAINT,
            );
            if response.clicked() {
                sort_click = Some(column.id.clone());
            }
            if response.hovered() {
                response.on_hover_text(format!("{} · {}", column.id, column.origin()));
            }
        }
    });
    if let Some(id) = sort_click {
        view.sort = match &view.sort {
            Some((current, true)) if *current == id => Some((id, false)),
            Some((current, false)) if *current == id => None,
            _ => Some((id, true)),
        };
        view_changed = true;
    }
    let separator = ui.available_rect_before_wrap();
    ui.painter().hline(
        separator.min.x..=separator.max.x,
        separator.min.y,
        egui::Stroke::new(1.0, theme::BORDER),
    );
    let mut more_note = None;
    let mut icon_budget = icons::DECODES_PER_FRAME;
    let output = egui::ScrollArea::both()
        .id_salt(("sheet", table))
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 26.0).max(ROW_H))
        .show_rows(ui, ROW_H, rows.len(), |ui, visible| {
            for index in visible {
                let row = &rows[index];
                let is_selected_row = selected.as_ref().is_some_and(|(key, _)| *key == row.key);
                let (line, _) = ui.allocate_exact_size(
                    Vec2::new(
                        ui.available_width().max(22.0 + widths.iter().sum::<f32>()),
                        ROW_H,
                    ),
                    Sense::hover(),
                );
                let painter = ui.painter_at(line);
                if is_selected_row {
                    painter.rect_filled(line, 0.0, theme::ACCENT_SOFT.gamma_multiply(0.6));
                } else if index % 2 == 1 {
                    painter.rect_filled(line, 0.0, Color32::from_white_alpha(3));
                }
                painter.hline(
                    line.min.x..=line.max.x,
                    line.max.y,
                    egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)),
                );
                let mut x = line.min.x;
                if let Some(project) = &editor.project {
                    let preview = icons::preview(
                        ui.ctx(),
                        &mut editor.form.thumbnails,
                        project,
                        &row.file,
                        &mut icon_budget,
                    );
                    let icon = egui::Rect::from_center_size(
                        egui::pos2(x + 11.0, line.center().y),
                        Vec2::splat(16.0),
                    );
                    icons::paint_file(
                        &painter,
                        icon,
                        preview,
                        icons::of_table(table),
                        IconTone::Normal,
                    );
                }
                x += 22.0;
                for (column_index, (column, width)) in shown.iter().zip(&widths).enumerate() {
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(x, line.min.y),
                        Vec2::new(*width, ROW_H),
                    );
                    x += width;
                    let in_range = range.contains(&(index, column_index));
                    let is_selected = selected
                        .as_ref()
                        .is_some_and(|(k, c)| *k == row.key && *c == column.id);
                    if in_range && !is_selected {
                        painter.rect_filled(cell.shrink(1.0), 2.0, theme::ACCENT_SOFT);
                    }
                    if is_selected && editing.is_none() {
                        painter.rect_stroke(
                            cell.shrink(1.0),
                            3.0,
                            egui::Stroke::new(1.5, theme::SELECTION),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let response =
                        ui.interact(cell, ui.id().with((&row.key, &column.id)), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            true,
                            is_selected,
                            format!("{}, {}", row.key, column.title),
                        )
                    });
                    if response.clicked() || response.double_clicked() {
                        hit = Some(Hit {
                            key: row.key.clone(),
                            column: column.id.clone(),
                            extend: ui.input(|i| EditorKey::ExtendSelection.held(i)),
                            double: response.double_clicked(),
                        });
                    }
                    if is_selected && editing.as_deref() == Some(column.id.as_str()) {
                        let room = editor_room(editor, table, column, cell);
                        let layer = egui::LayerId::new(
                            egui::Order::Foreground,
                            ui.id().with(("sheet-editor", &row.key, &column.id)),
                        );
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .layer_id(layer)
                                .max_rect(room.shrink2(Vec2::new(3.0, 2.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        child
                            .painter()
                            .rect_filled(room.shrink(1.0), 3.0, theme::PANEL);
                        match edit_cell(editor, &mut child, table, row, column) {
                            CellEdit::Committed(value, label) => {
                                commit = Some(CellCommit {
                                    row: row.clone(),
                                    column: column.clone(),
                                    value,
                                    label,
                                });
                            }
                            CellEdit::Cancelled | CellEdit::Done => stop_editing = true,
                            CellEdit::Open => {}
                        }
                        continue;
                    }
                    let (text, color) = match column.text(row) {
                        Some(text) => {
                            let color = match column.source {
                                ColumnSource::File | ColumnSource::Entry => theme::CODE,
                                ColumnSource::Folder => theme::MUTED,
                                _ => theme::TEXT,
                            };
                            (text, color)
                        }
                        None if column.lacks_module(row) => ("—".to_owned(), theme::FAINT),
                        None => match column.unread(row) {
                            Some(reason) => {
                                response.clone().on_hover_text(format!(
                                    "The modules of {} don't read: {reason}",
                                    row.key
                                ));
                                ("doesn't read".to_owned(), theme::WARN)
                            }
                            None => (String::new(), theme::FAINT),
                        },
                    };
                    let (align, tx) = if column.numeric {
                        (egui::Align2::RIGHT_CENTER, cell.max.x - 6.0)
                    } else {
                        (egui::Align2::LEFT_CENTER, cell.min.x + 4.0)
                    };
                    paint::text_within(
                        &painter,
                        egui::pos2(tx, cell.center().y),
                        align,
                        &text,
                        egui::FontId::monospace(12.5),
                        color,
                        *width - 10.0,
                    );
                }
            }
        });
    let below = {
        let content_bottom = output.content_size.y;
        let seen = output.state.offset.y + output.inner_rect.height();
        ((content_bottom - seen) / ROW_H).ceil().max(0.0) as usize
    };
    if below > 0 && rows.len() >= below {
        let last = &rows[rows.len() - 1];
        let second = rows.get(rows.len() - 2);
        let sort_column = view
            .sort
            .as_ref()
            .and_then(|(id, _)| shown.iter().find(|c| c.id == *id));
        let describe = |row: &SpreadsheetRow| match sort_column {
            Some(column) => format!(
                "{} with {}",
                row.key,
                column.text(row).unwrap_or_else(|| "—".to_owned())
            ),
            None => row.key.clone(),
        };
        more_note = Some(match second {
            Some(second) => format!(
                "… {below} more, to {} and {}",
                describe(second),
                describe(last)
            ),
            None => format!("… {below} more, to {}", describe(last)),
        });
    }
    if let Some(note) = more_note {
        ui.add_space(4.0);
        ui.label(theme::faint(note).size(12.0));
    }
    let hit_any = hit.is_some();
    if let Some(hit) = hit {
        let state = editor.views.spreadsheet.view_or_default(table);
        let same = state.selected.as_ref() == Some(&(hit.key.clone(), hit.column.clone()));
        if hit.extend && state.selected.is_some() {
            state.anchor = Some((hit.key.clone(), hit.column.clone()));
        } else {
            state.anchor = None;
            state.selected = Some((hit.key.clone(), hit.column.clone()));
            state.editing = None;
        }
        let editable = shown
            .iter()
            .find(|column| column.id == hit.column)
            .is_some_and(SpreadsheetColumn::editable);
        if editable && (hit.double || same) {
            state.open_cell(hit.column.clone());
        }
        let file = rows
            .iter()
            .find(|row| row.key == hit.key)
            .map_or(hit.key.clone(), |row| row.file.clone());
        if !editable {
            crate::content_tree::reveal(editor, &file);
        }
        editor.select(RowSelection {
            table: table.to_owned(),
            keys: vec![file],
            parts: vec![sheet_part(&hit.column)],
        });
    }
    let settled = commit.is_some() || stop_editing;
    if let Some(commit) = commit {
        write_cells(
            editor,
            table,
            &[(commit.row, commit.column, commit.value)],
            &commit.label,
        );
    }
    if settled {
        editor.views.spreadsheet.view_or_default(table).editing = None;
    }
    if settled || keyboard_free || editing.is_some() {
        keys(editor, ui, table, rows, shown, &range, settled);
    }
    hold_keyboard(
        editor,
        ui,
        table,
        grid,
        output.inner_rect,
        hit_any || settled,
    );
    status(editor, table, rows, shown, view);
    view_changed
}

fn selection_range(
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    selected: &Option<(String, String)>,
    anchor: &Option<(String, String)>,
) -> BTreeSet<(usize, usize)> {
    let mut range = BTreeSet::new();
    let (Some((key_a, col_a)), Some((key_b, col_b))) = (selected, anchor) else {
        return range;
    };
    let row_a = rows.iter().position(|r| r.key == *key_a);
    let row_b = rows.iter().position(|r| r.key == *key_b);
    let col_a = shown.iter().position(|c| c.id == *col_a);
    let col_b = shown.iter().position(|c| c.id == *col_b);
    let (Some(ra), Some(rb), Some(ca), Some(cb)) = (row_a, row_b, col_a, col_b) else {
        return range;
    };
    for row in ra.min(rb)..=ra.max(rb) {
        for column in ca.min(cb)..=ca.max(cb) {
            range.insert((row, column));
        }
    }
    range
}

/// Where a cell's editor draws: over the cell, and over the cells to its right and below as well
/// when it needs the room, for a value taking one of several shapes (the shape's switch beside the
/// value), a sound's tune on its slider or an action's bindings.
fn editor_room(
    editor: &Editor,
    table: &str,
    column: &SpreadsheetColumn,
    cell: egui::Rect,
) -> egui::Rect {
    let root = editor.shapes.shape_of(table);
    let size = match column.cell {
        CellKind::Tune(_) => Vec2::new(TUNE_EDITOR_W, 0.0),
        CellKind::Bindings => BINDINGS_EDITOR,
        CellKind::Form => match schema::kind(root, &column.schema) {
            SchemaKind::Union(_) | SchemaKind::Choice(_) => Vec2::new(SHAPED_EDITOR_W, 0.0),
            _ => Vec2::ZERO,
        },
    };
    egui::Rect::from_min_size(cell.min, cell.size().max(size))
}

const TUNE_EDITOR_W: f32 = 520.0;
const SHAPED_EDITOR_W: f32 = 260.0;
const BINDINGS_EDITOR: Vec2 = Vec2::new(460.0, 112.0);

enum CellEdit {
    Open,
    Committed(Value, String),
    /// The cell wrote its own edits as it went and is closed.
    Done,
    Cancelled,
}

fn edit_cell(
    editor: &mut Editor,
    ui: &mut Ui,
    table: &str,
    row: &SpreadsheetRow,
    column: &SpreadsheetColumn,
) -> CellEdit {
    if let CellKind::Bindings = column.cell {
        return bindings::cell(editor, ui, row);
    }
    if ui.input(|i| EditorKey::Cancel.pressed(i)) {
        return CellEdit::Cancelled;
    }
    if let CellKind::Tune(field) = column.cell {
        return tune_cell(editor, ui, row, field);
    }
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    let schema = column.schema.clone();
    let mut value = column
        .get(row)
        .cloned()
        .unwrap_or_else(|| match &editor.project {
            Some(project) => {
                let nearby = schema::Nearby::of(project, Some(&row.file));
                schema::template(root, &schema, nearby)
            }
            None => Value::Null,
        });
    let path = format!("{}/{}", row.pointer, column.id.replace('.', "/"));
    let mut actions = Vec::new();
    let Editor {
        project,
        shapes,
        views,
        ..
    } = editor;
    let Some(project) = project.as_ref() else {
        return CellEdit::Cancelled;
    };
    let state = views.spreadsheet.view_or_default(table);
    let autofocus = std::mem::take(&mut state.focus_editor);
    let mut cx = FormCx::new(project, root, shapes, &mut state.form, &mut actions)
        .editing(table, Some(&row.file));
    cx.autofocus = autofocus;
    let changed = ui
        .push_id(("sheet-cell", &row.key, &column.id), |ui| {
            ui.set_width(ui.available_width());
            forms::value(ui, &mut cx, &schema, &mut value, &path)
        })
        .inner;
    if !actions.is_empty() {
        for action in actions {
            crate::row_form::act(editor, action, table, &row.key);
        }
        return CellEdit::Done;
    }
    match changed.label(&row.file) {
        Some(label) => CellEdit::Committed(value, label),
        None => CellEdit::Open,
    }
}

/// A sound's volume or pitch on its slider, written as it moves; Enter closes it.
fn tune_cell(
    editor: &mut Editor,
    ui: &mut Ui,
    row: &SpreadsheetRow,
    field: &SfxTuneField,
) -> CellEdit {
    let Some(Ok(sound)) = editor.read::<SfxDef>(&row.key) else {
        return CellEdit::Cancelled;
    };
    let mut sound = (*sound).clone();
    if tune::scalar(ui, (field.of)(&mut sound), &field.range) {
        let label = format!("Edit {} of {}", field.name, file_title(&row.key));
        editor.write_content_as(ContentEdit::new(label).merging(), &row.key, &sound);
    }
    if ui.input(|i| EditorKey::Confirm.pressed(i)) {
        CellEdit::Done
    } else {
        CellEdit::Open
    }
}

fn write_cells(
    editor: &mut Editor,
    table: &str,
    cells: &[(SpreadsheetRow, SpreadsheetColumn, Value)],
    label: &str,
) {
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    for (row, column, value) in cells {
        let Some(mut whole) = files
            .get(&row.file)
            .cloned()
            .or_else(|| editor.fields_json(&row.file))
        else {
            continue;
        };
        let Some(fields) = whole.pointer_mut(&row.pointer) else {
            continue;
        };
        match &column.source {
            ColumnSource::File | ColumnSource::Folder | ColumnSource::Entry => continue,
            ColumnSource::First(path) => {
                set_path(fields, path, value.clone());
            }
            ColumnSource::Module { tag, path } => {
                let Some(reader) = editor.reader() else {
                    continue;
                };
                let key = &row.key;
                let mut modules = match reader.modules_in(table, fields) {
                    Ok(modules) => modules,
                    Err(error) => {
                        editor.toast(format!("{key}: the modules don't read: {error}"));
                        continue;
                    }
                };
                let index = match modules
                    .iter()
                    .position(|module| TypeTag::of_json(module).as_ref() == Some(tag))
                {
                    Some(index) => index,
                    None => {
                        let Some(template) = module_template(editor, table, key, tag) else {
                            continue;
                        };
                        modules.push(template);
                        modules.len() - 1
                    }
                };
                set_path(&mut modules[index], path, value.clone());
                match reader.with_modules_in(table, fields, &modules) {
                    Ok(with) => *fields = with,
                    Err(error) => {
                        editor.toast(format!("{key}: {error}"));
                        continue;
                    }
                }
            }
        }
        files.insert(row.file.clone(), whole);
    }
    let mut edit = ContentEdit::new(label);
    for (file, fields) in &files {
        edit.write_json(fields_file(file), fields);
    }
    editor.apply(edit);
}

fn module_template(editor: &Editor, table: &str, key: &str, tag: &str) -> Option<Value> {
    let root = &editor.shapes.table(table)?.shape;
    let variant = editor
        .shapes
        .module_variants(table)
        .into_iter()
        .find(|v| v.tag == tag)?;
    let nearby = schema::Nearby::of(editor.project.as_ref()?, Some(key));
    Some(schema::variant_template(root, &variant, nearby))
}

fn set_path(target: &mut Value, path: &[String], value: Value) {
    let mut current = target;
    for (index, step) in path.iter().enumerate() {
        if index + 1 == path.len() {
            if !current.is_object() {
                *current = Value::Object(serde_json::Map::new());
            }
            current[step] = value;
            return;
        }
        if !current.get(step).is_some_and(Value::is_object) {
            current[step] = Value::Object(serde_json::Map::new());
        }
        current = &mut current[step];
    }
}

/// Keeps the keyboard on the grid while a cell is picked and none is being edited, so Tab and
/// the arrows step through cells: egui otherwise hands Tab to the first focusable widget on
/// screen when nothing has focus. A modal dialog takes the keyboard back.
fn hold_keyboard(
    editor: &Editor,
    ui: &Ui,
    table: &str,
    grid: egui::Id,
    rect: egui::Rect,
    claimed: bool,
) {
    ui.interact(rect, grid, Sense::focusable_noninteractive());
    let picked = editor
        .views
        .spreadsheet
        .get(table)
        .is_some_and(|state| state.selected.is_some() && state.editing.is_none());
    ui.memory_mut(|memory| {
        if !picked || !editor.dialogs.is_empty() {
            memory.surrender_focus(grid);
            return;
        }
        if claimed || memory.has_focus(grid) || memory.focused().is_none() {
            memory.request_focus(grid);
            memory.set_focus_lock_filter(
                grid,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        }
    });
}

/// The grid's keys. In the frame a cell's edit settles only Tab is taken, so the key that
/// committed it doesn't act again on the grid.
fn keys(
    editor: &mut Editor,
    ui: &Ui,
    table: &str,
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    range: &BTreeSet<(usize, usize)>,
    settled: bool,
) {
    let state = editor.views.spreadsheet.view_or_default(table);
    let Some((key, column_id)) = state.selected.clone() else {
        return;
    };
    let row_index = rows.iter().position(|r| r.key == key);
    let column_index = shown.iter().position(|c| c.id == column_id);
    let (Some(row_index), Some(column_index)) = (row_index, column_index) else {
        return;
    };
    let editing = settled || state.editing.is_some();
    let Pressed {
        up,
        down,
        left,
        right,
        enter,
        tab,
        fill,
        escape,
        copy,
        paste,
    } = ui.input(|i| Pressed {
        up: !editing && EditorKey::Move.arrow(i) == Some([0, -1]),
        down: !editing && EditorKey::Move.arrow(i) == Some([0, 1]),
        left: !editing && EditorKey::Move.arrow(i) == Some([-1, 0]),
        right: !editing && EditorKey::Move.arrow(i) == Some([1, 0]),
        enter: !editing && EditorKey::Confirm.pressed(i),
        tab: EditorKey::NextCell.pressed(i),
        fill: EditorKey::FillDown.pressed(i),
        escape: editing && EditorKey::Cancel.pressed(i),
        copy: !editing && EditorKey::Copy.pressed(i),
        paste: (!editing).then(|| ui::keys::pasted(i)).flatten(),
    });
    let mut target = (row_index, column_index);
    if up {
        target.0 = row_index.saturating_sub(1);
    }
    if down {
        target.0 = (row_index + 1).min(rows.len().saturating_sub(1));
    }
    if left {
        target.1 = column_index.saturating_sub(1);
    }
    if right || tab {
        target.1 = (column_index + 1).min(shown.len().saturating_sub(1));
    }
    if escape {
        state.editing = None;
    }
    if tab {
        state.editing = None;
    }
    if target != (row_index, column_index) {
        state.selected = Some((rows[target.0].key.clone(), shown[target.1].id.clone()));
        state.anchor = None;
        if tab && shown[target.1].editable() {
            state.open_cell(shown[target.1].id.clone());
        }
        editor.select(RowSelection {
            table: table.to_owned(),
            keys: vec![rows[target.0].file.clone()],
            parts: vec![sheet_part(&shown[target.1].id)],
        });
        return;
    }
    if enter && shown[column_index].editable() {
        state.open_cell(column_id.clone());
        return;
    }
    if copy {
        let text = clipboard_text(rows, shown, range, &key, &column_id);
        ui.ctx().copy_text(text);
        return;
    }
    if fill {
        let column = shown[column_index].clone();
        if !column.editable() {
            return;
        }
        let Some(value) = column.get(&rows[row_index]).cloned() else {
            return;
        };
        let targets: Vec<&SpreadsheetRow> = if range.is_empty() {
            rows.get(row_index + 1).into_iter().collect()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(index, r)| range.contains(&(*index, column_index)) && r.key != key)
                .map(|(_, r)| r)
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        let cells: Vec<(SpreadsheetRow, SpreadsheetColumn, Value)> = targets
            .iter()
            .map(|&row| (row.clone(), column.clone(), value.clone()))
            .collect();
        let label = format!(
            "Fill {} down {} row{}",
            column.title,
            targets.len(),
            words::plural(targets.len())
        );
        write_cells(editor, table, &cells, &label);
        return;
    }
    if let Some(text) = paste {
        let mut cells = Vec::new();
        for (dy, line) in text.lines().enumerate() {
            let Some(row) = rows.get(row_index + dy) else {
                break;
            };
            for (dx, piece) in line.split('\t').enumerate() {
                let Some(column) = shown.get(column_index + dx) else {
                    break;
                };
                if !column.editable() {
                    continue;
                }
                let shapes = editor.shapes.clone();
                let root = shapes.shape_of(table);
                if let Some(value) = parse_cell(root, column, piece.trim()) {
                    cells.push((row.clone(), column.clone(), value));
                }
            }
        }
        if !cells.is_empty() {
            let count = cells.len();
            write_cells(
                editor,
                table,
                &cells,
                &format!("Paste {count} cell{}", words::plural(count)),
            );
        }
    }
}

struct Pressed {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    enter: bool,
    tab: bool,
    fill: bool,
    escape: bool,
    copy: bool,
    paste: Option<String>,
}

fn parse_cell(root: &Value, column: &SpreadsheetColumn, text: &str) -> Option<Value> {
    parse_as(root, &column.schema, text)
}

/// `text` as a value of `schema`: of the first of its alternatives it reads as, nothing when
/// the text is empty and nothing is one of them.
fn parse_as(root: &Value, schema: &Value, text: &str) -> Option<Value> {
    match schema::kind(root, schema) {
        SchemaKind::Integer => text.parse::<i64>().ok().map(Value::from),
        SchemaKind::Number => text.parse::<f64>().ok().map(Value::from),
        SchemaKind::Boolean => match text {
            "yes" | "true" | "1" => Some(Value::Bool(true)),
            "no" | "false" | "0" | "" => Some(Value::Bool(false)),
            _ => None,
        },
        SchemaKind::Null => text.is_empty().then_some(Value::Null),
        SchemaKind::Enum(values) => values
            .iter()
            .any(|value| value == text)
            .then(|| Value::String(text.to_owned())),
        SchemaKind::Const(value) => (value.as_str() == Some(text)).then(|| value.clone()),
        SchemaKind::Choice(alternatives) => {
            let empty_is_null = text.is_empty()
                && alternatives.iter().any(|alternative| {
                    matches!(
                        schema::kind(root, schema::resolve(root, alternative)),
                        SchemaKind::Null
                    )
                });
            if empty_is_null {
                return Some(Value::Null);
            }
            alternatives
                .iter()
                .find_map(|alternative| parse_as(root, schema::resolve(root, alternative), text))
        }
        _ => Some(Value::String(text.to_owned())),
    }
}

fn clipboard_text(
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    range: &BTreeSet<(usize, usize)>,
    key: &str,
    column_id: &str,
) -> String {
    if range.is_empty() {
        return rows
            .iter()
            .find(|r| r.key == key)
            .and_then(|row| {
                shown
                    .iter()
                    .find(|c| c.id == column_id)
                    .and_then(|c| c.text(row))
            })
            .unwrap_or_default();
    }
    let mut lines = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let cells: Vec<String> = shown
            .iter()
            .enumerate()
            .filter(|(column_index, _)| range.contains(&(row_index, *column_index)))
            .map(|(_, c)| c.text(row).unwrap_or_default())
            .collect();
        if !cells.is_empty() {
            lines.push(cells.join("\t"));
        }
    }
    lines.join("\n")
}

fn status(
    editor: &mut Editor,
    table: &str,
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    view: &SpreadsheetView,
) {
    let state = editor.views.spreadsheet.view_or_default(table);
    let mut left = Vec::new();
    if let Some((key, column_id)) = &state.selected
        && let Some(column) = shown.iter().find(|c| c.id == *column_id)
        && let Some(row) = rows.iter().find(|r| r.key == *key)
    {
        let mut cell = format!("{} · {}", row.place(), column.title);
        if let Some(text) = column.text(row) {
            cell.push_str(&format!(" · {text}"));
        }
        if let Some((last_key, last_column, change)) = &state.last_edit
            && last_key == key
            && last_column == column_id
        {
            cell = format!("{} · {} · {change}", row.place(), column.title);
        }
        left.push(cell);
    } else {
        left.push(format!("{table} · {} rows", rows.len()));
    }
    left.push(format!(
        "{} move · {} edits · {} next · {} fills down",
        EditorKey::Move.name(),
        EditorKey::Confirm.name(),
        EditorKey::NextCell.name(),
        EditorKey::FillDown.name()
    ));
    editor.status.left = left;
    if let Some((id, descending)) = &view.sort
        && let Some(column) = shown.iter().find(|c| c.id == *id)
    {
        editor.status.right.push(format!(
            "sorted by {}, {} first",
            column.title,
            if *descending { "highest" } else { "lowest" }
        ));
    }
}

pub fn inspect(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let table = selection.table.clone();
    let Some(key) = editor
        .views
        .spreadsheet
        .get(&table)
        .and_then(|state| state.selected.as_ref())
        .map(|(key, _)| key.clone())
    else {
        return;
    };
    let Some(column_id) = selection
        .parts
        .first()
        .and_then(|part| sheet_column(part))
        .map(str::to_owned)
    else {
        return;
    };
    let (rows, columns, _, _) = shown_rows(editor, &table);
    let Some(column) = columns.iter().find(|c| c.id == column_id).cloned() else {
        return;
    };
    ui.label(
        face::mono(format!("{table} · table › {}", column.title))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(6.0);
    let values: Vec<f64> = rows.iter().filter_map(|row| column.number(row)).collect();
    panel::tile(true).show(ui, |ui| {
        ui.vertical(|ui| {
            panel::title(ui, &column.title);
            form::kv_text(ui, "from", column.origin());
            if column.numeric && !values.is_empty() {
                let (low, high) = span(&values);
                let (from, to, count) = densest(&values);
                form::kv_text(
                    ui,
                    &format!("{} rows", values.len()),
                    format!("{} to {} · {count} are {} to {}", low, high, from, to),
                );
            } else {
                let texts: Vec<String> = rows.iter().filter_map(|row| column.text(row)).collect();
                let distinct: BTreeSet<&String> = texts.iter().collect();
                form::kv_text(
                    ui,
                    &format!("{} rows", texts.len()),
                    format!("{} different values", distinct.len()),
                );
            }
        });
    });
    ui.add_space(6.0);
    if column.numeric && !values.is_empty() {
        histogram(
            ui,
            &values,
            column.number(rows.iter().find(|r| r.key == key).unwrap_or(&rows[0])),
        );
    } else {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for row in &rows {
            if let Some(text) = column.text(row) {
                *counts.entry(text).or_default() += 1;
            }
        }
        let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (text, count) in counts.iter().take(8) {
            ui.horizontal(|ui| {
                ui.label(face::mono(text).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::muted(count.to_string()));
                });
            });
        }
        if counts.len() > 8 {
            ui.label(theme::faint(format!("… {} more values", counts.len() - 8)));
        }
    }
    ui.add_space(10.0);
    let Some(row) = rows.iter().find(|r| r.key == key) else {
        return;
    };
    sheet::section(ui, &row.place());
    let brief: Vec<&SpreadsheetColumn> = columns
        .iter()
        .filter(|c| c.id != column.id)
        .filter(|c| c.text(row).is_some())
        .take(6)
        .collect();
    for other in brief {
        form::kv_text(ui, &other.title, other.text(row).unwrap_or_default());
    }
    let modules: Vec<String> = row.modules.iter().filter_map(TypeTag::of_json).collect();
    if !modules.is_empty() {
        form::kv_text(ui, "modules", modules.join(" · "));
    }
    ui.add_space(6.0);
    if row.pointer.is_empty() && ui.button("→ Open as a form").clicked() {
        editor.select(RowSelection::row(&table, &key));
        editor.open_tab(EditorTab::Row {
            table: table.clone(),
            key: key.clone(),
        });
    }
}

fn span(values: &[f64]) -> (f64, f64) {
    values.iter().fold((f64::MAX, f64::MIN), |(low, high), v| {
        (low.min(*v), high.max(*v))
    })
}

fn densest(values: &[f64]) -> (f64, f64, usize) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let need = sorted.len().div_ceil(3) * 2;
    let need = need.min(sorted.len()).max(1);
    let mut best = (sorted[0], sorted[sorted.len() - 1], sorted.len());
    let mut best_width = f64::MAX;
    for start in 0..=sorted.len() - need {
        let (from, to) = (sorted[start], sorted[start + need - 1]);
        if to - from < best_width {
            best_width = to - from;
            let count = sorted.iter().filter(|v| **v >= from && **v <= to).count();
            best = (from, to, count);
        }
    }
    best
}

fn histogram(ui: &mut Ui, values: &[f64], highlight: Option<f64>) {
    let (low, high) = span(values);
    let distinct: BTreeSet<i64> = values.iter().map(|v| (*v * 100.0).round() as i64).collect();
    let integral = values.iter().all(|v| v.fract() == 0.0);
    let bins = if integral && ((high - low) as usize) < 12 {
        (high - low) as usize + 1
    } else {
        distinct.len().clamp(1, 12)
    };
    let width = ui.available_width().min(320.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 90.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let span = (high - low).max(f64::EPSILON);
    let mut counts = vec![0usize; bins];
    for v in values {
        let bin = ((((v - low) / span) * bins as f64).floor() as usize).min(bins - 1);
        counts[bin] += 1;
    }
    let max = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    let bar_w = (rect.width() / bins as f32).max(4.0);
    let bar_h = rect.height() - 18.0;
    let highlight_bin = highlight.map(|v| {
        let bin = (((v - low) / span) * bins as f64).floor() as usize;
        bin.min(bins - 1)
    });
    for (index, count) in counts.iter().enumerate() {
        let h = (*count as f32 / max) * bar_h;
        let x = rect.min.x + index as f32 * bar_w;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x + 2.0, rect.min.y + bar_h - h.max(2.0)),
            egui::pos2(x + bar_w - 2.0, rect.min.y + bar_h),
        );
        let color = if highlight_bin == Some(index) {
            theme::SELECTION
        } else {
            theme::RAISED.gamma_multiply(1.6)
        };
        painter.rect_filled(bar, 2.0, color);
        let label = if integral && bins as f64 == high - low + 1.0 {
            (low + index as f64).to_string()
        } else {
            (low + span * index as f64 / bins as f64).to_string()
        };
        if bins <= 12 {
            painter.text(
                egui::pos2(x + bar_w / 2.0, rect.max.y - 4.0),
                egui::Align2::CENTER_BOTTOM,
                label,
                egui::FontId::monospace(10.0),
                theme::FAINT,
            );
        }
    }
}

const SHEET_PART_PREFIX: &str = "#sheet/";

/// The part of a row selection that stands for the column `column` of a table's sheet.
pub(crate) fn sheet_part(column: &str) -> String {
    format!("{SHEET_PART_PREFIX}{column}")
}

/// The column a part of a row selection stands for, when it stands for one of a sheet.
pub(crate) fn sheet_column(part: &str) -> Option<&str> {
    part.strip_prefix(SHEET_PART_PREFIX)
}

/// A test of a numeric column against a value, built from a column, a comparison and a number
/// picked in turn, once added.
fn comparison_picker(ui: &mut Ui, columns: &[SpreadsheetColumn]) -> Option<SheetFilter> {
    let numeric: Vec<&SpreadsheetColumn> = columns.iter().filter(|column| column.numeric).collect();
    if numeric.is_empty() {
        return None;
    }
    let id = ui.id().with("comparison");
    let (mut column, mut test, mut value): (usize, FilterComparison, f64) = ui
        .memory(|m| m.data.get_temp(id))
        .unwrap_or((0, FilterComparison::AtLeast, 0.0));
    column = column.min(numeric.len() - 1);
    let mut added = None;
    ui.horizontal(|ui| {
        controls::combo(id.with("column"))
            .selected_text(&numeric[column].title)
            .show_ui(ui, |ui| {
                for (index, known) in numeric.iter().enumerate() {
                    ui.selectable_value(&mut column, index, &known.title);
                }
            });
        controls::combo(id.with("test"))
            .selected_text(test.sign())
            .show_ui(ui, |ui| {
                for &known in FilterComparison::VARIANTS {
                    ui.selectable_value(&mut test, known, known.sign());
                }
            });
        input::NumberField::new(&mut value).width(64.0).show(ui);
        if ui.button("add").clicked() {
            added = Some(SheetFilter::Compare {
                column: numeric[column].id.clone(),
                test,
                value,
            });
        }
    });
    ui.memory_mut(|m| m.data.insert_temp(id, (column, test, value)));
    added
}
