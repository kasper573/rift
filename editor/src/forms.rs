use std::collections::{HashMap, HashSet};

use egui::Ui;
use engine::core::content::{Content, LoadReport};
use engine::core::script::VariableType;
use serde_json::{Map, Value};

use crate::drafts;
use crate::project::{Project, ProjectFiles, row_file};
use crate::schema::{self, SchemaKind, SchemaShapes, SchemaVariant};
use crate::ui::assets::AssetKind;
use crate::ui::{browser, theme, widgets};
use crate::usages::ContentUsages;
use crate::views::model::busts;
use crate::views::script::nodes::NodeCatalogue;
use crate::views::vfx;
use crate::{audio, condition, picker, spans, thumbs, value};

pub struct FormCx<'a> {
    pub root: &'a Value,
    pub content: Option<Content>,
    pub catalogue: &'a NodeCatalogue,
    pub files: &'a ProjectFiles,
    pub usages: &'a ContentUsages,
    pub shapes: &'a SchemaShapes,
    pub state: &'a mut FormState,
    pub actions: &'a mut Vec<FormAction>,
    pub speaker: Option<String>,
    pub owner_table: Option<String>,
    pub owner_key: Option<String>,
    pub holes: Option<FormHoles>,
    pub autofocus: bool,
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
            root,
            content: project.content.clone(),
            catalogue: &project.catalogue,
            files: &project.files,
            usages: &project.usages,
            shapes,
            state,
            actions,
            speaker: None,
            owner_table: None,
            owner_key: None,
            holes: None,
            autofocus: false,
            report: project.report.as_ref(),
            load_errors: HashMap::new(),
        }
    }

    /// The row this form edits, whose load errors it then shows beside their fields.
    pub fn owned_by(mut self, table: &str, key: Option<&str>) -> FormCx<'a> {
        self.owner_table = Some(table.to_owned());
        self.owner_key = key.map(str::to_owned);
        self.load_errors = match (self.report, key) {
            (Some(report), Some(key)) => crate::project::errors_in(report, &row_file(table, key))
                .into_iter()
                .collect(),
            _ => HashMap::new(),
        };
        self
    }
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
    pub status: Vec<String>,
    pub thumbnails: thumbs::Thumbnails,
    pub audio: audio::WaveformCache,
}

pub enum FormAction {
    OpenRow { table: String, key: String },
    OpenScript { pointer: String },
    MakeShared { pointer: String },
    MakeInline { script: String },
    PickPlace { path: String },
    NewRow { table: String, path: String },
    RowAction(browser::BrowserAction),
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
}

