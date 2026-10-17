//! The parameters a vfx or shader row declares, and the one set of controls that fills them: the
//! preview's knobs, a shader layer's fills, and a script's values for a vfx.

use egui::{Color32, Ui};
use engine::core::math::Rgba;
use engine::core::vfx::{ParamDecl, ParamFill, ParamKind, ParamValue, ShaderParam};
use serde_json::{Map, Value};

use super::{SHADER_TABLE, TABLE};
use crate::forms::{self, FormChange, FormCx};
use crate::project::{ProjectFiles, row_file};
use crate::ui::{theme, widgets};

const PARAM_INDENT: f32 = 22.0;
const BOUND_WIDTH: f32 = 64.0;
const SOURCE_WIDTH: f32 = 110.0;
const VALUE_AND_RESET: f32 = 110.0;
const MIN_SLIDER: f32 = 60.0;

/// A parameter as a row declares it: the kind of value that fills it, the value when nothing
/// does, and for a ranged number, the slider's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    pub default: ParamValue,
    pub range: Option<(f32, f32)>,
}

impl From<&ShaderParam> for ParamSpec {
    fn from(param: &ShaderParam) -> ParamSpec {
        ParamSpec {
            name: param.name.clone(),
            kind: param.kind,
            default: param.default,
            range: param.range(),
        }
    }
}

impl From<&ParamDecl> for ParamSpec {
    fn from(param: &ParamDecl) -> ParamSpec {
        ParamSpec {
            name: param.name.clone(),
            kind: param.kind,
            default: match param.kind {
                ParamKind::Bool => ParamValue::Bool(false),
                ParamKind::Number => ParamValue::Number(1.0),
                ParamKind::Color => ParamValue::Color(Rgba::WHITE),
            },
            range: None,
        }
    }
}

/// The parameters the row `table/key` declares, each as far as it parses, so a row being worked
/// on still shows the ones that are whole.
pub fn params_of(files: &ProjectFiles, table: &str, key: &str) -> Vec<ParamSpec> {
    let Some(row) = files.json(&row_file(table, key)) else {
        return Vec::new();
    };
    let declared = row["params"].as_array().into_iter().flatten();
    match table {
        SHADER_TABLE => declared
            .filter_map(|param| serde_json::from_value::<ShaderParam>(param.clone()).ok())
            .map(|param| ParamSpec::from(&param))
            .collect(),
        TABLE => declared
            .filter_map(|param| serde_json::from_value::<ParamDecl>(param.clone()).ok())
            .map(|param| ParamSpec::from(&param))
            .collect(),
        _ => Vec::new(),
    }
}

/// One knob for a parameter's value, by its kind: a checkbox, a slider within the range or a
/// drag without one, or a color button. Says whether it moved.
pub fn param_control(ui: &mut Ui, spec: &ParamSpec, value: &mut ParamValue) -> bool {
    match spec.kind {
        ParamKind::Bool => {
            let mut flag = matches!(value, ParamValue::Bool(true));
            if ui.checkbox(&mut flag, "").changed() {
                *value = ParamValue::Bool(flag);
                return true;
            }
        }
        ParamKind::Number => {
            let mut amount = match value {
                ParamValue::Number(number) => *number,
                _ => 1.0,
            };
            let moved = match spec.range {
                Some((min, max)) => {
                    design_tokens::widgets::slider(ui, egui::Slider::new(&mut amount, min..=max))
                        .changed()
                }
                None => ui
                    .add(egui::DragValue::new(&mut amount).speed(0.01))
                    .changed(),
            };
            if moved {
                *value = ParamValue::Number(tidy(f64::from(amount)) as f32);
                return true;
            }
        }
        ParamKind::Color => {
            let held = match value {
                ParamValue::Color(color) => *color,
                _ => Rgba::WHITE,
            };
            let [r, g, b, a] = held.0.to_be_bytes();
            let mut color = Color32::from_rgba_unmultiplied(r, g, b, a);
            if ui.color_edit_button_srgba(&mut color).changed() {
                *value = ParamValue::Color(Rgba::from(color.to_srgba_unmultiplied()));
                return true;
            }
        }
    }
    false
}

