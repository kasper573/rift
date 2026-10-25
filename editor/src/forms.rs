use std::collections::{HashMap, HashSet};
use std::ops::BitOrAssign;
use ui::chip;
use ui::controls::{self, Pill};
use ui::egui;
use ui::form;
use ui::menu;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::assets::AssetKind;
use engine::core::content::fixture::FixtureLock;
use engine::core::content::{
    Content, ContentFile, LoadReport, fields_file, parent_of, with_content,
};
use engine::core::script::{Assignment, Target, VariableType};
use engine::systems::area::AreaDef;
use engine::systems::script::{Condition, RiftScripting, RunnerKind};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use ui::egui::Ui;
use ui::glyph::{self, Mark};

use crate::keymap::EditorKey;
use crate::project::{Project, ProjectFiles};
use crate::schema::{self, KeptValues, SchemaKind, SchemaShapes, SchemaVariant};
use crate::shell::browser;
use crate::tabs::{file_title, shown_path};
use crate::usages::ContentUsages;
use crate::views::model::busts;
use crate::views::script::nodes::NodeCatalogue;
use crate::views::vfx;
use crate::{audio, condition, picker, spans, thumbs, value};
use ui::input::{self, InputField, Typed, drafted, typed};

/// What a hole a blueprint's owners fill is drawn in.
pub const VARIABLE: egui::Color32 = ui::ink::TYRIAN;

pub const KEPT_TIP: &str = "Kept: the engine reads this from every game, so it stays as it is";

pub struct FormCx<'a> {
    pub project: &'a Project,
    pub root: &'a Value,
    pub content: Option<Content>,
    pub catalogue: &'a NodeCatalogue,
    pub files: &'a ProjectFiles,
    pub usages: &'a ContentUsages,
    pub shapes: &'a SchemaShapes,
    pub state: &'a mut FormState,
    pub actions: &'a mut Vec<FormAction>,
    pub speaker: Option<String>,
    pub row_table: Option<String>,
    pub row_key: Option<String>,
    /// What the scripts the form names run on, when that isn't the row being edited: the object
    /// of an area's scene whose scripts the form shows.
    pub runner: Option<RunnerKind>,
    pub holes: Option<FormHoles>,
    /// What the script being edited keeps by name, when the form edits a script.
    pub kept: Option<KeptValues>,
    pub autofocus: bool,
    /// What the row being edited keeps as the engine reads it, when it is a fixture.
    pub locks: Vec<FixtureLock>,
    report: Option<&'a LoadReport>,
    load_errors: HashMap<String, String>,
}

impl<'a> FormCx<'a> {
    pub fn new(
        project: &'a Project,
        root: &'a Value,
        shapes: &'a SchemaShapes,
        state: &'a mut FormState,
        actions: &'a mut Vec<FormAction>,
    ) -> FormCx<'a> {
        FormCx {
            project,
            root,
            content: project.content.clone(),
            catalogue: &project.catalogue,
            files: &project.files,
            usages: &project.usages,
            shapes,
            state,
            actions,
            speaker: None,
            row_table: None,
            row_key: None,
            runner: None,
            holes: None,
            kept: None,
            autofocus: false,
            locks: Vec::new(),
            report: project.report.as_ref(),
            load_errors: HashMap::new(),
        }
    }

    /// The row this form edits, whose load errors it then shows beside their fields.
    pub fn editing(mut self, table: &str, key: Option<&str>) -> FormCx<'a> {
        self.row_table = Some(table.to_owned());
        self.row_key = key.map(str::to_owned);
        self.locks = key.map_or_else(Vec::new, |key| self.shapes.locks(table, key).to_vec());
        self.load_errors = match (self.report, key) {
            (Some(report), Some(key)) => crate::project::errors_in(report, &fields_file(key))
                .into_iter()
                .collect(),
            _ => HashMap::new(),
        };
        self
    }

    /// Shows the load errors of `file` beside their fields instead: a part of the bundle being
    /// edited that this form edits rather than its config.
    pub fn errors_of(mut self, file: &str) -> FormCx<'a> {
        self.load_errors = self
            .report
            .map(|report| {
                crate::project::errors_in(report, file)
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        self
    }

    /// Where a fresh value of this form is written.
    pub fn nearby(&self) -> schema::Nearby<'_> {
        schema::Nearby {
            files: self.files,
            from: self.row_key.as_deref(),
            loaded: self.content.as_ref(),
            kept: self.kept.as_ref(),
        }
    }

    /// How `path` reads from the file this form edits: how many folders up from it their paths
    /// meet, and the path from there.
    pub fn seen_from_here(&self, path: &str) -> (usize, String) {
        seen_from(self.row_key.as_deref(), path)
    }

    fn field_of_row(&self, path: &str) -> (Option<String>, String) {
        (self.row_key.clone(), path.to_owned())
    }
}

/// How `path` reads from the file `own`: how many folders up from it their paths meet, and the
/// path from there.
pub fn seen_from(own: Option<&str>, path: &str) -> (usize, String) {
    let here: Vec<&str> = own.map_or(Vec::new(), |own| {
        parent_of(own)
            .split('/')
            .filter(|s| !s.is_empty())
            .collect()
    });
    let there: Vec<&str> = path.split('/').collect();
    let shared = here.iter().zip(&there).take_while(|(a, b)| a == b).count();
    (here.len() - shared, there[shared..].join("/"))
}

#[derive(Default)]
pub struct FormHoles {
    pub fields: HashMap<String, FieldHole>,
    pub variables: Vec<(String, VariableType)>,
    pub requests: Vec<HoleRequest>,
}

#[derive(Clone)]
pub enum FieldHole {
    Filled(String),
    Open(VariableType),
}

pub enum HoleRequest {
    Make { field: String, kind: VariableType },
    Use { field: String, name: String },
    Fix { field: String, name: String },
}

#[derive(Default)]
pub struct FormState {
    pub errors: HashMap<String, String>,
    pub folded: HashSet<String>,
    pub unfolded: HashSet<String>,
    /// Optional fields set where the file leaves them out, because the value set is what the
    /// game reads when they are: they stay shown at it, so setting one is never lost. Each is held
    /// by the row it was set in, so another row's field of the same path stays unset.
    pub set_to_default: HashSet<(Option<String>, String)>,
    pub status: Vec<String>,
    pub thumbnails: thumbs::Thumbnails,
    pub prop_thumbnails: thumbs::props::PropThumbnails,
    pub audio: audio::WaveformCache,
}

pub enum FormAction {
    OpenRow {
        table: String,
        key: String,
    },
    OpenNode {
        name: String,
    },
    PickPlace {
        path: String,
    },
    NewRow {
        table: String,
        path: String,
    },
    RowAction(browser::BrowserAction),
    Reveal {
        path: String,
    },
    GiveBust,
    NewScriptModule,
    /// A script made for the object at the scene node `node` of the area being edited, and named in
    /// its scripts.
    NewObjectScript {
        node: String,
    },
}

#[derive(Default)]
pub struct FormChange(pub Option<String>);

