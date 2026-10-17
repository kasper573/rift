use std::str::FromStr;

use egui::{Align, Layout, RichText, Ui};
use engine::core::script::VariableType;
use engine::systems::script::RiftScripts;
use serde_json::Value;

use super::card_body;
use super::{theme, widgets};
use crate::forms::{FormAction, FormChange, FormCx};
use crate::project::row_file;
use crate::schema;
use crate::value::RowValue;
use crate::views::model::busts;
use crate::views::script::{graph, nodes, panel};
use crate::{condition, forms};

pub fn script_cards(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    owner: &mut Value,
    field: &str,
    path: &str,
) -> FormChange {
    let variants: Vec<schema::SchemaVariant> = cx
        .shapes
        .module_variants(table)
        .into_iter()
        .filter(|v| v.tag.ends_with("_script"))
        .collect();
    if !owner[field].is_array() {
        owner[field] = Value::Array(Vec::new());
    }
    let mut changed = FormChange::default();
    let count = owner[field].as_array().map_or(0, Vec::len);
    let mut remove = None;
    let mut duplicate = None;
    for index in 0..count {
        let module_path = format!("{path}/{field}/{index}");
        let module = &mut owner[field][index];
        let tag = module.kind_or("script").to_owned();
        widgets::card_frame(true).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::strong(tag.replace('_', " ")).monospace());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button(theme::faint("…"), |ui| {
                        if ui.button("Duplicate").clicked() {
                            duplicate = Some(index);
                            ui.close();
                        }
                        if ui.button("Remove").clicked() {
                            remove = Some(index);
                            ui.close();
                        }
                    });
                });
            });
            changed |= card_body(ui, cx, table, &variants, module, &module_path);
        });
        if tag == "inline_script" {
            outline_block(ui, cx, &owner[field][index]);
        }
        ui.add_space(2.0);
    }
    ui.horizontal(|ui| {
        let button = ui.add(
            egui::Button::new(RichText::new("+ Add script").color(theme::TEXT))
                .fill(theme::RAISED)
                .stroke(egui::Stroke::new(1.0, theme::BORDER)),
        );
        egui::Popup::from_toggle_button_response(&button)
            .id(ui.id().with(("add-script", path)))
            .width(300.0)
            .show(|ui| {
                for variant in &variants {
                    let description = match variant.tag.as_str() {
                        "inline_script" => "its nodes on this object",
                        _ => "names a script row",
                    };
                    if ui
                        .add(
                            egui::Button::new(format!(
                                "{}  ·  {description}",
                                variant.tag.replace('_', " ")
                            ))
                            .frame(false),
                        )
                        .clicked()
                    {
                        let template = schema::variant_template(cx.root, variant);
                        owner[field].as_array_mut().expect("scripts").push(template);
                        changed = FormChange::at(&format!("{path}/{field}"));
                        ui.close();
                    }
                }
            });
    });
    let list = owner[field].as_array_mut().expect("scripts");
    if let Some(index) = remove {
        list.remove(index);
        changed = FormChange::at(&format!("{path}/{field}"));
    }
    if let Some(index) = duplicate {
        let copy = list[index].clone();
        list.insert(index + 1, copy);
        changed = FormChange::at(&format!("{path}/{field}"));
    }
    changed
}