/// A shader layer's fill for one of its shader's parameters: a value of its own, or one of the
/// owning vfx row's parameters of the same kind to take it from. A parameter left at its
/// default writes nothing.
pub fn param_fill(
    ui: &mut Ui,
    cx: &mut FormCx,
    spec: &ParamSpec,
    fills: &mut Map<String, Value>,
    path: &str,
) -> FormChange {
    let fill = fills
        .get(&spec.name)
        .and_then(|fill| serde_json::from_value::<ParamFill>(fill.clone()).ok());
    let bindable: Vec<String> = match (&cx.owner_table, &cx.owner_key) {
        (Some(table), Some(key)) if table == TABLE => params_of(cx.files, TABLE, key)
            .into_iter()
            .filter(|own| own.kind == spec.kind)
            .map(|own| own.name)
            .collect(),
        _ => Vec::new(),
    };
    let mut changed = FormChange::default();
    let mut set = |fills: &mut Map<String, Value>, next: Option<ParamFill>| {
        match next {
            Some(next) => fills.insert(
                spec.name.clone(),
                serde_json::to_value(next).unwrap_or(Value::Null),
            ),
            None => fills.remove(&spec.name),
        };
        changed = FormChange::at(path);
    };
    ui.horizontal(|ui| {
        forms::label_column(ui).label(theme::muted(&spec.name).monospace().size(13.0));
        let beside = if bindable.is_empty() {
            VALUE_AND_RESET
        } else {
            VALUE_AND_RESET + SOURCE_WIDTH
        };
        ui.spacing_mut().slider_width = (ui.available_width() - beside).max(MIN_SLIDER);
        match &fill {
            Some(ParamFill::Param { param }) => {
                widgets::chip(ui, &format!("◇ {param}"));
            }
            other => {
                let mut value = match other {
                    Some(ParamFill::Value(value)) => *value,
                    _ => spec.default,
                };
                if other.is_none() {
                    ui.style_mut().visuals.override_text_color = Some(theme::MUTED);
                }
                if param_control(ui, spec, &mut value) {
                    set(fills, Some(ParamFill::Value(value)));
                }
            }
        }
        if !bindable.is_empty() {
            let source = match &fill {
                Some(ParamFill::Param { param }) => format!("◇ {param}"),
                _ => "value".to_owned(),
            };
            widgets::combo_box(ui.id().with(path).with("source"))
                .width(SOURCE_WIDTH)
                .selected_text(theme::mono(source).size(12.5))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, "value").clicked() {
                        set(fills, Some(ParamFill::Value(spec.default)));
                    }
                    for param in &bindable {
                        if ui.selectable_label(false, format!("◇ {param}")).clicked() {
                            set(
                                fills,
                                Some(ParamFill::Param {
                                    param: param.clone(),
                                }),
                            );
                        }
                    }
                });
        }
        if fill.is_some()
            && ui
                .add(egui::Label::new(theme::faint("↺").size(12.0)).sense(egui::Sense::click()))
                .on_hover_text("Back to the shader's default")
                .clicked()
        {
            set(fills, None);
        }
    });
    forms::error_line(ui, cx, path);
    changed
}