impl FormChange {
    pub fn at(path: &str) -> FormChange {
        FormChange(Some(path.to_owned()))
    }

    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    /// The history step the change makes to the file `key`: the field it changed, by name, and
    /// an entry of a list by its place counted from one ("layers #2 › duration").
    pub fn label(&self, key: &str) -> Option<String> {
        let mut field = String::new();
        for segment in self.0.as_deref()?.split('/').filter(|s| !s.is_empty()) {
            match segment.parse::<usize>() {
                Ok(index) => field.push_str(&format!(" #{}", index + 1)),
                Err(_) if field.is_empty() => field.push_str(segment),
                Err(_) => field.push_str(&format!(" › {segment}")),
            }
        }
        Some(match field.as_str() {
            "" => format!("Edit {}", file_title(key)),
            field => format!("Edit {field} of {}", file_title(key)),
        })
    }
}

impl BitOrAssign for FormChange {
    fn bitor_assign(&mut self, other: FormChange) {
        if self.0.is_none() {
            self.0 = other.0;
        }
    }
}

const LABEL_WIDTH: f32 = 124.0;

// The column is allocated at its full width whatever the label's length, so every value beside it
// starts on the same line.
pub fn label_column(ui: &mut Ui) -> Ui {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(LABEL_WIDTH, ui.spacing().interact_size.y),
        egui::Sense::hover(),
    );
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    )
}

/// The form of `typed`, drawn from the schema of its type as the game reads it: a part of a file
/// edited as its type, with no field of it named here.
pub fn of_type<T: JsonSchema + Serialize + DeserializeOwned + 'static>(
    ui: &mut Ui,
    cx: &mut FormCx,
    typed: &mut T,
    path: &str,
) -> FormChange {
    of_type_without(ui, cx, typed, path, &[])
}

/// The form of `typed` as `of_type` draws it, without the fields at `hidden`, pointers into the
/// value that another part of the view edits.
pub fn of_type_without<T: JsonSchema + Serialize + DeserializeOwned + 'static>(
    ui: &mut Ui,
    cx: &mut FormCx,
    typed: &mut T,
    path: &str,
    hidden: &[&str],
) -> FormChange {
    let reader = cx.project.reader();
    let in_scope = || reader.scope().clone();
    let Ok(mut json) = with_content(in_scope(), || value::json(typed)).0 else {
        form::hint(ui, "does not write as the game reads it");
        return FormChange::default();
    };
    let shape = schema::of_type::<T>();
    let root = std::mem::replace(&mut cx.root, shape);
    let hidden: Vec<&str> = hidden
        .iter()
        .map(|pointer| pointer.trim_start_matches('/'))
        .collect();
    let changed = if hidden.is_empty() {
        value(ui, cx, shape, &mut json, path)
    } else {
        object_without(ui, cx, shape, &mut json, path, &hidden)
    };
    cx.root = root;
    if !changed.is_some() {
        return changed;
    }
    match with_content(in_scope(), || serde_json::from_value::<T>(json)).0 {
        Ok(read) => {
            *typed = read;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(path.to_owned(), error.to_string());
            FormChange::default()
        }
    }
}

pub fn value(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    let hole = cx
        .holes
        .as_ref()
        .and_then(|holes| holes.fields.get(path).cloned());
    let kind = match hole {
        None => return plain(ui, cx, schema, value, path),
        Some(FieldHole::Filled(name)) => {
            hole_chip(ui, cx, path, &name);
            return FormChange::default();
        }
        Some(FieldHole::Open(kind)) => kind,
    };
    let mut changed = FormChange::default();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        hole_menu(ui, cx, path, kind);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_width(ui.available_width());
            changed = plain(ui, cx, schema, value, path);
        });
    });
    changed
}

fn variable_chip(ui: &mut Ui, name: &str) -> egui::Response {
    controls::pill(ui, &format!("◇ {name}"), Pill::Tyrian).interact(egui::Sense::click())
}