pub(super) fn script_extras(ui: &mut Ui, cx: &mut FormCx, table: &str, module: &Value, path: &str) {
    let tag = module.kind();
    if tag != "inline_script" && tag != "shared_script" {
        return;
    }
    let Some(pointer) = path.find("/modules/").map(|at| path[at..].to_owned()) else {
        return;
    };
    let key = cx.speaker.clone();
    let inline = tag == "inline_script";
    if inline {
        let count = module["nodes"].as_object().map_or(0, |nodes| nodes.len());
        let dialogue = module["nodes"].as_object().map_or(0, |nodes| {
            nodes.values().filter(|n| n["type"] == "dialogue").count()
        });
        widgets::kv_text(
            ui,
            "nodes",
            if dialogue > 0 {
                format!("{count} · {dialogue} dialogue")
            } else {
                count.to_string()
            },
        );
        let events: Vec<String> = module["nodes"]
            .as_object()
            .into_iter()
            .flat_map(|nodes| nodes.values())
            .filter(|node| node["type"] == "on")
            .filter_map(|node| node["event"].as_str().map(str::to_owned))
            .collect();
        widgets::kv_text(
            ui,
            "answers",
            if events.is_empty() {
                "nothing yet".to_owned()
            } else {
                events.join(" · ")
            },
        );
    }
    if let (Some(content), Some(owner_key)) = (
        cx.content.clone(),
        key.as_deref().or(cx.owner_key.as_deref()),
    ) && content.has_built::<RiftScripts>()
        && let Some(bound) = panel::owner_row(&content, table, owner_key, &pointer)
    {
        let may = content.built::<RiftScripts>().may_do(bound.row);
        if !inline {
            let answers: Vec<&str> = may.answers.iter().map(|kind| kind.name()).collect();
            widgets::kv_text(
                ui,
                "answers",
                if answers.is_empty() {
                    "nothing".to_owned()
                } else {
                    answers.join(" · ")
                },
            );
        }
        let mut lines: Vec<String> = may
            .reaches
            .iter()
            .map(|reach| panel::describe_reach(&content, reach))
            .collect();
        lines.sort();
        lines.dedup();
        if !may.dialogue.is_empty() {
            lines.push(format!("speaks {}", may.dialogue.join(", ")));
        }
        if !may.calls.is_empty() {
            lines.push(format!("runs {}", may.calls.join(", ")));
        }
        widgets::kv(ui, "may do", |ui| {
            ui.add(
                egui::Label::new(if lines.is_empty() {
                    theme::faint("nothing yet")
                } else {
                    theme::muted(lines.join(" · "))
                })
                .wrap(),
            );
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if inline {
            if ui.button("→ Open script").clicked() {
                cx.actions.push(FormAction::OpenScript {
                    pointer: pointer.clone(),
                });
            }
            if ui
                .button("Make shared…")
                .on_hover_text("Move these nodes into a new script row")
                .clicked()
            {
                cx.actions.push(FormAction::MakeShared {
                    pointer: pointer.clone(),
                });
            }
        } else if let Some(script) = module["script"].as_str() {
            if ui.button("→ Open script").clicked() {
                cx.actions.push(FormAction::OpenRow {
                    table: "script".to_owned(),
                    key: script.to_owned(),
                });
            }
            if ui
                .button("Make inline")
                .on_hover_text("Copy the script's nodes into this row")
                .clicked()
            {
                cx.actions.push(FormAction::MakeInline {
                    script: script.to_owned(),
                });
            }
        }
    });
}

pub(super) fn outline_block(ui: &mut Ui, cx: &mut FormCx, module: &Value) {
    let Some(nodes) = module["nodes"]
        .as_object()
        .filter(|nodes| !nodes.is_empty())
    else {
        return;
    };
    ui.add_space(4.0);
    widgets::section(ui, "Outline · read-only");
    let catalogue = cx.catalogue;
    let rows = nodes::outline(nodes, catalogue);
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            for row in rows.iter().take(40) {
                let node = &nodes[&row.name];
                let summary = nodes::summary(node, catalogue);
                ui.horizontal(|ui| {
                    ui.add_space(14.0 * row.depth as f32);
                    if let Some(via) = &row.via {
                        ui.label(theme::code(format!("{via} →")).size(12.0));
                    }
                    let (glyph, color) = graph::kind_glyph(node);
                    ui.label(RichText::new(glyph).color(color).size(12.0));
                    ui.label(theme::mono(node.kind()).size(12.5).strong());
                    ui.label(theme::mono(&row.name).size(12.5).color(theme::CODE));
                    if !summary.is_empty() {
                        ui.label(
                            theme::muted(format!("· {}", widgets::clip(&summary, 60))).size(12.0),
                        );
                    }
                });
            }
            if rows.len() > 40 {
                ui.label(theme::faint(format!("… {} more", rows.len() - 40)));
            }
        });
}

pub(super) fn script_summary(module: &Value) -> String {
    if module["type"] == "shared_script" {
        let mut parts = vec!["shared".to_owned()];
        if let Some(values) = module["values"]
            .as_object()
            .or_else(|| module["variables"].as_object())
        {
            for (name, value) in values {
                parts.push(format!(
                    "{name} {}",
                    value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| value.as_f64().map(|number| number.to_string()))
                        .unwrap_or_else(|| value.to_string())
                ));
            }
        } else if let Some(script) = module["script"].as_str() {
            parts.push(script.to_owned());
        }
        return parts.join(" · ");
    }
    let nodes = module["nodes"].as_object();
    let events: Vec<String> = nodes
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .filter(|node| node["type"] == "on")
        .filter_map(|node| node["event"].as_str().map(str::to_owned))
        .collect();
    let count = nodes.map_or(0, |nodes| nodes.len());
    if events.is_empty() {
        format!("{count} nodes")
    } else {
        format!("{} · {count} nodes", events.join(", "))
    }
}