impl std::ops::BitOrAssign for FormChange {
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
    egui::Frame::new()
        .fill(theme::VARIABLE_FILL)
        .stroke(egui::Stroke::new(1.0, theme::VARIABLE))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("◇ {name}"))
                    .monospace()
                    .size(12.5)
                    .color(theme::VARIABLE),
            );
        })
        .response
        .interact(egui::Sense::click())
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
    if let Some(marker) = schema::marker(schema) {
        match marker {
            "x-table" => {
                return reference(ui, cx, schema, value, path);
            }
            "x-asset" => return asset(ui, cx, schema, value, path),
            "x-place" => return place(ui, cx, schema, value, path),
            "x-color" => return color(ui, value, path),
            "x-face" => return face(ui, cx, value, path),
            "x-span" => return spans::edit(ui, cx, value, path),
            "x-condition" => return condition::edit(ui, cx, schema, value, path),
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
        SchemaKind::Enum(options) => enum_combo(ui, &options, value, path),
        SchemaKind::Union(variants) => union(ui, cx, &variants, value, path),
        SchemaKind::Choice(alternatives) => choice(ui, cx, &alternatives, value, path),
        SchemaKind::Object => object(ui, cx, schema, value, path),
        SchemaKind::Array(items) => array(ui, cx, items, value, path),
        SchemaKind::Map(values) => map(ui, cx, values, value, path),
        SchemaKind::Text => text(ui, cx, value, path),
        SchemaKind::Integer => number(ui, cx, schema, value, path, true),
        SchemaKind::Number => number(ui, cx, schema, value, path, false),
        SchemaKind::Boolean => boolean(ui, value, path),
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
        let present = value.get(name).is_some_and(|v| !v.is_null());
        let optional = !required.contains(&name.as_str()) || schema::is_optional(field);
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
            widgets::section(ui, name);
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
            ui.horizontal(|ui| {
                widgets::section(ui, name);
                if optional
                    && ui
                        .small_button("×")
                        .on_hover_text(format!("Unset {name}"))
                        .clicked()
                {
                    value.as_object_mut().expect("object").remove(name);
                    changed = FormChange::at(&field_path);
                }
            });
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
                        let fresh = schema::template(cx.root, field);
                        value
                            .as_object_mut()
                            .expect("object")
                            .insert(name.clone(), fresh);
                        changed = FormChange::at(&field_path);
                    }
                    return;
                }
                if optional {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Label::new(theme::faint("✕").size(11.0))
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(format!("Unset {name}"))
                            .clicked()
                        {
                            value.as_object_mut().expect("object").remove(name);
                            changed = FormChange::at(&field_path);
                        }
                        ui.add(
                            egui::Label::new(theme::muted(name).monospace().size(13.0)).truncate(),
                        );
                    });
                } else {
                    ui.add(egui::Label::new(theme::muted(name).monospace().size(13.0)).truncate());
                }
            }
            if let Some(child) = value.get_mut(name).filter(|_| present) {
                ui.vertical(|ui| {
                    ui.set_min_width(ui.available_width());
                    changed |= self::value(ui, cx, field, child, &field_path);
                });
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
    widgets::section(ui, name);
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
    let specs = vfx::params_of(cx.files, &table, &owner);
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
                if ui
                    .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(egui::Sense::click()))
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
                entries.insert(spec.name.clone(), schema::template(cx.root, value_schema));
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
            if ui
                .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(egui::Sense::click()))
                .on_hover_text("Drop it")
                .clicked()
            {
                entries.remove(&key);
                changed = FormChange::at(path);
            }
            ui.label(theme::warn(format!("{key} · not a parameter of {table}/{owner}")).size(12.5));
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
    widgets::combo_box(ui.id().with(path).with("variant"))
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
        let mut fresh = schema::variant_template(cx.root, variant);
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
    let mut draft = drafts::draft_or(ui, id, current.clone());
    let response = boxed_input(ui, Some("tiles"), |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut draft)
                .frame(egui::Frame::NONE)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        )
    });
    if response.changed() {
        drafts::set_draft(ui, id, &draft.clone());
    }
    let mut changed = FormChange::default();
    if committed(ui, &response) {
        drafts::clear_draft(ui, id);
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
            *value = schema::template(cx.root, first);
            changed = FormChange::at(path);
        }
        return changed;
    }
    let current = alternatives
        .iter()
        .position(|alternative| schema::fits(cx.root, alternative, value));
    let mut picked = current;
    if alternatives.len() > 1 {
        widgets::combo_box(ui.id().with(path).with("choice"))
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
        *value = schema::template(cx.root, alternatives[at]);
        changed = FormChange::at(path);
    }
    if let Some(at) = picked {
        let schema = alternatives[at];
        if schema::type_names(schema) != vec!["null"] {
            changed |= self::value(ui, cx, schema, value, &format!("{path}~{at}"));
        }
    }
    changed
}