fn hole_chip(ui: &mut Ui, cx: &mut FormCx, path: &str, name: &str) {
    let kind = cx
        .holes
        .as_ref()
        .and_then(|holes| holes.variables.iter().find(|(n, _)| n == name))
        .map(|(_, kind)| *kind);
    ui.horizontal(|ui| {
        let chip = variable_chip(ui, name)
            .on_hover_text("Filled by the variable at each placement; click to change");
        ui.label(theme::muted(kind.map_or("", VariableType::name)).size(12.5));
        egui::Popup::from_toggle_button_response(&chip)
            .id(ui.id().with(("hole", path)))
            .show(|ui| {
                let fitting: Vec<String> = cx
                    .holes
                    .as_ref()
                    .map(|holes| {
                        holes
                            .variables
                            .iter()
                            .filter(|(other, k)| other != name && Some(*k) == kind)
                            .map(|(other, _)| other.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for other in fitting {
                    if ui.button(format!("Use variable {other}")).clicked() {
                        request(
                            cx,
                            HoleRequest::Use {
                                field: path.to_owned(),
                                name: other,
                            },
                        );
                        ui.close();
                    }
                }
                if ui
                    .button("Use a fixed value")
                    .on_hover_text("The same for every placement")
                    .clicked()
                {
                    request(
                        cx,
                        HoleRequest::Fix {
                            field: path.to_owned(),
                            name: name.to_owned(),
                        },
                    );
                    ui.close();
                }
            });
    });
}

fn hole_menu(ui: &mut Ui, cx: &mut FormCx, path: &str, kind: VariableType) {
    let button = ui
        .add(egui::Button::new(theme::muted("◇")).frame(false))
        .on_hover_text("Make this field a variable, or fill it from one");
    egui::Popup::from_toggle_button_response(&button)
        .id(ui.id().with(("hole", path)))
        .width(260.0)
        .show(|ui| {
            ui.label(
                theme::faint(format!("{} · {}", field_name(path), kind.name()))
                    .monospace()
                    .size(11.5),
            );
            if ui
                .button("Make variable")
                .on_hover_text(
                    "A new one of this type, without a default; every placement keeps its value",
                )
                .clicked()
            {
                request(
                    cx,
                    HoleRequest::Make {
                        field: path.to_owned(),
                        kind,
                    },
                );
                ui.close();
            }
            let fitting: Vec<String> = cx
                .holes
                .as_ref()
                .map(|holes| {
                    holes
                        .variables
                        .iter()
                        .filter(|(_, k)| *k == kind)
                        .map(|(name, _)| name.clone())
                        .collect()
                })
                .unwrap_or_default();
            if fitting.is_empty() {
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.button(format!("Use variable · no {} variable yet", kind.name()));
                });
            }
            for name in fitting {
                if ui.button(format!("Use variable {name}")).clicked() {
                    request(
                        cx,
                        HoleRequest::Use {
                            field: path.to_owned(),
                            name,
                        },
                    );
                    ui.close();
                }
            }
        });
}

fn request(cx: &mut FormCx, request: HoleRequest) {
    if let Some(holes) = cx.holes.as_mut() {
        holes.requests.push(request);
    }
}

fn field_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn plain(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    let schema = schema::resolve(cx.root, schema);
    if let Some(kind) = geometry_of(schema) {
        return geometry(ui, cx, kind, value, path);
    }
    if let Some(blank) = schema.get("x-blank").and_then(Value::as_str) {
        let present = schema::non_null(cx.root, schema);
        match schema::kind(cx.root, present) {
            SchemaKind::Integer => {
                return number_field(ui, cx, present, value, path, true, Some(blank));
            }
            SchemaKind::Number => {
                return number_field(ui, cx, present, value, path, false, Some(blank));
            }
            _ => {}
        }
    }
    let schema = schema::non_null(cx.root, schema);
    if let Some(marker) = schema::marker(schema) {
        match marker {
            "x-table" => {
                return reference(ui, cx, schema, value, path);
            }
            "x-asset" => return asset(ui, cx, schema, value, path),
            "x-place" => return place(ui, cx, schema, value, path),
            "x-color" => return color(ui, value, path),
            "x-face" => return face(ui, cx, value, path),
            "x-span" => {
                return as_type(ui, cx, value, path, |ui, cx, span| {
                    spans::edit(ui, cx, span, path)
                });
            }
            "x-condition" => {
                return as_type(ui, cx, value, path, |ui, cx, typed| {
                    condition::edit(ui, cx, typed, path)
                });
            }
            "x-assignment" => {
                return as_type(ui, cx, value, path, |ui, cx, typed| {
                    assignment(ui, cx, typed, path)
                });
            }
            _ => {}
        }
    }
    match schema::kind(cx.root, schema) {
        SchemaKind::Const(constant) => {
            if value != constant {
                *value = constant.clone();
            }
            ui.label(constant.as_str().unwrap_or_default());
            FormChange::default()
        }
        SchemaKind::Enum(options) => enum_combo(ui, cx, &options, value, path),
        SchemaKind::Union(variants) => union(ui, cx, &variants, value, path),
        SchemaKind::Choice(alternatives) => choice(ui, cx, &alternatives, value, path),
        SchemaKind::Object => object(ui, cx, schema, value, path),
        SchemaKind::Array(items) => array(ui, cx, items, value, path),
        SchemaKind::Map(values) => map(ui, cx, values, schema::key_table(schema), value, path),
        SchemaKind::Text => text(ui, cx, value, path),
        SchemaKind::Integer => number(ui, cx, schema, value, path, true),
        SchemaKind::Number => number(ui, cx, schema, value, path, false),
        SchemaKind::Boolean => boolean(ui, cx, value, path),
        SchemaKind::Null => {
            ui.label("—");
            FormChange::default()
        }
        SchemaKind::Any => raw_json(ui, cx, value, path),
    }
}

pub fn object(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    object_without(ui, cx, schema, value, path, &[])
}

pub fn object_without(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
    hidden: &[&str],
) -> FormChange {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    let required = schema::required(schema);
    let mut changed = FormChange::default();
    for (name, field) in schema::properties(schema) {
        if schema::tag_of(schema).is_some_and(|(tag, _)| tag == *name)
            || hidden.contains(&name.as_str())
        {
            continue;
        }
        let field_path = format!("{path}/{name}");
        let optional = !required.contains(&name.as_str()) || schema::is_optional(field);
        if optional
            && value.get(name).is_none_or(Value::is_null)
            && cx
                .state
                .set_to_default
                .contains(&cx.field_of_row(&field_path))
        {
            let fresh = schema::template(cx.root, field, cx.nearby());
            value
                .as_object_mut()
                .expect("object")
                .insert(name.clone(), fresh);
        }
        let present = value.get(name).is_some_and(|v| !v.is_null());
        let resolved = schema::resolve(cx.root, field);
        let nested = present
            && schema::marker(resolved).is_none()
            && matches!(schema::kind(cx.root, resolved), SchemaKind::Object);
        if let Some(sibling) = field.get("x-params-of").and_then(Value::as_str) {
            changed |= params_map(
                ui,
                cx,
                ParamsMapField {
                    parent_schema: schema,
                    field,
                    name,
                    sibling,
                    path: &field_path,
                    optional,
                },
                value,
            );
            continue;
        }
        let list_of_objects = match schema::kind(cx.root, resolved) {
            SchemaKind::Array(items) => matches!(
                schema::kind(cx.root, schema::resolve(cx.root, items)),
                SchemaKind::Object | SchemaKind::Union(_)
            ),
            _ => false,
        };
        if list_of_objects {
            sheet::section(ui, name);
            let fields = value.as_object_mut().expect("object");
            let child = fields
                .entry(name.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            changed |= self::value(ui, cx, field, child, &field_path);
            if optional && child.as_array().is_some_and(Vec::is_empty) {
                fields.remove(name);
            }
            continue;
        }
        if nested {
            let unset = sheet::section_with(ui, name, |ui| {
                optional && controls::remove(ui, &format!("Unset {name}")).clicked()
            });
            if unset {
                value.as_object_mut().expect("object").remove(name);
                cx.state
                    .set_to_default
                    .remove(&cx.field_of_row(&field_path));
                changed = FormChange::at(&field_path);
            }
            if let Some(child) = value.get_mut(name) {
                changed |= object(ui, cx, field, child, &field_path);
            }
            continue;
        }
        ui.horizontal(|ui| {
            let mut label_ui = label_column(ui);
            {
                let ui = &mut label_ui;
                if optional && !present {
                    if ui
                        .add(
                            egui::Label::new(
                                theme::faint(format!("+ {name}")).monospace().size(13.0),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text(format!("Set {name}"))
                        .clicked()
                    {
                        let fresh = schema::template(cx.root, field, cx.nearby());
                        value
                            .as_object_mut()
                            .expect("object")
                            .insert(name.clone(), fresh);
                        cx.state.set_to_default.insert(cx.field_of_row(&field_path));
                        changed = FormChange::at(&field_path);
                    }
                    return;
                }
                if optional {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                            .on_hover_text(format!("Unset {name}"))
                            .clicked()
                        {
                            value.as_object_mut().expect("object").remove(name);
                            cx.state
                                .set_to_default
                                .remove(&cx.field_of_row(&field_path));
                            changed = FormChange::at(&field_path);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(theme::muted(name).monospace().size(13.0))
                                    .truncate(),
                            );
                        });
                    });
                } else {
                    ui.add(egui::Label::new(theme::muted(name).monospace().size(13.0)).truncate());
                }
            }
            if let Some(child) = value.get_mut(name).filter(|_| present) {
                let fixed = cx
                    .locks
                    .iter()
                    .any(|lock| lock.fixed(&field_path).is_some());
                ui.add_enabled_ui(!fixed, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        changed |= self::value(ui, cx, field, child, &field_path);
                    });
                })
                .response
                .on_disabled_hover_text("Read by the engine as it is");
            } else {
                ui.label(theme::faint("—"));
            }
        });
    }
    changed
}

struct ParamsMapField<'a> {
    parent_schema: &'a Value,
    field: &'a Value,
    name: &'a str,
    sibling: &'a str,
    path: &'a str,
    optional: bool,
}