pub(super) fn script_values(
    ui: &mut Ui,
    cx: &mut FormCx,
    module: &mut Value,
    path: &str,
) -> FormChange {
    let Some(script) = module["script"].as_str().map(str::to_owned) else {
        return FormChange::default();
    };
    let Some(row) = cx.files.json(&row_file("script", &script)) else {
        return FormChange::default();
    };
    let Some(variables) = row["variables"].as_object() else {
        return FormChange::default();
    };
    if variables.is_empty() {
        return FormChange::default();
    }
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, variable) in variables {
        let ty = VariableType::from_str(variable.kind_or("text")).unwrap_or(VariableType::Text);
        let kind = schema::variable_schema(ty, variable["table"].as_str());
        properties.insert(name.clone(), kind);
        if variable.get("default").is_none() {
            required.push(Value::String(name.clone()));
        }
    }
    let schema =
        serde_json::json!({ "type": "object", "properties": properties, "required": required });
    if !module["values"].is_object() {
        module["values"] = Value::Object(serde_json::Map::new());
    }
    forms::object(
        ui,
        cx,
        &schema,
        &mut module["values"],
        &format!("{path}/values"),
    )
}

pub(super) fn speaker_of(table: &str, key: &str) -> Option<String> {
    (table == "npc").then(|| key.to_owned())
}

pub(super) fn is_objective(tag: &str) -> bool {
    !matches!(
        tag,
        "shown"
            | "requires"
            | "grants"
            | "hand_in"
            | "hand_over"
            | "reward"
            | "pick_one"
            | "daily"
            | "time_limit"
            | "drop"
            | "inline_script"
            | "shared_script"
    )
}

pub fn summary_of(root: &Value, module: &Value) -> String {
    let Some(object) = module.as_object() else {
        return String::new();
    };
    let shape = module_shape(root, module);
    let mut parts = Vec::new();
    if object.get("type").and_then(Value::as_str) == Some("bust") {
        let own = object
            .get("own")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        return format!("{} expressions", busts::generic().count() + own);
    }
    for (name, field) in object {
        if name == "type" {
            continue;
        }
        match field {
            Value::String(text) => parts.push(text.clone()),
            Value::Number(number) => {
                let number = number.as_f64().unwrap_or(0.0);
                parts.push(
                    schema::number_shorthand(root, &shape["properties"][name], number)
                        .unwrap_or_else(|| format!("{name} {number}")),
                );
            }
            Value::Array(items) => parts.push(format!("{} {name}", items.len())),
            Value::Object(nodes) if name == "nodes" => parts.push(format!("{} nodes", nodes.len())),
            Value::Object(fields) if fields.contains_key("type") => {
                parts.push(condition::summary(field))
            }
            _ => {}
        }
        if parts.len() >= 3 {
            break;
        }
    }
    parts.join(" · ")
}

pub fn module_lines(root: &Value, module: &Value) -> String {
    let shape = module_shape(root, module);
    let mut lines: Vec<String> = Vec::new();
    for (name, field) in module.as_object().into_iter().flatten() {
        if name == "type" {
            continue;
        }
        let field_shape = &shape["properties"][name];
        match field {
            Value::Array(items) => {
                let item_shape =
                    schema::resolve(root, &schema::non_null(root, field_shape)["items"]);
                for item in items {
                    match item.as_object() {
                        Some(_) => lines.push(entry_cells(root, item_shape, item).join("  ")),
                        None => lines.push(item.to_string()),
                    }
                }
            }
            Value::Object(_) => lines.push(format!("{name} …")),
            other => lines.push(format!("{name} {}", cell(root, field_shape, other))),
        }
    }
    lines.join("\n")
}

fn module_shape<'a>(root: &'a Value, module: &Value) -> &'a Value {
    schema::settle(root, &root["properties"]["modules"]["items"], module)
}

fn entry_cells(root: &Value, shape: &Value, entry: &Value) -> Vec<String> {
    let fields = schema::properties(schema::settle(root, shape, entry));
    if fields.is_empty() {
        return entry
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, value)| cell(root, &Value::Null, value))
            .collect();
    }
    fields
        .into_iter()
        .filter_map(|(name, field)| match entry.get(name.as_str()) {
            Some(value) if !value.is_null() => Some(cell(root, field, value)),
            _ if field.get("x-count").is_some() => field
                .get("default")
                .map(|default| cell(root, field, default)),
            _ => field
                .get("x-blank")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
        .collect()
}

fn cell(root: &Value, field: &Value, value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or(0.0);
            if schema::non_null(root, field)["x-unit"] == "%" {
                format!("{number}%")
            } else {
                number.to_string()
            }
        }
        other => other.to_string(),
    }
}
