use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use egui::{Color32, Key, Sense, Ui, Vec2};
use engine::core::content::ContentRow;
use engine::systems::npc::{self, NpcDef};
use engine::systems::script::RiftScripts;
use serde_json::Value;

use crate::app::Editor;
use crate::forms::{self, FormCx, FormState};
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::schema::{self, SchemaKind};
use crate::tabs::EditorTab;
use crate::ui::assets::ICON_DECODES_PER_FRAME;
use crate::ui::widgets;
use crate::ui::{browser, theme};
use crate::user::SpreadsheetView;

const ROW_H: f32 = 29.0;
const KEY_W: f32 = 110.0;
const CELL_W: f32 = 84.0;
const MAX_DEFAULT_COLUMNS: usize = 9;
const CAN_BE_FOUGHT: &str = "can be fought";

#[derive(Default)]
pub struct SpreadsheetViews {
    pub views: HashMap<String, SpreadsheetState>,
}

impl SpreadsheetViews {
    pub fn state_mut(&mut self, table: &str) -> &mut SpreadsheetState {
        self.views.entry(table.to_owned()).or_default()
    }
}

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
}

impl SpreadsheetState {
    fn open_cell(&mut self, column: String) {
        self.editing = Some(column);
        self.form = FormState::default();
        self.focus_editor = true;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ColumnSource {
    Key,
    First(Vec<String>),
    Module { tag: String, path: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct SpreadsheetColumn {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub source: ColumnSource,
    pub schema: Value,
    pub numeric: bool,
}

pub struct SpreadsheetRow {
    pub key: String,
    pub value: Arc<Value>,
}

impl SpreadsheetRow {
    pub fn has_module(&self, tag: &str) -> bool {
        (*self.value)["modules"]
            .as_array()
            .is_some_and(|modules| modules.iter().any(|module| module["type"] == tag))
    }
}

impl SpreadsheetColumn {
    pub fn get<'a>(&self, row: &'a Value) -> Option<&'a Value> {
        match &self.source {
            ColumnSource::Key => None,
            ColumnSource::First(path) => dig(row, path),
            ColumnSource::Module { tag, path } => {
                let module = row["modules"]
                    .as_array()?
                    .iter()
                    .find(|module| module["type"] == *tag)?;
                dig(module, path)
            }
        }
    }

    pub fn text(&self, row: &SpreadsheetRow) -> Option<String> {
        if self.source == ColumnSource::Key {
            return Some(row.key.clone());
        }
        self.get(&row.value).map(cell_text)
    }

    pub fn number(&self, row: &SpreadsheetRow) -> Option<f64> {
        self.get(&row.value).and_then(Value::as_f64)
    }

    pub fn lacks_module(&self, row: &SpreadsheetRow) -> bool {
        match &self.source {
            ColumnSource::Module { tag, .. } => !row.has_module(tag),
            _ => false,
        }
    }

    pub fn origin(&self) -> String {
        match &self.source {
            ColumnSource::Key => "the row's key".to_owned(),
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

pub fn columns_of(editor: &Editor, table: &str, rows: &[SpreadsheetRow]) -> Vec<SpreadsheetColumn> {
    let mut columns = vec![SpreadsheetColumn {
        id: "key".to_owned(),
        title: "key".to_owned(),
        sub: String::new(),
        source: ColumnSource::Key,
        schema: Value::Null,
        numeric: false,
    }];
    let Some(shape) = editor.shapes.table(table) else {
        return columns;
    };
    let root = &shape.shape;
    let mut path = Vec::new();
    leaves(root, root, &mut path, &mut |path, schema| {
        if path.first().is_some_and(|first| first == "modules") {
            return;
        }
        let sub = match path.len() {
            1 => "row".to_owned(),
            _ => path[path.len() - 2].trim_end_matches('s').to_owned(),
        };
        columns.push(SpreadsheetColumn {
            id: path.join("."),
            title: path.last().cloned().unwrap_or_default(),
            sub,
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
                1 => short_tag(&variant.tag),
                _ => path[path.len() - 2].trim_end_matches('s').to_owned(),
            };
            columns.push(SpreadsheetColumn {
                id: format!("{}.{}", variant.tag, path.join(".")),
                title: path.last().cloned().unwrap_or_default(),
                sub,
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
            let Some(modules) = (*row.value)["modules"].as_array() else {
                continue;
            };
            for module in modules.iter().filter(|m| m["type"] == variant.tag) {
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
                sub: map_name.trim_end_matches('s').to_owned(),
                source: ColumnSource::Module {
                    tag: variant.tag.clone(),
                    path: vec![map_name, entry],
                },
                schema: serde_json::json!({ "type": if numeric { "number" } else { "string" } }),
                numeric,
            });
        }
    }
    columns
}

fn short_tag(tag: &str) -> String {
    match tag {
        "shared_script" => "shared".to_owned(),
        "inline_script" => "inline".to_owned(),
        other => other.to_owned(),
    }
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

pub fn rows_of(editor: &Editor, table: &str) -> Vec<SpreadsheetRow> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    let mut rows: Vec<SpreadsheetRow> = project
        .files
        .keys_of(table)
        .filter_map(|key| {
            Some(SpreadsheetRow {
                key: key.to_owned(),
                value: project.files.json(&row_file(table, key))?,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
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
            .filter(|column| column.source != ColumnSource::Key)
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

fn named_filters(editor: &Editor, table: &str) -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut named = BTreeMap::new();
    if table == NpcDef::TABLE {
        named.insert(CAN_BE_FOUGHT, can_be_fought(editor));
    }
    named
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
        .map(|id| id.key(&content).to_owned())
        .collect()
}

fn filters_of(view: &SpreadsheetView) -> Vec<String> {
    view.filter
        .split(';')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .collect()
}

fn passes(
    row: &SpreadsheetRow,
    filter: &str,
    named: &BTreeMap<&'static str, BTreeSet<String>>,
    columns: &[SpreadsheetColumn],
) -> bool {
    if let Some(keys) = named.get(filter) {
        return keys.contains(&row.key);
    }
    if let Some(tag) = filter.strip_prefix("has ") {
        return row.has_module(tag);
    }
    if let Some(tag) = filter.strip_prefix("lacks ") {
        return !row.has_module(tag);
    }
    for operator in [">=", "<=", "=", ">", "<"] {
        if let Some((column_id, wanted)) = filter.split_once(operator) {
            let (column_id, wanted) = (column_id.trim(), wanted.trim());
            let Some(column) = columns
                .iter()
                .find(|c| c.id == column_id || c.title == column_id)
            else {
                return true;
            };
            return match (column.number(row), wanted.parse::<f64>()) {
                (Some(have), Ok(want)) => match operator {
                    ">=" => have >= want,
                    "<=" => have <= want,
                    ">" => have > want,
                    "<" => have < want,
                    _ => (have - want).abs() < f64::EPSILON,
                },
                _ => column
                    .text(row)
                    .is_some_and(|text| operator == "=" && text.eq_ignore_ascii_case(wanted)),
            };
        }
    }
    true
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
    let all = rows_of(editor, table);
    let columns = columns_cached(editor, table, &all);
    let view = view_of(editor, table, &columns);
    let named = named_filters(editor, table);
    let filters = filters_of(&view);
    let search = editor
        .views
        .spreadsheet
        .views
        .get(table)
        .map(|state| state.search.clone())
        .unwrap_or_default();
    let total = all.len();
    let mut rows: Vec<SpreadsheetRow> = all
        .into_iter()
        .filter(|row| {
            filters
                .iter()
                .all(|filter| passes(row, filter, &named, &columns))
        })
        .filter(|row| matches_search(row, &search, &columns))
        .collect();
    if let Some((id, descending)) = &view.sort
        && let Some(column) = columns.iter().find(|c| c.id == *id)
    {
        sort_rows(&mut rows, column, *descending);
    }
    (rows, columns, view, total)
}

fn columns_cached(
    editor: &mut Editor,
    table: &str,
    rows: &[SpreadsheetRow],
) -> Arc<Vec<SpreadsheetColumn>> {
    let generation = editor
        .project
        .as_ref()
        .map_or(0, |project| project.generation);
    if let Some((built_for, columns)) = &editor.views.spreadsheet.state_mut(table).columns
        && *built_for == generation
    {
        return columns.clone();
    }
    let columns = Arc::new(columns_of(editor, table, rows));
    editor.views.spreadsheet.state_mut(table).columns = Some((generation, columns.clone()));
    columns
}

pub fn show(editor: &mut Editor, ui: &mut Ui, table: &str) {
    let (rows, columns, mut view, total) = shown_rows(editor, table);
    let shown: Vec<&SpreadsheetColumn> = view
        .columns
        .iter()
        .filter_map(|id| columns.iter().find(|c| c.id == *id))
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
    let mut filters = filters_of(view);
    let module_tags: Vec<String> = editor
        .shapes
        .module_variants(table)
        .into_iter()
        .map(|v| v.tag)
        .collect();
    ui.horizontal(|ui| {
        let state = editor.views.spreadsheet.state_mut(table);
        ui.allocate_ui(Vec2::new(220.0, 26.0), |ui| {
            widgets::search_box(ui, &mut state.search, "Filter rows", None);
        });
        let mut removed = None;
        for (index, filter) in filters.iter().enumerate() {
            let (_, closed) = widgets::chip_closable(ui, filter);
            if closed {
                removed = Some(index);
            }
        }
        if let Some(index) = removed {
            filters.remove(index);
            changed = true;
        }
        ui.menu_button("+ filter", |ui| {
            if table == NpcDef::TABLE && ui.button(CAN_BE_FOUGHT).clicked() {
                filters.push(CAN_BE_FOUGHT.to_owned());
                changed = true;
                ui.close();
            }
            for tag in &module_tags {
                if ui.button(format!("has {tag}")).clicked() {
                    filters.push(format!("has {tag}"));
                    changed = true;
                    ui.close();
                }
            }
            for tag in &module_tags {
                if ui.button(format!("lacks {tag}")).clicked() {
                    filters.push(format!("lacks {tag}"));
                    changed = true;
                    ui.close();
                }
            }
            ui.separator();
            ui.label(theme::faint("a column test, like damage > 5"));
            let id = ui.id().with("custom-filter");
            let mut text: String = ui.memory(|m| m.data.get_temp(id)).unwrap_or_default();
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(160.0)
                    .hint_text("column > value"),
            );
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(Key::Enter))
                && !text.trim().is_empty()
            {
                filters.push(text.trim().to_owned());
                text.clear();
                changed = true;
                ui.close();
            }
            ui.memory_mut(|m| m.data.insert_temp(id, text));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(theme::muted(format!("{shown} of {total} rows")));
            ui.menu_button("columns ▾", |ui| {
                ui.set_min_width(220.0);
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for column in columns.iter().filter(|c| c.source != ColumnSource::Key) {
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
        view.filter = filters.join(";");
    }
    changed
}

struct CellCommit {
    key: String,
    column: SpreadsheetColumn,
    value: Value,
    label: String,
}

struct Hit {
    key: String,
    column: String,
    shift: bool,
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
    let keyboard_free = !ui.ctx().egui_wants_keyboard_input();
    let (selected, anchor, editing) = {
        let state = editor.views.spreadsheet.state_mut(table);
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
            ColumnSource::Key => KEY_W,
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
            let painter = ui.painter();
            let sorted = view.sort.as_ref().filter(|(id, _)| *id == column.id);
            let title = match sorted {
                Some((_, true)) => format!("{} ▾", column.title.to_uppercase()),
                Some((_, false)) => format!("{} ▴", column.title.to_uppercase()),
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
            widgets::text_within(
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
    let mut icon_budget = ICON_DECODES_PER_FRAME;
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
                if let Some(image) = editor.project.as_ref().and_then(|project| {
                    browser::row_icon(
                        &ui.ctx().clone(),
                        &mut editor.form,
                        project,
                        table,
                        &row.key,
                        &mut icon_budget,
                    )
                }) {
                    let icon = egui::Rect::from_center_size(
                        egui::pos2(x + 11.0, line.center().y),
                        image.size,
                    );
                    painter.image(image.texture, icon, image.uv, Color32::WHITE);
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
                    if response.clicked() || response.double_clicked() {
                        hit = Some(Hit {
                            key: row.key.clone(),
                            column: column.id.clone(),
                            shift: ui.input(|i| i.modifiers.shift),
                            double: response.double_clicked(),
                        });
                    }
                    if is_selected && editing.as_deref() == Some(column.id.as_str()) {
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(cell.shrink2(Vec2::new(3.0, 2.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        match edit_cell(editor, &mut child, table, row, column) {
                            CellEdit::Committed(value, label) => {
                                commit = Some(CellCommit {
                                    key: row.key.clone(),
                                    column: column.clone(),
                                    value,
                                    label,
                                });
                            }
                            CellEdit::Cancelled => stop_editing = true,
                            CellEdit::Open => {}
                        }
                        continue;
                    }
                    let (text, color) = match column.text(row) {
                        Some(text) => (
                            text,
                            if column.source == ColumnSource::Key {
                                theme::CODE
                            } else {
                                theme::TEXT
                            },
                        ),
                        None if column.lacks_module(row) => ("—".to_owned(), theme::FAINT),
                        None => (String::new(), theme::FAINT),
                    };
                    let (align, tx) = if column.numeric {
                        (egui::Align2::RIGHT_CENTER, cell.max.x - 6.0)
                    } else {
                        (egui::Align2::LEFT_CENTER, cell.min.x + 4.0)
                    };
                    widgets::text_within(
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
    if let Some(hit) = hit {
        let state = editor.views.spreadsheet.state_mut(table);
        let same = state.selected.as_ref() == Some(&(hit.key.clone(), hit.column.clone()));
        if hit.shift && state.selected.is_some() {
            state.anchor = Some((hit.key.clone(), hit.column.clone()));
        } else {
            state.anchor = None;
            state.selected = Some((hit.key.clone(), hit.column.clone()));
            state.editing = None;
        }
        if hit.double || (same && hit.column != "key") {
            state.open_cell(hit.column.clone());
        }
        editor.select(RowSelection {
            table: table.to_owned(),
            keys: vec![hit.key.clone()],
            parts: vec![format!("#sheet/{}", hit.column)],
        });
    }
    if let Some(commit) = commit {
        write_cells(
            editor,
            table,
            &[(commit.key, commit.column, commit.value)],
            &commit.label,
        );
        editor.views.spreadsheet.state_mut(table).editing = None;
    }
    if stop_editing {
        editor.views.spreadsheet.state_mut(table).editing = None;
    }
    if keyboard_free || editing.is_some() {
        keys(editor, ui, table, rows, shown, &range);
    }
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

enum CellEdit {
    Open,
    Committed(Value, String),
    Cancelled,
}

fn edit_cell(
    editor: &mut Editor,
    ui: &mut Ui,
    table: &str,
    row: &SpreadsheetRow,
    column: &SpreadsheetColumn,
) -> CellEdit {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        return CellEdit::Cancelled;
    }
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    let schema = column.schema.clone();
    let mut value = column
        .get(&row.value)
        .cloned()
        .unwrap_or_else(|| schema::template(root, &schema));
    let path = format!("/{}", column.id.replace('.', "/"));
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
    let state = views.spreadsheet.state_mut(table);
    let autofocus = std::mem::take(&mut state.focus_editor);
    let mut cx = FormCx::new(project, root, shapes, &mut state.form, &mut actions)
        .owned_by(table, Some(&row.key));
    cx.autofocus = autofocus;
    let changed = ui
        .push_id(("sheet-cell", &row.key, &column.id), |ui| {
            ui.set_width(ui.available_width());
            forms::value(ui, &mut cx, &schema, &mut value, &path)
        })
        .inner;
    match changed.0 {
        Some(label) => CellEdit::Committed(value, format!("{label} of {}", row.key)),
        None => CellEdit::Open,
    }
}

fn write_cells(
    editor: &mut Editor,
    table: &str,
    cells: &[(String, SpreadsheetColumn, Value)],
    label: &str,
) {
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    for (key, column, value) in cells {
        let Some(row) = rows
            .get(key)
            .cloned()
            .or_else(|| editor.row_value(table, key))
        else {
            continue;
        };
        let mut row = row;
        match &column.source {
            ColumnSource::Key => continue,
            ColumnSource::First(path) => {
                set_path(&mut row, path, value.clone());
            }
            ColumnSource::Module { tag, path } => {
                let mut modules = row["modules"].as_array().cloned().unwrap_or_default();
                let index = match modules.iter().position(|module| module["type"] == *tag) {
                    Some(index) => index,
                    None => {
                        let Some(template) = module_template(editor, table, tag) else {
                            continue;
                        };
                        modules.push(template);
                        modules.len() - 1
                    }
                };
                set_path(&mut modules[index], path, value.clone());
                row["modules"] = Value::Array(modules);
            }
        }
        rows.insert(key.clone(), row);
    }
    let mut edit = ContentEdit::new(label);
    for (key, row) in &rows {
        edit.write_json(row_file(table, key), row);
    }
    editor.apply(edit);
}

fn module_template(editor: &Editor, table: &str, tag: &str) -> Option<Value> {
    let root = &editor.shapes.table(table)?.shape;
    let variant = editor
        .shapes
        .module_variants(table)
        .into_iter()
        .find(|v| v.tag == tag)?;
    Some(schema::variant_template(root, &variant))
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

fn keys(
    editor: &mut Editor,
    ui: &Ui,
    table: &str,
    rows: &[SpreadsheetRow],
    shown: &[SpreadsheetColumn],
    range: &BTreeSet<(usize, usize)>,
) {
    let state = editor.views.spreadsheet.state_mut(table);
    let Some((key, column_id)) = state.selected.clone() else {
        return;
    };
    let row_index = rows.iter().position(|r| r.key == key);
    let column_index = shown.iter().position(|c| c.id == column_id);
    let (Some(row_index), Some(column_index)) = (row_index, column_index) else {
        return;
    };
    let editing = state.editing.is_some();
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
        up: !editing && i.key_pressed(Key::ArrowUp),
        down: !editing && i.key_pressed(Key::ArrowDown),
        left: !editing && i.key_pressed(Key::ArrowLeft),
        right: !editing && i.key_pressed(Key::ArrowRight),
        enter: !editing && i.key_pressed(Key::Enter),
        tab: i.key_pressed(Key::Tab),
        fill: i.modifiers.command && i.key_pressed(Key::D),
        escape: editing && i.key_pressed(Key::Escape),
        copy: !editing && i.modifiers.command && i.key_pressed(Key::C),
        paste: (!editing)
            .then(|| {
                i.events.iter().find_map(|event| match event {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            })
            .flatten(),
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
        if tab && shown[target.1].source != ColumnSource::Key {
            state.open_cell(shown[target.1].id.clone());
        }
        editor.select(RowSelection {
            table: table.to_owned(),
            keys: vec![rows[target.0].key.clone()],
            parts: vec![format!("#sheet/{}", shown[target.1].id)],
        });
        return;
    }
    if enter && shown[column_index].source != ColumnSource::Key {
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
        if column.source == ColumnSource::Key {
            return;
        }
        let Some(value) = column.get(&rows[row_index].value).cloned() else {
            return;
        };
        let targets: Vec<String> = if range.is_empty() {
            rows.get(row_index + 1)
                .map(|r| vec![r.key.clone()])
                .unwrap_or_default()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(index, r)| range.contains(&(*index, column_index)) && r.key != key)
                .map(|(_, r)| r.key.clone())
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        let cells: Vec<(String, SpreadsheetColumn, Value)> = targets
            .iter()
            .map(|k| (k.clone(), column.clone(), value.clone()))
            .collect();
        let label = format!(
            "Fill {} down {} row{}",
            column.title,
            targets.len(),
            widgets::plural(targets.len())
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
                if column.source == ColumnSource::Key {
                    continue;
                }
                let shapes = editor.shapes.clone();
                let root = shapes.shape_of(table);
                if let Some(value) = parse_cell(root, column, piece.trim()) {
                    cells.push((row.key.clone(), column.clone(), value));
                }
            }
        }
        if !cells.is_empty() {
            let count = cells.len();
            write_cells(
                editor,
                table,
                &cells,
                &format!("Paste {count} cell{}", widgets::plural(count)),
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
    match schema::kind(root, &column.schema) {
        SchemaKind::Integer => text.parse::<i64>().ok().map(Value::from),
        SchemaKind::Number => text.parse::<f64>().ok().map(Value::from),
        SchemaKind::Boolean => match text {
            "yes" | "true" | "1" => Some(Value::Bool(true)),
            "no" | "false" | "0" | "" => Some(Value::Bool(false)),
            _ => None,
        },
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
    let state = editor.views.spreadsheet.state_mut(table);
    let mut left = Vec::new();
    if let Some((key, column_id)) = &state.selected
        && let Some(column) = shown.iter().find(|c| c.id == *column_id)
        && let Some(row) = rows.iter().find(|r| r.key == *key)
    {
        let mut cell = format!("{table}/{key} · {}", column.title);
        if let Some(text) = column.text(row) {
            cell.push_str(&format!(" · {text}"));
        }
        if let Some((last_key, last_column, change)) = &state.last_edit
            && last_key == key
            && last_column == column_id
        {
            cell = format!("{table}/{key} · {} · {change}", column.title);
        }
        left.push(cell);
    } else {
        left.push(format!("{table} · {} rows", rows.len()));
    }
    left.push("arrows move · Enter edits · Tab next · Ctrl D fills down".to_owned());
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
    let Some(key) = selection.keys.first().cloned() else {
        return;
    };
    let Some(column_id) = selection
        .parts
        .first()
        .and_then(|part| part.strip_prefix("#sheet/"))
        .map(str::to_owned)
    else {
        return;
    };
    let (rows, columns, _, _) = shown_rows(editor, &table);
    let Some(column) = columns.iter().find(|c| c.id == column_id).cloned() else {
        return;
    };
    ui.label(
        theme::mono(format!("{table} · table › {}", column.title))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(6.0);
    let values: Vec<f64> = rows.iter().filter_map(|row| column.number(row)).collect();
    widgets::card_frame(true).show(ui, |ui| {
        ui.vertical(|ui| {
            widgets::card_title(ui, &column.title);
            widgets::kv_text(ui, "from", column.origin());
            if column.numeric && !values.is_empty() {
                let (low, high) = span(&values);
                let (from, to, count) = densest(&values);
                widgets::kv_text(
                    ui,
                    &format!("{} rows", values.len()),
                    format!("{} to {} · {count} are {} to {}", low, high, from, to),
                );
            } else {
                let texts: Vec<String> = rows.iter().filter_map(|row| column.text(row)).collect();
                let distinct: BTreeSet<&String> = texts.iter().collect();
                widgets::kv_text(
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
                ui.label(theme::mono(text).size(12.0));
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
    widgets::section(ui, &format!("{table}/{key}"));
    if let Some(row) = rows.iter().find(|r| r.key == key) {
        let brief: Vec<&SpreadsheetColumn> = columns
            .iter()
            .filter(|c| c.source != ColumnSource::Key && c.id != column.id)
            .filter(|c| c.text(row).is_some())
            .take(6)
            .collect();
        for other in brief {
            widgets::kv_text(ui, &other.title, other.text(row).unwrap_or_default());
        }
        let modules: Vec<String> = (*row.value)["modules"]
            .as_array()
            .map(|modules| {
                modules
                    .iter()
                    .filter_map(|m| m["type"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if !modules.is_empty() {
            widgets::kv_text(ui, "modules", modules.join(" · "));
        }
    }
    ui.add_space(6.0);
    if ui.button("→ Open as a form").clicked() {
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