/// A map keyed by the parameters another row declares, that row named by a sibling field: a
/// line per declared parameter, given or left to its default, and any key the row no longer
/// declares shown so it can be dropped.
fn params_map(ui: &mut Ui, cx: &mut FormCx, at: ParamsMapField, parent: &mut Value) -> FormChange {
    let ParamsMapField {
        parent_schema,
        field,
        name,
        sibling,
        path,
        optional,
    } = at;
    let mut changed = FormChange::default();
    sheet::section(ui, name);
    let owner = parent[sibling].as_str().unwrap_or_default().to_owned();
    let table = schema::properties(parent_schema)
        .iter()
        .find(|(field, _)| *field == sibling)
        .and_then(|(_, schema)| schema::table_of(schema::resolve(cx.root, schema)))
        .map(str::to_owned);
    let Some(table) = table.filter(|_| !owner.is_empty()) else {
        ui.label(theme::faint(format!("pick {sibling} first")).size(12.5));
        return changed;
    };
    let specs = vfx::params_of(cx.project, &table, &owner);
    let value_schema = schema::resolve(cx.root, field)
        .get("additionalProperties")
        .unwrap_or(&Value::Null);
    let typed = schema::marker(schema::resolve(cx.root, value_schema)) == Some("x-param-fill");
    let fields = parent.as_object_mut().expect("object");
    let entries = fields
        .entry(name.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entries.is_object() {
        *entries = Value::Object(Map::new());
    }
    let entries = entries.as_object_mut().expect("object");
    for spec in &specs {
        let entry_path = format!("{path}/{}", spec.name);
        if typed {
            changed |= vfx::param_fill(ui, cx, spec, entries, &entry_path);
            continue;
        }
        ui.horizontal(|ui| {
            label_column(ui).label(theme::muted(&spec.name).monospace().size(13.0));
            if let Some(entry) = entries.get_mut(&spec.name) {
                ui.vertical(|ui| {
                    ui.set_min_width(ui.available_width() - 24.0);
                    changed |= self::value(ui, cx, value_schema, entry, &entry_path);
                });
                if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                    .on_hover_text("Leave it to the vfx")
                    .clicked()
                {
                    entries.remove(&spec.name);
                    changed = FormChange::at(&entry_path);
                }
            } else if ui
                .add(
                    egui::Label::new(theme::faint("+ give").size(12.5)).sense(egui::Sense::click()),
                )
                .on_hover_text(format!(
                    "Give {}; left out, the vfx uses its own",
                    spec.name
                ))
                .clicked()
            {
                entries.insert(
                    spec.name.clone(),
                    schema::template(cx.root, value_schema, cx.nearby()),
                );
                changed = FormChange::at(&entry_path);
            }
        });
        error_line(ui, cx, &entry_path);
    }
    let stale: Vec<String> = entries
        .keys()
        .filter(|key| !specs.iter().any(|spec| spec.name == **key))
        .cloned()
        .collect();
    for key in stale {
        ui.horizontal(|ui| {
            if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                .on_hover_text("Drop it")
                .clicked()
            {
                entries.remove(&key);
                changed = FormChange::at(path);
            }
            ui.label(
                theme::warn(format!("{key} · not a parameter of {}", shown_path(&owner)))
                    .size(12.5),
            );
        });
        error_line(ui, cx, &format!("{path}/{key}"));
    }
    error_line(ui, cx, path);
    if optional && entries.is_empty() {
        fields.remove(name);
    }
    changed
}

fn union(
    ui: &mut Ui,
    cx: &mut FormCx,
    variants: &[SchemaVariant],
    value: &mut Value,
    path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    if let Some(name) = value.get("var").and_then(Value::as_str).map(str::to_owned)
        && value.as_object().is_some_and(|map| map.len() == 1)
    {
        ui.horizontal(|ui| {
            variable_chip(ui, &name);
            let kinds: Vec<String> = variants
                .iter()
                .map(|variant| schema::title(&variant.tag).to_lowercase())
                .collect();
            ui.label(theme::muted(format!("variable · {}", kinds.join(" or "))).size(12.5));
        });
        return changed;
    }
    changed |= variant_picker(ui, cx, variants, value, path);
    changed |= variant_body(ui, cx, variants, value, path);
    changed
}

fn variant_picker(
    ui: &mut Ui,
    cx: &mut FormCx,
    variants: &[SchemaVariant],
    value: &mut Value,
    path: &str,
) -> FormChange {
    let current = schema::variant_of(variants, value).map(|variant| variant.tag.clone());
    let field = variants
        .first()
        .map(|variant| variant.field.clone())
        .unwrap_or_else(|| "type".to_owned());
    let mut picked = current.clone();
    controls::combo(ui.id().with(path).with("variant"))
        .selected_text(
            current
                .as_deref()
                .map(schema::title)
                .unwrap_or_else(|| "pick…".to_owned()),
        )
        .show_ui(ui, |ui| {
            for variant in variants {
                ui.selectable_value(
                    &mut picked,
                    Some(variant.tag.clone()),
                    schema::title(&variant.tag),
                );
            }
        });
    if picked != current
        && let Some(tag) = &picked
        && let Some(variant) = variants.iter().find(|variant| variant.tag == *tag)
    {
        let mut fresh = schema::variant_template(cx.root, variant, cx.nearby());
        if let (Some(fresh), Some(old)) = (fresh.as_object_mut(), value.as_object()) {
            for (name, old_value) in old {
                if name != &field && fresh.contains_key(name) {
                    fresh.insert(name.clone(), old_value.clone());
                }
            }
        }
        *value = fresh;
        return FormChange::at(path);
    }
    FormChange::default()
}

fn variant_body(
    ui: &mut Ui,
    cx: &mut FormCx,
    variants: &[SchemaVariant],
    value: &mut Value,
    path: &str,
) -> FormChange {
    let Some(variant) = schema::variant_of(variants, value) else {
        return FormChange::default();
    };
    let schema = variant.schema;
    if let Some(kind) = geometry_of(schema) {
        geometry(ui, cx, kind, value, path)
    } else if schema::properties(schema).len() > 1 {
        object(ui, cx, schema, value, path)
    } else {
        FormChange::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FieldGeometry {
    Pos,
    Rect,
}

pub fn geometry_of(schema: &Value) -> Option<FieldGeometry> {
    if schema.get("x-pos").is_some() {
        Some(FieldGeometry::Pos)
    } else if schema.get("x-rect").is_some() {
        Some(FieldGeometry::Rect)
    } else {
        None
    }
}

fn geometry(
    ui: &mut Ui,
    cx: &mut FormCx,
    kind: FieldGeometry,
    value: &mut Value,
    path: &str,
) -> FormChange {
    let id = ui.id().with(path).with("geometry");
    let number = |name: &str| {
        value[name]
            .as_f64()
            .map(|number| number.to_string())
            .unwrap_or_else(|| "0".to_owned())
    };
    let current = match kind {
        FieldGeometry::Pos => format!("{}, {}", number("x"), number("y")),
        FieldGeometry::Rect => format!(
            "{}, {} · {} × {}",
            number("x"),
            number("y"),
            number("w"),
            number("h")
        ),
    };
    let typed = typed(ui, id, &current, |ui, draft| {
        InputField::new(draft).unit("m").show(ui)
    });
    let mut changed = FormChange::default();
    if let Typed::Cancelled = typed {
        cx.state.errors.remove(path);
    }
    if let Typed::Committed(draft) = typed {
        let numbers: Vec<f64> = draft
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect();
        let names: &[&str] = match kind {
            FieldGeometry::Pos => &["x", "y"],
            FieldGeometry::Rect => &["x", "y", "w", "h"],
        };
        if numbers.len() == names.len() {
            if !value.is_object() {
                *value = Value::Object(Map::new());
            }
            for (name, number) in names.iter().zip(numbers) {
                if value[*name].as_f64() != Some(number) {
                    value[*name] = Value::from(number);
                    changed = FormChange::at(path);
                }
            }
            cx.state.errors.remove(path);
        } else {
            cx.state.errors.insert(
                path.to_owned(),
                format!("{} numbers, like {current}", names.len()),
            );
        }
    }
    error_line(ui, cx, path);
    changed
}

fn choice(
    ui: &mut Ui,
    cx: &mut FormCx,
    alternatives: &[&Value],
    value: &mut Value,
    path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    let names: Vec<String> = alternatives
        .iter()
        .map(|alternative| describe(alternative))
        .collect();
    if let Some(name) = value.get("var").and_then(Value::as_str).map(str::to_owned)
        && value.as_object().is_some_and(|map| map.len() == 1)
    {
        let kinds: Vec<String> = names
            .iter()
            .filter(|n| n.as_str() != "Var")
            .map(|n| n.to_lowercase())
            .collect();
        let mut back = false;
        ui.horizontal(|ui| {
            let response = variable_chip(ui, &name)
                .on_hover_text("A hole each owner fills; click for a fixed value");
            if response.clicked() {
                back = true;
            }
            ui.label(theme::muted(format!("variable · {}", kinds.join(" or "))).size(12.5));
        });
        if back
            && let Some(first) = alternatives
                .iter()
                .find(|alternative| describe(alternative) != "Var")
        {
            *value = schema::template(cx.root, first, cx.nearby());
            changed = FormChange::at(path);
        }
        return changed;
    }
    let current = alternatives
        .iter()
        .position(|alternative| schema::fits(cx.root, alternative, value));
    let mut picked = current;
    if alternatives.len() > 1 {
        controls::combo(ui.id().with(path).with("choice"))
            .selected_text(current.map(|at| names[at].as_str()).unwrap_or("pick…"))
            .show_ui(ui, |ui| {
                for (at, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut picked, Some(at), name);
                }
            });
    }
    if picked != current
        && let Some(at) = picked
    {
        *value = schema::template(cx.root, alternatives[at], cx.nearby());
        changed = FormChange::at(path);
    }
    if let Some(at) = picked {
        let schema = alternatives[at];
        if schema::type_names(schema) != vec!["null"] && !is_one_value(schema) {
            changed |= self::value(ui, cx, schema, value, &format!("{path}~{at}"));
        }
    }
    changed
}

/// Whether `schema` takes a single value, which naming it among choices already picks.
fn is_one_value(schema: &Value) -> bool {
    matches!(schema::kind(schema, schema), SchemaKind::Enum(values) if values.len() == 1)
}

fn describe(schema: &Value) -> String {
    if schema::marker(schema) == Some("x-condition") {
        return "Expression".to_owned();
    }
    if let Some(marker) = schema::marker(schema) {
        return schema::title(marker.trim_start_matches("x-"));
    }
    if let SchemaKind::Enum(values) = schema::kind(schema, schema)
        && let [only] = values.as_slice()
    {
        return schema::title(only);
    }
    if let Some((_, tag)) = schema::tag_of(schema) {
        return schema::title(&tag);
    }
    if schema.get("oneOf").is_some() || schema.get("anyOf").is_some() {
        return "expression".to_owned();
    }
    let types = schema::type_names(schema);
    match types.iter().find(|name| **name != "null") {
        Some(name) => schema::title(name),
        None => "value".to_owned(),
    }
}

fn array(ui: &mut Ui, cx: &mut FormCx, items: &Value, value: &mut Value, path: &str) -> FormChange {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    let mut changed = FormChange::default();
    let item_schema = schema::resolve(cx.root, items);
    if schema::marker(item_schema) == Some("x-span") {
        return as_type(ui, cx, value, path, |ui, cx, line| {
            spans::line_field(ui, cx, line, path)
        });
    }
    if let Some(options) = item_schema.get("enum").and_then(Value::as_array) {
        let list = value.as_array_mut().expect("array");
        ui.horizontal_wrapped(|ui| {
            for option in options {
                let text = option.as_str().unwrap_or("");
                let on = list.contains(option);
                if chip::toggle(ui, text, on).clicked() {
                    if on {
                        list.retain(|held| held != option);
                    } else {
                        list.push(option.clone());
                    }
                    changed = FormChange::at(path);
                }
            }
        });
        return changed;
    }
    let inline = schema::marker(item_schema).is_none()
        && matches!(schema::kind(cx.root, item_schema), SchemaKind::Object)
        && schema::properties(item_schema).len() <= 4
        && schema::properties(item_schema).iter().all(|(_, field)| {
            !matches!(
                schema::kind(cx.root, schema::resolve(cx.root, field)),
                SchemaKind::Array(_) | SchemaKind::Object | SchemaKind::Map(_)
            )
        });
    let singular = path
        .rsplit('/')
        .next()
        .map(|name| name.strip_suffix('s').unwrap_or(name).to_owned())
        .filter(|name| !name.is_empty() && name.parse::<usize>().is_err())
        .unwrap_or_else(|| "entry".to_owned());
    let boxed = !inline
        && schema::marker(item_schema).is_none()
        && matches!(
            schema::kind(cx.root, item_schema),
            SchemaKind::Object | SchemaKind::Union(_)
        );
    let allowed: Option<Vec<String>> = cx
        .locks
        .iter()
        .find_map(|lock| lock.variants(path))
        .map(<[String]>::to_vec);
    ui.vertical(|ui| {
        let list = value.as_array_mut().expect("array");
        let mut remove = None;
        let mut swap = None;
        let count = list.len();
        for (index, item) in list.iter_mut().enumerate() {
            let item_path = format!("{path}/{index}");
            let kept = cx.locks.iter().any(|lock| lock.keeps(path, item));
            let pinned: Vec<String> = cx
                .locks
                .iter()
                .flat_map(|lock| lock.pinned(path, item))
                .map(str::to_owned)
                .collect();
            if boxed {
                panel::card(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        match schema::kind(cx.root, item_schema) {
                            SchemaKind::Union(variants) => {
                                let variants = within(variants, allowed.as_deref());
                                changed |= variant_picker(ui, cx, &variants, item, &item_path);
                            }
                            _ => {
                                ui.label(theme::strong(format!("{singular} {}", index + 1)));
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let response = if kept {
                                ui.label(theme::faint("kept").size(12.0))
                                    .on_hover_text(KEPT_TIP)
                            } else {
                                glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                                    .on_hover_text("Remove")
                            };
                            if response.clicked() && !kept {
                                remove = Some(index);
                            }
                            response.context_menu(|ui| {
                                swap = menu::move_entries(ui, index, count).or(swap);
                            });
                        });
                    });
                    ui.add_space(4.0);
                    error_line(ui, cx, &item_path);
                    changed |= match schema::kind(cx.root, item_schema) {
                        SchemaKind::Union(variants) => {
                            variant_body(ui, cx, &variants, item, &item_path)
                        }
                        _ => object(ui, cx, item_schema, item, &item_path),
                    };
                });
                continue;
            }
            ui.horizontal(|ui| {
                let response = if kept {
                    ui.label(theme::faint("·")).on_hover_text(KEPT_TIP)
                } else {
                    controls::remove(ui, "Remove")
                };
                if response.clicked() && !kept {
                    remove = Some(index);
                }
                response.context_menu(|ui| {
                    swap = menu::move_entries(ui, index, count).or(swap);
                });
                if inline {
                    if !item.is_object() {
                        *item = Value::Object(Map::new());
                    }
                    let properties = schema::properties(item_schema);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (name, field) in properties.iter().rev() {
                            let field_path = format!("{item_path}/{name}");
                            let locked = pinned.iter().any(|pin| pin == *name);
                            let present = item.get(*name).is_some_and(|v| !v.is_null());
                            let resolved = schema::resolve(cx.root, field);
                            let is_reference = schema::marker(resolved) == Some("x-table");
                            if is_reference {
                                let mut draft = item
                                    .get(*name)
                                    .cloned()
                                    .unwrap_or(Value::String(String::new()));
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        ui.available_width().max(120.0),
                                        ui.spacing().interact_size.y,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if locked {
                                            ui.disable();
                                        }
                                        let edited =
                                            self::value(ui, cx, field, &mut draft, &field_path);
                                        if edited.is_some() {
                                            item.as_object_mut()
                                                .expect("object")
                                                .insert((*name).clone(), draft);
                                            changed |= edited;
                                        }
                                    },
                                );
                            } else {
                                let mut draft = if present {
                                    item[*name].clone()
                                } else if field.get("x-blank").is_some() {
                                    Value::Null
                                } else {
                                    schema::template(cx.root, field, cx.nearby())
                                };
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        inline_width(ui, field),
                                        ui.spacing().interact_size.y,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if locked {
                                            ui.disable();
                                        }
                                        if !present {
                                            ui.style_mut().visuals.override_text_color =
                                                Some(theme::MUTED);
                                        }
                                        let edited =
                                            self::value(ui, cx, field, &mut draft, &field_path);
                                        if edited.is_some() {
                                            item.as_object_mut()
                                                .expect("object")
                                                .insert((*name).clone(), draft);
                                            changed |= edited;
                                        }
                                    },
                                )
                                .response
                                .on_hover_text(if present {
                                    name.to_string()
                                } else {
                                    format!("{name} · not set, the default")
                                });
                            }
                        }
                    });
                } else {
                    ui.vertical(|ui| {
                        changed |= self::value(ui, cx, items, item, &item_path);
                    });
                }
            });
        }
        if let Some(index) = remove {
            list.remove(index);
            changed = FormChange::at(path);
        }
        if let Some((a, b)) = swap {
            list.swap(a, b);
            changed = FormChange::at(path);
        }
        error_line(ui, cx, path);
        if ui.button(format!("+ {singular}")).clicked() {
            let first_allowed = match schema::kind(cx.root, item_schema) {
                SchemaKind::Union(variants) if allowed.is_some() => {
                    within(variants, allowed.as_deref())
                        .first()
                        .map(|variant| schema::variant_template(cx.root, variant, cx.nearby()))
                }
                _ => None,
            };
            let mut fresh =
                first_allowed.unwrap_or_else(|| schema::template(cx.root, items, cx.nearby()));
            name_newcomer(&mut fresh, list, &singular);
            list.push(fresh);
            changed = FormChange::at(path);
        }
    });
    changed
}