/// The parameters a shader row declares: each a name, a kind, a default, and for a number its
/// range.
pub fn shader_params(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let mut changed = FormChange::default();
    if !value["params"].is_array() {
        value["params"] = Value::Array(Vec::new());
    }
    let count = value["params"].as_array().map_or(0, Vec::len);
    widgets::section_count(ui, "params", count);
    let mut remove = None;
    for index in 0..count {
        let path = format!("/params/{index}");
        let param = &mut value["params"][index];
        let kind = param["kind"].as_str().unwrap_or("number").to_owned();
        ui.horizontal(|ui| {
            if ui
                .add(egui::Label::new(theme::faint("×")).sense(egui::Sense::click()))
                .on_hover_text("Remove")
                .clicked()
            {
                remove = Some(index);
            }
            ui.scope(|ui| {
                ui.set_width(120.0);
                changed |= forms::text(ui, cx, &mut param["name"], &format!("{path}/name"));
            });
            let mut picked = kind.clone();
            widgets::combo_box(("param-kind", index))
                .width(90.0)
                .selected_text(theme::mono(&kind).size(13.0))
                .show_ui(ui, |ui| {
                    for kind in ["number", "bool", "color"] {
                        ui.selectable_value(&mut picked, kind.to_owned(), kind);
                    }
                });
            if picked != kind {
                param["kind"] = Value::String(picked.clone());
                let fields = param.as_object_mut().expect("object");
                fields.remove("min");
                fields.remove("max");
                match picked.as_str() {
                    "color" => {
                        fields.insert(
                            "default".to_owned(),
                            serde_json::json!([255, 255, 255, 255]),
                        );
                    }
                    "bool" => {
                        fields.insert("default".to_owned(), Value::Bool(false));
                    }
                    _ => {
                        fields.insert("default".to_owned(), Value::from(0.0));
                        fields.insert("min".to_owned(), Value::from(0.0));
                        fields.insert("max".to_owned(), Value::from(1.0));
                    }
                }
                changed = FormChange::at(&path);
            }
            match picked.as_str() {
                "color" => {
                    changed |= forms::color(ui, &mut param["default"], &format!("{path}/default"));
                }
                "bool" => {
                    let mut flag = param["default"].as_bool().unwrap_or(false);
                    if ui.checkbox(&mut flag, "").changed() {
                        param["default"] = Value::Bool(flag);
                        changed |= FormChange::at(&format!("{path}/default"));
                    }
                }
                _ => {}
            }
        });
        forms::error_line(ui, cx, &format!("{path}/name"));
        if kind == "number" {
            changed |= number_param_fields(ui, cx, param, &path);
        }
    }
    if let Some(index) = remove {
        value["params"].as_array_mut().expect("array").remove(index);
        changed = FormChange::at("/params");
    }
    forms::error_line(ui, cx, "/params");
    if ui.button("+ param").clicked() {
        let taken: Vec<&str> = value["params"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|param| param["name"].as_str())
            .collect();
        let name = (1..)
            .map(|n| format!("param_{n}"))
            .find(|name| !taken.contains(&name.as_str()))
            .unwrap_or_default();
        value["params"]
            .as_array_mut()
            .expect("array")
            .push(serde_json::json!({
                "name": name, "kind": "number", "default": 1.0, "min": 0.0, "max": 2.0
            }));
        changed = FormChange::at("/params");
    }
    if value["params"].as_array().is_some_and(Vec::is_empty) {
        value.as_object_mut().expect("object").remove("params");
    }
    changed
}

fn number_param_fields(ui: &mut Ui, cx: &mut FormCx, param: &mut Value, path: &str) -> FormChange {
    let mut changed = FormChange::default();
    let schema = serde_json::json!({ "type": "number" });
    ui.horizontal(|ui| {
        ui.add_space(PARAM_INDENT);
        for bound in ["min", "max"] {
            ui.label(theme::faint(bound).size(12.0));
            ui.scope(|ui| {
                ui.set_width(BOUND_WIDTH);
                changed |= forms::number(
                    ui,
                    cx,
                    &schema,
                    &mut param[bound],
                    &format!("{path}/{bound}"),
                    false,
                );
            });
        }
        ui.label(theme::faint("default").size(12.0));
        let range = param["min"]
            .as_f64()
            .zip(param["max"].as_f64())
            .filter(|(min, max)| min < max);
        let mut amount = param["default"].as_f64().unwrap_or_default();
        let moved = match range {
            Some((min, max)) => design_tokens::widgets::slider(
                ui,
                egui::Slider::new(&mut amount, min..=max).show_value(true),
            )
            .changed(),
            None => ui
                .add(egui::DragValue::new(&mut amount).speed(0.01))
                .changed(),
        };
        if moved {
            param["default"] = Value::from(tidy(amount));
            changed |= FormChange::at(&format!("{path}/default"));
        }
    });
    for bound in ["min", "max", "default"] {
        forms::error_line(ui, cx, &format!("{path}/{bound}"));
    }
    changed
}

/// A slider's position as a number worth writing into a row: four decimals, no float noise.
fn tidy(number: f64) -> f64 {
    (number * 10_000.0).round() / 10_000.0
}
