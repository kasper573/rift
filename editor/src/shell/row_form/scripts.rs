use std::collections::BTreeMap;
use ui::egui;
use ui::form;
use ui::theme;

use engine::core::content::ContentFile;
use engine::field;
use engine::systems::npc::NpcDef;
use engine::systems::script::{
    Condition, RiftScripts, RunnerKind, ScriptDef, ScriptId, ScriptModule, VarValue,
};
use serde_json::Value;
use ui::egui::{Align, Layout, Ui};
use ui::glyph::Mark;

use crate::forms::{self, FormAction, FormChange, FormCx};
use crate::project::Project;
use crate::views::script::nodes::{NodeText, var_value_text};
use crate::views::script::panel;
use crate::{condition, schema};

/// The scripts `scripts` lists at `path`, each as a card, in a row of `table`, run by owners of
/// `kind`. Adding one names the nearest script they can run, or asks for `none_fits` when there's
/// none.
pub fn script_cards(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    kind: RunnerKind,
    scripts: &mut Vec<ScriptModule>,
    path: &str,
    none_fits: FormAction,
) -> FormChange {
    let mut changed = FormChange::default();
    let mut remove = None;
    let mut duplicate = None;
    for (index, module) in scripts.iter_mut().enumerate() {
        let module_path = format!("{path}/{index}");
        ui::panel::card(true).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::strong(ScriptModule::TAG).monospace());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
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
            ui.add_space(4.0);
            changed |= script_module_body(ui, cx, table, module, &module_path);
        });
        ui.add_space(2.0);
    }
    if ui::controls::button(ui, "+ Add script").clicked() {
        match super::runnable_script(cx, kind) {
            Some(script) => {
                scripts.push(ScriptModule {
                    script,
                    values: required_values(cx, script),
                });
                changed = FormChange::at(path);
            }
            None => cx.actions.push(none_fits),
        }
    }
    if let Some(index) = remove {
        scripts.remove(index);
        changed = FormChange::at(path);
    }
    if let Some(index) = duplicate {
        let copy = scripts[index].clone();
        scripts.insert(index + 1, copy);
        changed = FormChange::at(path);
    }
    changed
}

/// The fields of a module naming a script row: which script, the values for its variables, and
/// what the script may do for the row running it.
pub(super) fn script_module_body(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    module: &mut ScriptModule,
    path: &str,
) -> FormChange {
    let mut changed = forms::of_type_without(ui, cx, module, path, &[field!(ScriptModule, values)]);
    changed |= script_values(ui, cx, module, path);
    script_extras(ui, cx, table, module, path);
    changed
}

fn script_extras(ui: &mut Ui, cx: &mut FormCx, table: &str, module: &ScriptModule, path: &str) {
    let Some(pointer) = path.find("/modules/").map(|at| path[at..].to_owned()) else {
        return;
    };
    let key = cx.speaker.clone();
    if let (Some(content), Some(row_key)) =
        (cx.content.clone(), key.as_deref().or(cx.row_key.as_deref()))
        && content.has_built::<RiftScripts>()
        && let Some(bound) = panel::owner_row(&content, table, row_key, &pointer)
    {
        let may = content.built::<RiftScripts>().may_do(bound.row);
        let answers: Vec<&str> = may.answers.iter().map(|kind| kind.name()).collect();
        form::kv_text(
            ui,
            "answers",
            if answers.is_empty() {
                "nothing".to_owned()
            } else {
                answers.join(" · ")
            },
        );
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
        form::kv(ui, "may do", |ui| {
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
    let reader = cx.project.reader();
    if let Some(script) = reader.path_of(module.script) {
        ui.add_space(4.0);
        if ui.button((Mark::Arrow.text(13.0), "Open script")).clicked() {
            cx.actions.push(FormAction::OpenRow {
                table: ScriptDef::TABLE.to_owned(),
                key: script.to_owned(),
            });
        }
    }
}

pub(super) fn script_summary(project: &Project, module: &ScriptModule) -> String {
    let reader = project.reader();
    let text = NodeText {
        project,
        reader: &reader,
        catalogue: &project.catalogue,
    };
    let mut parts = vec![reader.path_of(module.script).unwrap_or("?").to_owned()];
    for (name, value) in &module.values {
        parts.push(format!("{name} {}", var_value_text(&text, value)));
    }
    parts.join(" · ")
}

/// The values `script` asks for with no default of its own, each at a value of its type, which
/// a module naming it starts with so it runs.
pub(super) fn required_values(cx: &FormCx, script: ScriptId) -> BTreeMap<String, VarValue> {
    let Some(script) = cx.project.read_id(script) else {
        return BTreeMap::new();
    };
    let (shape, required) = values_shape(&script);
    let values: serde_json::Map<String, Value> = required
        .iter()
        .map(|name| {
            let fresh = schema::template(&shape, schema::property(&shape, name), cx.nearby());
            (name.clone(), fresh)
        })
        .collect();
    cx.project
        .reader()
        .from_json(&Value::Object(values))
        .unwrap_or_default()
}

/// The values of `script`'s variables as a form shows them, and those it can't run without.
fn values_shape(script: &ScriptDef) -> (Value, Vec<String>) {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, variable) in &script.variables {
        let kind = schema::variable_schema(variable.ty, variable.table.as_deref());
        properties.insert(name.clone(), kind);
        if variable.default.is_none() {
            required.push(name.clone());
        }
    }
    (schema::object_of(properties, required.clone()), required)
}

fn script_values(
    ui: &mut Ui,
    cx: &mut FormCx,
    module: &mut ScriptModule,
    path: &str,
) -> FormChange {
    let Some(script) = cx.project.read_id(module.script) else {
        return FormChange::default();
    };
    if script.variables.is_empty() {
        return FormChange::default();
    }
    let (shape, required) = values_shape(&script);
    let values_path = format!("{path}{}", field!(ScriptModule, values));
    let reader = cx.project.reader();
    let Ok(mut values) = reader.to_json(&module.values) else {
        form::hint(ui, "the values do not write as the game reads them");
        return FormChange::default();
    };
    let mut changed = FormChange::default();
    if let Some(given) = values.as_object_mut() {
        for name in &required {
            if given.contains_key(name) {
                continue;
            }
            given.insert(
                name.clone(),
                schema::template(&shape, schema::property(&shape, name), cx.nearby()),
            );
            changed = FormChange::at(&values_path);
        }
    }
    changed |= forms::object(ui, cx, &shape, &mut values, &values_path);
    if !changed.is_some() {
        return changed;
    }
    match reader.from_json::<BTreeMap<String, VarValue>>(&values) {
        Ok(read) => {
            module.values = read;
            changed
        }
        Err(error) => {
            cx.state.errors.insert(values_path, error);
            FormChange::default()
        }
    }
}

pub(super) fn speaker_of(table: &str, key: &str) -> Option<String> {
    (table == NpcDef::TABLE).then(|| key.to_owned())
}

pub fn summary_of(cx: &FormCx, module: &Value) -> String {
    let reader = cx.project.reader();
    schema::summary_of(cx.root, module_shape(cx.root, module), module, |field| {
        reader
            .from_json::<Condition>(field)
            .ok()
            .map(|condition| condition::summary(cx.project, &condition))
    })
}

pub fn module_lines(root: &Value, module: &Value) -> String {
    schema::lines_of(root, module_shape(root, module), module)
}

fn module_shape<'a>(root: &'a Value, module: &Value) -> &'a Value {
    schema::modules_in(root).map_or(&Value::Null, |items| schema::settle(root, items, module))
}