// A new list item with a blank name gets one of its own, so it is valid as it lands and can be
// told from the others.
fn name_newcomer(item: &mut Value, siblings: &[Value], singular: &str) {
    if item.get("name").and_then(Value::as_str) != Some("") {
        return;
    }
    let taken: HashSet<&str> = siblings
        .iter()
        .filter_map(|sibling| sibling["name"].as_str())
        .collect();
    let fresh = (1..)
        .map(|n| format!("{singular}_{n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or_default();
    item["name"] = Value::String(fresh);
}

/// A map's entries, each under its key: typed, or the file it names when `keys` names the table
/// of the files its keys name, which a picker adds and no typing renames.
fn map(
    ui: &mut Ui,
    cx: &mut FormCx,
    values: &Value,
    keys: Option<&str>,
    value: &mut Value,
    path: &str,
) -> FormChange {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    let mut changed = FormChange::default();
    ui.vertical(|ui| {
        let entries = value.as_object_mut().expect("object");
        let mut remove = None;
        let mut rename = None;
        for (key, item) in entries.iter_mut() {
            let item_path = format!("{path}/{key}");
            ui.horizontal(|ui| {
                if controls::remove(ui, "Remove").clicked() {
                    remove = Some(key.clone());
                }
                match keys {
                    Some(_) => {
                        ui.add_sized(
                            egui::vec2(110.0, 18.0),
                            egui::Label::new(theme::mono(shown_path(key)).size(13.0)).truncate(),
                        )
                        .on_hover_text(key.as_str());
                    }
                    None => {
                        let id = ui.id().with(&item_path).with("key");
                        let typed = drafted(ui, id, key, |ui, draft| {
                            InputField::new(draft).width(110.0).show(ui)
                        });
                        if let Some(draft) = typed.filter(|draft| !draft.is_empty()) {
                            rename = Some((key.clone(), draft));
                        }
                    }
                }
                ui.vertical(|ui| {
                    changed |= self::value(ui, cx, values, item, &item_path);
                });
            });
            error_line(ui, cx, &key_error_path(&item_path));
        }
        if let Some(key) = remove {
            entries.remove(&key);
            changed = FormChange::at(path);
        }
        if let Some((from, to)) = rename
            && from != to
        {
            let error_path = key_error_path(&format!("{path}/{from}"));
            if entries.contains_key(&to) {
                cx.state
                    .errors
                    .insert(error_path, format!("{to} is already a key"));
            } else if let Some(item) = entries.remove(&from) {
                cx.state.errors.remove(&error_path);
                entries.insert(to, item);
                changed = FormChange::at(path);
            }
        }
        if let Some(table) = keys {
            let mut picked = String::new();
            if crate::picker::row(ui, cx, table, &mut picked, path)
                && !picked.is_empty()
                && !entries.contains_key(&picked)
            {
                entries.insert(picked, schema::template(cx.root, values, cx.nearby()));
                changed = FormChange::at(path);
            }
            return;
        }
        ui.horizontal(|ui| {
            let id = ui.id().with(path).with("new-key");
            let mut draft = input::draft_of(ui, id).unwrap_or_default();
            let response = InputField::new(&mut draft)
                .hint("new key")
                .width(110.0)
                .show(ui);
            if response.changed() {
                input::set_draft(ui, id, &draft);
            }
            let add = ui.small_button("+ add").clicked()
                || (response.lost_focus() && ui.input(|i| EditorKey::Confirm.pressed(i)));
            if add && !draft.is_empty() && !entries.contains_key(&draft) {
                entries.insert(draft, schema::template(cx.root, values, cx.nearby()));
                input::clear_draft(ui, id);
                changed = FormChange::at(path);
            }
        });
    });
    changed
}

pub fn text(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let id = ui.id().with(path).with("text");
    let current = value.as_str().unwrap_or_default().to_owned();
    let multiline = current.len() > 60 || current.contains('\n');
    let autofocus = std::mem::take(&mut cx.autofocus);
    let typed = typed(ui, id, &current, |ui, draft| {
        let field = InputField::new(draft);
        let response = if multiline { field.rows(2) } else { field }.show(ui);
        if autofocus {
            response.request_focus();
        }
        response
    });
    let mut changed = FormChange::default();
    match typed {
        Typed::Cancelled => {
            cx.state.errors.remove(path);
        }
        Typed::Committed(draft) => {
            *value = Value::String(draft);
            changed = FormChange::at(path);
        }
        Typed::Pending => {}
    }
    error_line(ui, cx, path);
    changed
}

/// A text edited as `text` edits one.
pub fn text_of(ui: &mut Ui, cx: &mut FormCx, text: &mut String, path: &str) -> FormChange {
    let mut json = Value::String(std::mem::take(text));
    let changed = self::text(ui, cx, &mut json, path);
    if let Value::String(typed) = json {
        *text = typed;
    }
    changed
}

/// The fields of `typed`, the variant of `root` that `schema` is, as `object_without` draws them:
/// a part of a type the schema of a wider type describes.
pub fn variant_of_type<T: Serialize + DeserializeOwned>(
    ui: &mut Ui,
    cx: &mut FormCx,
    root: &'static Value,
    schema: &Value,
    typed: &mut T,
    path: &str,
    hidden: &[&str],
) -> FormChange {
    let reader = cx.project.reader();
    let Ok(mut value) = reader.to_json(typed) else {
        form::hint(ui, "does not write as the game reads it");
        return FormChange::default();
    };
    let held = std::mem::replace(&mut cx.root, root);
    let changed = object_without(ui, cx, schema, &mut value, path, hidden);
    cx.root = held;
    if !changed.is_some() {
        return changed;
    }
    match reader.from_json::<T>(&value) {
        Ok(read) => {
            *typed = read;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(path.to_owned(), error);
            FormChange::default()
        }
    }
}

/// The room a condition nested in a row of fields needs beside its name: less than this left on
/// the row, and it starts the next, so its own row of blocks wraps within the whole width.
const NESTED_CONDITION_ROOM: f32 = 220.0;

/// The fields of `typed`, the variant of `root` that `schema` is, in a row as a sentence lays them
/// out, its conditions edited by `condition`.
pub fn inline_fields_of<T: Serialize + DeserializeOwned>(
    ui: &mut Ui,
    cx: &mut FormCx,
    root: &'static Value,
    schema: &Value,
    typed: &mut T,
    path: &str,
    condition: &mut dyn FnMut(&mut Ui, &mut FormCx, &mut Condition, &str) -> FormChange,
) -> FormChange {
    let reader = cx.project.reader();
    let Ok(mut value) = reader.to_json(typed) else {
        return FormChange::default();
    };
    let held = std::mem::replace(&mut cx.root, root);
    let mut changed = FormChange::default();
    let required = schema::required(schema);
    for (name, field) in schema::properties(schema) {
        if schema::tag_of(schema).is_some_and(|(tag, _)| tag == *name) {
            continue;
        }
        let field_path = format!("{path}/{name}");
        let resolved = schema::resolve(cx.root, field);
        let optional = !required.contains(&name.as_str()) || schema::is_optional(field);
        if value.get(name).is_none_or(Value::is_null) {
            if optional {
                continue;
            }
            let fresh = schema::template(cx.root, field, cx.nearby());
            value[name] = fresh;
        }
        let child = &mut value[name];
        if schema::marker(resolved) == Some("x-condition") {
            if ui.available_size_before_wrap().x < NESTED_CONDITION_ROOM {
                ui.end_row();
            }
            ui.label(theme::muted(name).size(12.0));
            changed |= as_type(ui, cx, child, &field_path, |ui, cx, nested| {
                condition(ui, cx, nested, &field_path)
            });
            continue;
        }
        if schema::table_of(schema::non_null(cx.root, field)).is_none() {
            ui.label(theme::muted(name).size(12.0));
        }
        ui.scope(|ui| {
            ui.set_max_width(170.0);
            changed |= self::value(ui, cx, field, child, &field_path);
        });
    }
    cx.root = held;
    if !changed.is_some() {
        return changed;
    }
    match reader.from_json::<T>(&value) {
        Ok(read) => {
            *typed = read;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(path.to_owned(), error);
            FormChange::default()
        }
    }
}

/// The file `value` edited as the `Row` it reads as, by an editor of a part of it: a row that
/// doesn't read shows a hint instead, its form saying why.
pub fn of_row<Row: Serialize + DeserializeOwned>(
    ui: &mut Ui,
    cx: &mut FormCx,
    value: &mut Value,
    path: &str,
    edit: impl FnOnce(&mut Ui, &mut FormCx, &mut Row) -> FormChange,
) -> FormChange {
    let reader = cx.project.reader();
    let Ok(mut row) = reader.from_json::<Row>(value) else {
        form::hint(ui, "shows once the row reads");
        return FormChange::default();
    };
    let changed = edit(ui, cx, &mut row);
    if !changed.is_some() {
        return changed;
    }
    match reader.to_json(&row) {
        Ok(json) => {
            *value = json;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(path.to_owned(), error);
            FormChange::default()
        }
    }
}

/// `value` edited as the `T` it reads as, by an editor of `T`'s own: where the form of a schema
/// meets a type it doesn't walk. A value that doesn't read is edited as raw JSON.
pub fn as_type<T: Serialize + DeserializeOwned>(
    ui: &mut Ui,
    cx: &mut FormCx,
    value: &mut Value,
    path: &str,
    edit: impl FnOnce(&mut Ui, &mut FormCx, &mut T) -> FormChange,
) -> FormChange {
    let reader = cx.project.reader();
    let Ok(mut typed) = reader.from_json::<T>(value) else {
        return raw_json(ui, cx, value, path);
    };
    let changed = edit(ui, cx, &mut typed);
    if !changed.is_some() {
        return changed;
    }
    match reader.to_json(&typed) {
        Ok(json) => {
            *value = json;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(path.to_owned(), error);
            FormChange::default()
        }
    }
}

pub fn number(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
    integer: bool,
) -> FormChange {
    number_field(ui, cx, schema, value, path, integer, None)
}

/// An amount of `unit`, edited as `number` edits one.
pub fn quantity(
    ui: &mut Ui,
    cx: &mut FormCx,
    amount: &mut f32,
    unit: Option<&str>,
    path: &str,
) -> FormChange {
    let schema = match unit {
        Some(unit) => serde_json::json!({ "type": "number", "x-unit": unit }),
        None => serde_json::json!({ "type": "number" }),
    };
    let Ok(mut json) = value::json(amount) else {
        return FormChange::default();
    };
    let changed = number(ui, cx, &schema, &mut json, path, false);
    if let Some(typed) = json.as_f64() {
        *amount = typed as f32;
    }
    changed
}

fn number_field(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
    integer: bool,
    blank: Option<&str>,
) -> FormChange {
    let id = ui.id().with(path).with("number");
    let current = value
        .as_f64()
        .map(|number| number.to_string())
        .unwrap_or_default();
    let unit = schema["x-unit"].as_str().map(str::to_owned);
    let autofocus = std::mem::take(&mut cx.autofocus);
    let typed = typed(ui, id, &current, |ui, draft| {
        let mut field = InputField::new(draft);
        if let Some(unit) = &unit {
            field = field.unit(unit);
        }
        if let Some(blank) = blank {
            field = field.hint(blank);
        }
        let response = field.show(ui);
        if autofocus {
            response.request_focus();
        }
        response
    });
    let mut changed = FormChange::default();
    if let Typed::Cancelled = typed {
        cx.state.errors.remove(path);
    }
    if let Typed::Committed(draft) = typed {
        if blank.is_some() && draft.trim().is_empty() {
            if !value.is_null() {
                *value = Value::Null;
                cx.state.errors.remove(path);
                changed = FormChange::at(path);
            }
        } else if draft.trim() != current {
            match draft.trim().parse::<f64>() {
                Ok(parsed) => {
                    let min = schema::minimum(schema);
                    let max = schema::maximum(schema);
                    if integer && parsed.fract() != 0.0 {
                        cx.state.errors.insert(
                            path.to_owned(),
                            format!("a whole number, not {}", draft.trim()),
                        );
                    } else if min.is_some_and(|min| parsed < min)
                        || max.is_some_and(|max| parsed > max)
                    {
                        cx.state.errors.insert(
                            path.to_owned(),
                            match (min, max) {
                                (Some(min), Some(max)) => {
                                    format!("{} to {} only", min, max)
                                }
                                (Some(min), None) => format!("at least {}", min),
                                (None, Some(max)) => format!("at most {}", max),
                                (None, None) => String::new(),
                            },
                        );
                    } else {
                        cx.state.errors.remove(path);
                        *value = if integer {
                            Value::from(parsed as i64)
                        } else {
                            Value::from(parsed)
                        };
                        changed = FormChange::at(path);
                    }
                }
                Err(_) => {
                    cx.state
                        .errors
                        .insert(path.to_owned(), format!("not a number: {}", draft.trim()));
                }
            }
        }
    }
    error_line(ui, cx, path);
    changed
}

pub fn metres(value: f32) -> String {
    ((value as f64 * 1000.0).round() / 1000.0).to_string()
}

const INLINE_WIDTH: f32 = 64.0;
const INLINE_TEXT_WIDTH: f32 = 150.0;
const INPUT_AROUND_TEXT: f32 = 40.0;

fn inline_width(ui: &Ui, field: &Value) -> f32 {
    let Some(blank) = field.get("x-blank").and_then(Value::as_str) else {
        return if schema::type_names(field).contains(&"string") {
            INLINE_TEXT_WIDTH
        } else {
            INLINE_WIDTH
        };
    };
    let font = input::font(ui);
    let hint = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(blank.to_owned(), font, theme::MUTED)
            .size()
            .x
    });
    INLINE_WIDTH.max(hint + INPUT_AROUND_TEXT)
}