fn describe(schema: &Value) -> String {
    if let Some(marker) = schema::marker(schema) {
        return schema::title(marker.trim_start_matches("x-"));
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
        return spans::line_field(ui, cx, value, path);
    }
    if let Some(options) = item_schema.get("enum").and_then(Value::as_array) {
        let list = value.as_array_mut().expect("array");
        ui.horizontal_wrapped(|ui| {
            for option in options {
                let text = option.as_str().unwrap_or("");
                let on = list.contains(option);
                if widgets::toggle_chip(ui, text, on).clicked() {
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
    ui.vertical(|ui| {
        let list = value.as_array_mut().expect("array");
        let mut remove = None;
        let mut swap = None;
        let count = list.len();
        for (index, item) in list.iter_mut().enumerate() {
            let item_path = format!("{path}/{index}");
            if boxed {
                widgets::card_frame(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        match schema::kind(cx.root, item_schema) {
                            SchemaKind::Union(variants) => {
                                changed |= variant_picker(ui, cx, &variants, item, &item_path);
                            }
                            _ => {
                                ui.label(theme::strong(format!("{singular} {}", index + 1)));
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let response = ui
                                .add(
                                    egui::Label::new(theme::faint("✕").size(11.0))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Remove");
                            if response.clicked() {
                                remove = Some(index);
                            }
                            response.context_menu(|ui| {
                                if index > 0 && ui.button("Move up").clicked() {
                                    swap = Some((index, index - 1));
                                    ui.close();
                                }
                                if index + 1 < count && ui.button("Move down").clicked() {
                                    swap = Some((index, index + 1));
                                    ui.close();
                                }
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
                let response = ui
                    .add(egui::Label::new(theme::faint("×")).sense(egui::Sense::click()))
                    .on_hover_text("Remove");
                if response.clicked() {
                    remove = Some(index);
                }
                response.context_menu(|ui| {
                    if index > 0 && ui.button("Move up").clicked() {
                        swap = Some((index, index - 1));
                        ui.close();
                    }
                    if index + 1 < count && ui.button("Move down").clicked() {
                        swap = Some((index, index + 1));
                        ui.close();
                    }
                });
                if inline {
                    if !item.is_object() {
                        *item = Value::Object(Map::new());
                    }
                    let properties = schema::properties(item_schema);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (name, field) in properties.iter().rev() {
                            let field_path = format!("{item_path}/{name}");
                            let present = item.get(*name).is_some_and(|v| !v.is_null());
                            let resolved = schema::resolve(cx.root, field);
                            let is_reference = schema::marker(resolved) == Some("x-table");
                            if is_reference {
                                let child = item
                                    .as_object_mut()
                                    .expect("object")
                                    .entry((*name).clone())
                                    .or_insert(Value::String(String::new()));
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        ui.available_width().max(120.0),
                                        ui.spacing().interact_size.y,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        changed |= self::value(ui, cx, field, child, &field_path);
                                    },
                                );
                            } else {
                                let mut draft = if present {
                                    item[*name].clone()
                                } else if field.get("x-blank").is_some() {
                                    Value::Null
                                } else {
                                    schema::template(cx.root, field)
                                };
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        inline_width(ui, field),
                                        ui.spacing().interact_size.y,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
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
            let mut fresh = schema::template(cx.root, items);
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

fn map(ui: &mut Ui, cx: &mut FormCx, values: &Value, value: &mut Value, path: &str) -> FormChange {
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
                if ui.small_button("×").clicked() {
                    remove = Some(key.clone());
                }
                let id = ui.id().with(&item_path).with("key");
                let mut draft = drafts::draft_or(ui, id, key.clone());
                let response = ui.add(egui::TextEdit::singleline(&mut draft).desired_width(110.0));
                if response.changed() {
                    drafts::set_draft(ui, id, &draft.clone());
                }
                if committed(ui, &response) {
                    drafts::clear_draft(ui, id);
                    if draft != *key && !draft.is_empty() {
                        rename = Some((key.clone(), draft));
                    }
                }
                ui.vertical(|ui| {
                    changed |= self::value(ui, cx, values, item, &item_path);
                });
            });
        }
        if let Some(key) = remove {
            entries.remove(&key);
            changed = FormChange::at(path);
        }
        if let Some((from, to)) = rename
            && let Some(item) = entries.remove(&from)
        {
            entries.insert(to, item);
            changed = FormChange::at(path);
        }
        ui.horizontal(|ui| {
            let id = ui.id().with(path).with("new-key");
            let mut draft = drafts::draft_of(ui, id).unwrap_or_default();
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .hint_text("new key")
                    .desired_width(110.0),
            );
            if response.changed() {
                drafts::set_draft(ui, id, &draft.clone());
            }
            let add = ui.small_button("+ add").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if add && !draft.is_empty() && !entries.contains_key(&draft) {
                entries.insert(draft, schema::template(cx.root, values));
                drafts::clear_draft(ui, id);
                changed = FormChange::at(path);
            }
        });
    });
    changed
}

pub fn text(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let id = ui.id().with(path).with("text");
    let current = value.as_str().unwrap_or_default().to_owned();
    let mut draft = drafts::draft_or(ui, id, current.clone());
    let multiline = current.len() > 60 || current.contains('\n');
    let response = boxed_input(ui, None, |ui| {
        if multiline {
            ui.add(
                egui::TextEdit::multiline(&mut draft)
                    .frame(egui::Frame::NONE)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
        } else {
            ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            )
        }
    });
    if response.changed() {
        drafts::set_draft(ui, id, &draft.clone());
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && response.has_focus() {
        drafts::clear_draft(ui, id);
        cx.state.errors.remove(path);
        return FormChange::default();
    }
    let mut changed = FormChange::default();
    if committed(ui, &response) {
        drafts::clear_draft(ui, id);
        if draft != current {
            *value = Value::String(draft);
            changed = FormChange::at(path);
        }
    }
    error_line(ui, cx, path);
    changed
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
    let mut draft = drafts::draft_or(ui, id, current.clone());
    let unit = schema["x-unit"].as_str().map(str::to_owned);
    let response = boxed_input(ui, unit.as_deref(), |ui| {
        let mut edit = egui::TextEdit::singleline(&mut draft)
            .frame(egui::Frame::NONE)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY);
        if let Some(blank) = blank {
            edit = edit.hint_text(theme::muted(blank));
        }
        ui.add(edit)
    });
    if std::mem::take(&mut cx.autofocus) {
        response.request_focus();
    }
    if response.changed() {
        drafts::set_draft(ui, id, &draft.clone());
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && response.has_focus() {
        drafts::clear_draft(ui, id);
        cx.state.errors.remove(path);
        return FormChange::default();
    }
    let mut changed = FormChange::default();
    if committed(ui, &response) {
        drafts::clear_draft(ui, id);
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
                    if min.is_some_and(|min| parsed < min) || max.is_some_and(|max| parsed > max) {
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
                            Value::from(parsed.round() as i64)
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

pub fn tiles(value: f32) -> String {
    ((value as f64 * 1000.0).round() / 1000.0).to_string()
}

const INLINE_WIDTH: f32 = 64.0;
const INLINE_TEXT_WIDTH: f32 = 150.0;
const BOXED_INPUT_AROUND_TEXT: f32 = 40.0;

fn inline_width(ui: &Ui, field: &Value) -> f32 {
    let Some(blank) = field.get("x-blank").and_then(Value::as_str) else {
        return if schema::type_names(field).contains(&"string") {
            INLINE_TEXT_WIDTH
        } else {
            INLINE_WIDTH
        };
    };
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let hint = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(blank.to_owned(), font, theme::MUTED)
            .size()
            .x
    });
    INLINE_WIDTH.max(hint + BOXED_INPUT_AROUND_TEXT)
}

pub fn boxed_input(
    ui: &mut Ui,
    unit: Option<&str>,
    input: impl FnOnce(&mut Ui) -> egui::Response,
) -> egui::Response {
    let focused_id = ui.ctx().memory(|m| m.focused());
    let frame = egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 3));
    let mut inner = None;
    let outer = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(unit) = unit {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::faint(unit).size(12.0));
                    inner = Some(input(ui));
                });
            } else {
                inner = Some(input(ui));
            }
        });
    });
    let response = inner.expect("the input was drawn");
    if focused_id == Some(response.id) || response.has_focus() {
        ui.painter().rect_stroke(
            outer.response.rect,
            4.0,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn boolean(ui: &mut Ui, value: &mut Value, path: &str) -> FormChange {
    let mut on = value.as_bool().unwrap_or(false);
    if ui.checkbox(&mut on, "").changed() {
        *value = Value::Bool(on);
        return FormChange::at(path);
    }
    FormChange::default()
}

fn enum_combo(ui: &mut Ui, options: &[String], value: &mut Value, path: &str) -> FormChange {
    let current = value.as_str().unwrap_or_default().to_owned();
    let mut picked = current.clone();
    widgets::combo_box(ui.id().with(path).with("enum"))
        .selected_text(schema::title(&current))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(&mut picked, option.clone(), schema::title(option));
            }
        });
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
    let mut draft = drafts::draft_or(ui, id, current.clone());
    let response = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace),
    );
    if response.changed() {
        drafts::set_draft(ui, id, &draft.clone());
    }
    if committed(ui, &response) {
        drafts::clear_draft(ui, id);
        if draft != current {
            match serde_json::from_str::<Value>(&draft) {
                Ok(parsed) => {
                    *value = parsed;
                    return FormChange::at(path);
                }
                Err(error) => {
                    cx.state.errors.insert(path.to_owned(), error.to_string());
                }
            }
        }
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
    let kinds: Vec<AssetKind> = schema["x-asset"]
        .as_str()
        .and_then(AssetKind::named)
        .map_or_else(|| AssetKind::ALL.to_vec(), |kind| vec![kind]);
    let mut file = value.as_str().unwrap_or_default().to_owned();
    if picker::asset_of(ui, cx, &kinds, &mut file, path) {
        *value = Value::String(file);
        return FormChange::at(path);
    }
    FormChange::default()
}

fn face(ui: &mut Ui, cx: &mut FormCx, value: &mut Value, path: &str) -> FormChange {
    let mut faces: Vec<String> = busts::generic().map(str::to_owned).collect();
    if let Some(speaker) = &cx.speaker
        && let Some(npc) = cx.files.json(&row_file("npc", speaker))
        && let Some(model) = npc["model"].as_str()
        && let Some(model) = cx.files.json(&row_file("model", model))
    {
        for module in model["modules"].as_array().into_iter().flatten() {
            if module["type"] == "bust" {
                for own in module["own"].as_array().into_iter().flatten() {
                    if let Some(name) = own["name"].as_str() {
                        faces.push(name.to_owned());
                    }
                }
            }
        }
    }
    enum_combo(ui, &faces, value, path)
}

fn place(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    value: &mut Value,
    path: &str,
) -> FormChange {
    if !value.is_object() {
        *value = schema::template(cx.root, schema);
    }
    let mut changed = FormChange::default();
    let mut area = value["area"].as_str().unwrap_or_default().to_owned();
    if picker::row(ui, cx, "area", &mut area, &format!("{path}/area")) {
        value["area"] = Value::String(area);
        changed = FormChange::at(path);
    }
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(28.0, ui.spacing().interact_size.y),
            egui::Label::new(theme::muted("at").monospace().size(13.0)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("⌖ pick")
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

pub fn drafted(
    ui: &mut Ui,
    id: egui::Id,
    current: &str,
    input: impl FnOnce(&mut Ui, &mut String) -> egui::Response,
) -> Option<String> {
    let mut draft = drafts::draft_or(ui, id, current.to_owned());
    let response = input(ui, &mut draft);
    if committed(ui, &response) {
        drafts::clear_draft(ui, id);
        return (draft != current).then_some(draft);
    }
    if response.has_focus() {
        drafts::set_draft(ui, id, &draft);
    } else {
        drafts::clear_draft(ui, id);
    }
    None
}

pub fn committed(ui: &Ui, response: &egui::Response) -> bool {
    response.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape))
}

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