fn boolean(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let mut on = value.as_bool().unwrap_or(false);
    let response = ui.checkbox(&mut on, "");
    if std::mem::take(&mut cx.autofocus) {
        response.request_focus();
    }
    if response.changed() {
        *value = Value::Bool(on);
        return FormChange::at(path);
    }
    FormChange::default()
}

fn enum_combo(
    ui: &mut Ui,
    cx: &mut FormCx,
    options: &[String],
    value: &mut Value,
    path: &str,
) -> FormChange {
    let current = value.as_str().unwrap_or_default().to_owned();
    let mut picked = current.clone();
    let combo = controls::combo(ui.id().with(path).with("enum"))
        .selected_text(schema::title(&current))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(&mut picked, option.clone(), schema::title(option));
            }
        });
    if std::mem::take(&mut cx.autofocus) {
        combo.response.request_focus();
    }
    if picked != current {
        *value = Value::String(picked);
        return FormChange::at(path);
    }
    FormChange::default()
}

pub fn color(ui: &mut Ui, value: &mut Value, path: &str) -> FormChange {
    let mut rgba = value::rgba(value);
    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
        *value = Value::Array(rgba.iter().map(|byte| Value::from(*byte)).collect());
        return FormChange::at(path);
    }
    FormChange::default()
}

pub fn raw_json(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let id = ui.id().with(path).with("json");
    let current = serde_json::to_string(value).unwrap_or_default();
    let typed = typed(ui, id, &current, |ui, draft| {
        InputField::new(draft).show(ui)
    });
    match typed {
        Typed::Cancelled => {
            cx.state.errors.remove(path);
        }
        Typed::Committed(draft) => match serde_json::from_str::<Value>(&draft) {
            Ok(parsed) => {
                *value = parsed;
                return FormChange::at(path);
            }
            Err(error) => {
                cx.state.errors.insert(path.to_owned(), error.to_string());
            }
        },
        Typed::Pending => {}
    }
    error_line(ui, cx, path);
    FormChange::default()
}

pub fn reference(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    let table = schema::table_of(schema).unwrap_or_default().to_owned();
    let mut key = value.as_str().unwrap_or_default().to_owned();
    let changed = picker::row(ui, cx, &table, &mut key, path);
    if changed {
        *value = Value::String(key);
        return FormChange::at(path);
    }
    FormChange::default()
}

fn asset(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    let Some(kind) = schema["x-asset"]
        .as_str()
        .and_then(|kind| kind.parse::<AssetKind>().ok())
    else {
        return FormChange::default();
    };
    let mut file = value.as_str().unwrap_or_default().to_owned();
    if picker::asset_of(ui, cx, kind, &mut file, path) {
        *value = Value::String(file);
        return FormChange::at(path);
    }
    FormChange::default()
}

fn face(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let model = cx
        .speaker
        .as_deref()
        .and_then(|speaker| busts::model_of_npc(cx.project, speaker));
    let faces = busts::face_names(model.as_deref());
    enum_combo(ui, cx, &faces, value, path)
}

fn place(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    if !value.is_object() {
        *value = schema::template(cx.root, schema, cx.nearby());
    }
    let mut changed = FormChange::default();
    let mut area = value["area"].as_str().unwrap_or_default().to_owned();
    if picker::row(ui, cx, AreaDef::TABLE, &mut area, &format!("{path}/area")) {
        value["area"] = Value::String(area);
        if let Some(walkable) = cx.nearby().walkable(value) {
            *value = walkable;
        }
        changed = FormChange::at(path);
    }
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(28.0, ui.spacing().interact_size.y),
            egui::Label::new(theme::muted("at").monospace().size(13.0)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("pick"))
                .on_hover_text("Pick the place on its area")
                .clicked()
            {
                cx.actions.push(FormAction::PickPlace {
                    path: path.to_owned(),
                });
            }
            changed |= geometry(ui, cx, FieldGeometry::Pos, value, path);
        });
    });
    changed
}

/// How a text input over a draft of `current` ended this frame: still pending, committed to a
/// different text, or cancelled back to `current`.
pub fn error_line(ui: &mut Ui, cx: &FormCx, path: &str) {
    if let Some(error) = cx
        .state
        .errors
        .get(path)
        .or_else(|| cx.load_errors.get(path))
    {
        ui.colored_label(theme::DANGER, error);
    }
}

/// The variants a list's entries may take: those `allowed`, when a lock holds the list to some.
fn within<'v>(
    variants: Vec<SchemaVariant<'v>>,
    allowed: Option<&[String]>,
) -> Vec<SchemaVariant<'v>> {
    match allowed {
        Some(allowed) => variants
            .into_iter()
            .filter(|variant| allowed.contains(&variant.tag))
            .collect(),
        None => variants,
    }
}

/// Where the form keeps what is wrong with the key of the map entry at `item_path`, apart from
/// what is wrong with its value.
fn key_error_path(item_path: &str) -> String {
    format!("{item_path}#key")
}

/// An assignment of a set node: which value the script keeps it writes, picked among those the
/// script declares, and what it writes there.
fn assignment(
    ui: &mut Ui,
    cx: &mut FormCx,
    assignment: &mut Assignment<RiftScripting>,
    path: &str,
) -> FormChange {
    let kept = cx.kept.clone().unwrap_or_default();
    let declared = kept
        .locals
        .iter()
        .map(|(name, _)| Target::Local(name.clone()))
        .chain(
            kept.state
                .iter()
                .map(|(name, _)| Target::State(name.clone())),
        );
    let mut picked = assignment.target.clone();
    controls::combo(ui.id().with(path).with("target"))
        .selected_text(kept_name(&assignment.target))
        .show_ui(ui, |ui| {
            for target in declared {
                let name = kept_name(&target);
                ui.selectable_value(&mut picked, target, name);
            }
        });
    let mut changed = FormChange::default();
    if picked != assignment.target {
        assignment.target = picked;
        changed = FormChange::at(path);
    }
    changed |= condition::edit(ui, cx, &mut assignment.to, &format!("{path}/to"));
    changed
}

fn kept_name(target: &Target) -> String {
    match target {
        Target::Local(name) => format!("local {name}"),
        Target::State(name) => format!("state {name}"),
    }
}
