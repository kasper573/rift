use std::str::FromStr;

use egui::{Align, Layout, RichText, Sense, Ui, Vec2};
use engine::core::math::Pos;
use engine::core::script::VariableType;
use engine::core::tiling::{CellPos, Tiles};
use engine::systems::area::{self, AreaModuleKind};
use serde_json::Value;

use super::inspect::hidden_fields;
use super::{holes, modules, placement};
use crate::app::Editor;
use crate::forms::FormCx;
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::ui::{inspector, theme, widgets};
use crate::value::RowValue;
use crate::{forms, schema};

pub fn show_blueprint(editor: &mut Editor, ui: &mut Ui, key: &str, selected: &[usize]) {
    let Some(mut row) = editor.row_value("blueprint", key) else {
        return;
    };
    let parts: Vec<Value> = row["parts"].as_array().cloned().unwrap_or_default();
    let crumb = match selected {
        [one] => format!(
            "blueprint/{key} › {}",
            parts
                .get(*one)
                .map(super::shapes::label)
                .unwrap_or_default()
        ),
        [] => format!("blueprint/{key}"),
        many => format!("blueprint/{key} › {} parts", many.len()),
    };
    ui.label(theme::muted(crumb).monospace().size(13.0));
    ui.add_space(4.0);
    widgets::section_count(ui, "Parts", parts.len());
    let mut pick: Option<Vec<usize>> = None;
    let shift = ui.input(|i| i.modifiers.shift);
    for (index, part) in parts.iter().enumerate() {
        let kind = part.kind_or("part");
        let at = super::shapes::bounds(part, None)
            .map(|b| format!("{}, {}", b.min().x, b.min().y))
            .unwrap_or_default();
        let on = selected.contains(&index);
        let full = ui.available_rect_before_wrap();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(full.width(), 22.0), Sense::click());
        if on {
            ui.painter().rect_filled(rect, 3.0, theme::ACCENT_SOFT);
        } else if response.hovered() {
            ui.painter().rect_filled(
                rect,
                3.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8),
            );
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(6.0, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        widgets::badge(&mut child, kind);
        child.label(
            theme::mono(super::shapes::label(part))
                .size(13.0)
                .color(if on { theme::TEXT } else { theme::CODE }),
        );
        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(theme::muted(at).monospace().size(12.0));
        });
        if response.clicked() {
            let mut next: Vec<usize> = if shift { selected.to_vec() } else { Vec::new() };
            if let Some(at) = next.iter().position(|i| *i == index) {
                next.remove(at);
            } else {
                next.push(index);
            }
            pick = Some(next);
        }
    }
    let mut add: Option<Value> = None;
    ui.horizontal(|ui| {
        let button = ui.add(
            egui::Button::new("+ Add part")
                .fill(theme::RAISED)
                .stroke(egui::Stroke::new(1.0, theme::BORDER)),
        );
        ui.label(theme::faint("anything an area places but scripts").size(12.0));
        egui::Popup::from_toggle_button_response(&button)
            .id(ui.id().with(("add-part", key)))
            .width(300.0)
            .show(|ui| {
                add = add_part_popup(editor, ui);
            });
    });
    if let Some(next) = pick {
        editor.selection = RowSelection {
            table: "blueprint".to_owned(),
            keys: vec![key.to_owned()],
            parts: next.iter().copied().map(super::blueprint_part).collect(),
        };
    }
    if let Some(part) = add {
        let label = super::shapes::label(&part);
        row["parts"].as_array_mut().expect("parts").push(part);
        let index = row["parts"].as_array().map_or(0, Vec::len) - 1;
        let mut edit = ContentEdit::new(format!("Add part {label} · blueprint/{key}"));
        edit.write_json(row_file("blueprint", key), &row);
        if editor.apply(edit) {
            editor.selection = RowSelection {
                table: "blueprint".to_owned(),
                keys: vec![key.to_owned()],
                parts: vec![super::blueprint_part(index)],
            };
        }
        return;
    }
    if !selected.is_empty() {
        ui.add_space(6.0);
        part_cards(editor, ui, key, &mut row, selected);
    }
    placed_list(editor, ui, key);
    variables_section(editor, ui, key, &row);
    blueprint_modules(editor, ui, key, &mut row);
}

pub(super) fn add_part_popup(editor: &Editor, ui: &mut Ui) -> Option<Value> {
    ui.label(
        RichText::new("ADD PART")
            .size(11.0)
            .color(theme::MUTED)
            .family(theme::medium()),
    );
    let mut picked = None;
    for (group, kinds) in modules::ADD_MENU {
        let kinds: Vec<&(AreaModuleKind, &str)> = kinds
            .iter()
            .filter(|(kind, _)| {
                !matches!(
                    kind,
                    AreaModuleKind::InlineScript | AreaModuleKind::SharedScript
                )
            })
            .collect();
        if kinds.is_empty() {
            continue;
        }
        ui.add_space(2.0);
        ui.label(RichText::new(*group).size(11.0).color(theme::FAINT));
        for &(kind, description) in kinds {
            if modules::module_menu_row(ui, kind.tag(), description)
                && let Some(template) = modules::module_template(editor, kind, CellPos::new(0, 0))
            {
                picked = Some(template);
                ui.close();
            }
        }
    }
    ui.separator();
    ui.add_enabled_ui(false, |ui| {
        ui.label(
            RichText::new("inline · shared script")
                .monospace()
                .color(theme::FAINT),
        );
        ui.label(
            RichText::new("the blueprint's own, under Modules")
                .size(11.5)
                .color(theme::FAINT),
        );
    });
    picked
}

struct PlacedAt {
    area: String,
    at: Pos<Tiles>,
    this: bool,
    origin: usize,
}

pub(super) fn part_cards(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    row: &mut Value,
    selected: &[usize],
) {
    let shapes = editor.shapes.clone();
    let shape = shapes.shape_of("area");
    let variants = shapes.module_variants("area");
    let variables: Vec<(String, VariableType)> = row["variables"]
        .as_object()
        .map(|vars| {
            vars.iter()
                .filter_map(|(name, v)| {
                    Some((name.clone(), VariableType::from_str(v.kind()).ok()?))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut changed = false;
    let mut actions = Vec::new();
    let mut hole_ops: Vec<holes::HoleOp> = Vec::new();
    let mut remove: Option<usize> = None;
    let mut duplicate: Option<usize> = None;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, shape, shapes, form, &mut actions)
            .owned_by("blueprint", Some(key));
        for &index in selected {
            let path = super::blueprint_part(index);
            let Some(part) = row["parts"].get_mut(index) else {
                continue;
            };
            let kind = part.kind_or("part").to_owned();
            let module_kind = part.module_kind();
            let holes = holes::holes_in(part, "");
            let holeable = holes::holeable_fields(shapes, part);
            widgets::card_frame(true).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, &kind.replace('_', " "), "", |ui| {
                    if ui.button("Duplicate").clicked() {
                        duplicate = Some(index);
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Remove").shortcut_text("Del"))
                        .clicked()
                    {
                        remove = Some(index);
                        ui.close();
                    }
                });
                ui.add_space(4.0);
                let mut plain = part.clone();
                let before = plain.clone();
                cx.holes = Some(forms::FormHoles {
                    fields: holeable
                        .iter()
                        .map(|field| {
                            (
                                format!("{path}{}", field.pointer),
                                forms::FieldHole::Open(field.kind),
                            )
                        })
                        .chain(holes.iter().map(|hole| {
                            (
                                format!("{path}{}", hole.pointer),
                                forms::FieldHole::Filled(hole.name.clone()),
                            )
                        }))
                        .collect(),
                    variables: variables.clone(),
                    requests: Vec::new(),
                });
                match schema::variant_of(&variants, &plain) {
                    Some(variant) => {
                        changed |= forms::object_without(
                            ui,
                            &mut cx,
                            variant.schema,
                            &mut plain,
                            &path,
                            hidden_fields(module_kind),
                        )
                        .is_some();
                    }
                    None => {
                        changed |= forms::raw_json(ui, &mut cx, &mut plain, &path).is_some();
                    }
                }
                if plain != before {
                    *part = plain;
                }
                if module_kind == Some(AreaModuleKind::Object) {
                    changed |= crate::ui::inspector::scripts::script_cards(
                        ui, &mut cx, "area", part, "scripts", &path,
                    )
                    .is_some();
                }
                if let Some(holes) = cx.holes.take() {
                    for request in holes.requests {
                        hole_ops.push(holes::hole_op(index, &path, request));
                    }
                }
            });
            ui.add_space(4.0);
        }
    }
    let mut made: Vec<holes::MadeVariable> = Vec::new();
    for op in hole_ops {
        made.extend(holes::apply_hole_op(&editor.shapes, row, op));
        changed = true;
    }
    if let Some(index) = remove {
        row["parts"].as_array_mut().map(|list| list.remove(index));
        let mut edit = ContentEdit::new(format!("Remove part {index} · blueprint/{key}"));
        edit.write_json(row_file("blueprint", key), row);
        if editor.apply(edit) {
            editor.selection.parts.clear();
        }
        return;
    }
    if let Some(index) = duplicate {
        let copy = row["parts"][index].clone();
        if let Some(list) = row["parts"].as_array_mut() {
            list.insert(index + 1, copy);
        }
        changed = true;
    }
    if changed {
        let mut edit = ContentEdit::new(format!("Edit parts · blueprint/{key}"));
        if !made.is_empty() {
            let keeping = placements_keeping(editor, key, &made);
            let kept = kept_values(&keeping, key);
            let names: Vec<String> = made
                .iter()
                .map(|variable| format!("{} · {}", variable.name, variable.kind.name()))
                .collect();
            edit = ContentEdit::new(format!(
                "Make variable {} · no default · values kept by {kept} placement{} · blueprint/{key}",
                names.join(", "),
                widgets::plural(kept),
            ));
            for (area_key, area_row) in keeping {
                edit.write_json(row_file("area", &area_key), &area_row);
            }
        }
        edit.write_json(row_file("blueprint", key), row);
        editor.apply(edit);
    }
    for action in actions {
        inspector::act(editor, action, "blueprint", key);
    }
}

pub(super) fn placed_list(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return;
    };
    let Some(id) = content.by_key::<area::blueprint::BlueprintDef>(key) else {
        return;
    };
    let editing: Option<(String, usize)> = editor.views.area.views.values().find_map(|view| {
        let modules = super::modules::area_modules(editor, &view.area);
        let chain = placement::resolve_in_place(&content, &modules, &view.in_place)?;
        let innermost = chain.last()?;
        (innermost.blueprint == key).then(|| (view.area.clone(), innermost.module))
    });
    let mut placed: Vec<PlacedAt> = Vec::new();
    let mut nested = 0;
    for area_id in content.ids::<area::AreaDef>() {
        let area_key = area_id.key(&content).to_owned();
        for placement in area::placements(&content, area_id) {
            if placement.blueprint != id {
                continue;
            }
            if placement.chain.len() > 1 {
                nested += 1;
                continue;
            }
            let this = editing
                .as_ref()
                .is_some_and(|(a, m)| *a == area_key && *m == placement.origin);
            placed.push(PlacedAt {
                area: area_key.clone(),
                at: placement.at,
                this,
                origin: placement.origin,
            });
        }
    }
    let mut areas: Vec<&String> = placed.iter().map(|placed| &placed.area).collect();
    areas.sort();
    areas.dedup();
    let heading = match areas.as_slice() {
        [] => "Placed nowhere yet".to_owned(),
        [one] => format!("Placed · {}, all on {one}", placed.len()),
        many => format!("Placed · {} in {} areas", placed.len(), many.len()),
    };
    widgets::section(ui, &heading);
    let mut open: Option<&PlacedAt> = None;
    for placed in placed.iter().take(8) {
        let response = ui
            .horizontal(|ui| {
                widgets::badge(ui, "area");
                ui.label(theme::mono(&placed.area).size(13.0).color(theme::CODE));
                ui.label(
                    theme::mono(format!(
                        "{}, {}",
                        forms::tiles(placed.at.x),
                        forms::tiles(placed.at.y)
                    ))
                    .size(13.0),
                );
                if placed.this {
                    ui.label(theme::faint("this one").size(12.0));
                }
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            open = Some(placed);
        }
    }
    if placed.len() > 8 {
        ui.label(
            theme::faint(format!("… {} more", placed.len() - 8))
                .monospace()
                .size(12.5),
        );
    }
    if nested > 0 {
        ui.label(theme::faint(format!("{nested} more inside other blueprints")).size(12.5));
    }
    if let Some(placed) = open {
        super::open_area_at(
            editor,
            &placed.area,
            Some(placed.at),
            Some(format!("/modules/{}", placed.origin)),
        );
    }
}

pub(super) fn variables_section(editor: &Editor, ui: &mut Ui, key: &str, row: &Value) {
    let Some(variables) = row["variables"].as_object() else {
        return;
    };
    if variables.is_empty() {
        return;
    }
    widgets::section_count(ui, "Variables", variables.len());
    let area_keys: Vec<String> = editor
        .project
        .as_ref()
        .and_then(|p| p.content.clone())
        .map(|content| {
            content
                .ids::<area::AreaDef>()
                .map(|id| id.key(&content).to_owned())
                .collect()
        })
        .unwrap_or_default();
    let placements: Vec<Value> = area_keys
        .iter()
        .filter_map(|area_key| editor.row_value("area", area_key))
        .flat_map(|row| row["modules"].as_array().cloned().unwrap_or_default())
        .filter(|module| module.is_module(AreaModuleKind::Blueprint) && module["blueprint"] == key)
        .collect();
    let mut holes = Vec::new();
    for part in row["parts"].as_array().into_iter().flatten() {
        area::blueprint::holes(part, &mut holes);
    }
    for module in row["modules"].as_array().into_iter().flatten() {
        area::blueprint::holes(module, &mut holes);
    }
    for (name, variable) in variables {
        let count = holes.iter().filter(|h| *h == name).count();
        let default = match variable.get("default") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => "no default".to_owned(),
            Some(other) => other.to_string(),
        };
        ui.horizontal(|ui| {
            ui.label(theme::muted("◇").size(13.0));
            ui.label(theme::mono(name).size(13.0).color(theme::TEXT));
            ui.label(
                theme::muted(format!("{} · {default}", variable.kind_or("?")))
                    .monospace()
                    .size(12.5),
            );
        });
        let own = placements
            .iter()
            .filter(|placement| placement["values"].get(name).is_some())
            .count();
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            ui.label(
                theme::faint(format!(
                    "{count} hole{} · {own} with own value{}",
                    widgets::plural(count),
                    widgets::plural(own)
                ))
                .monospace()
                .size(12.0),
            );
        });
    }
}

pub(super) fn blueprint_modules(editor: &mut Editor, ui: &mut Ui, key: &str, row: &mut Value) {
    let shapes = editor.shapes.clone();
    let shape = shapes.shape_of("blueprint");
    let count = row["modules"].as_array().map_or(0, Vec::len);
    widgets::section_count(ui, "Modules", count);
    let mut changed = false;
    let mut actions = Vec::new();
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, shape, shapes, form, &mut actions)
            .owned_by("blueprint", Some(key));
        changed |= crate::ui::inspector::scripts::script_cards(
            ui,
            &mut cx,
            "blueprint",
            row,
            "modules",
            "",
        )
        .is_some();
    }
    if changed {
        let mut edit = ContentEdit::new(format!("Edit modules · blueprint/{key}"));
        edit.write_json(row_file("blueprint", key), row);
        editor.apply(edit);
    }
    for action in actions {
        inspector::act(editor, action, "blueprint", key);
    }
}

fn kept_values(keeping: &[(String, Value)], key: &str) -> usize {
    keeping
        .iter()
        .map(|(_, row)| {
            row["modules"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|module| {
                    module.is_module(AreaModuleKind::Blueprint) && module["blueprint"] == key
                })
                .count()
        })
        .sum()
}

fn placements_keeping(
    editor: &Editor,
    key: &str,
    made: &[holes::MadeVariable],
) -> Vec<(String, Value)> {
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for area_id in content.ids::<area::AreaDef>() {
        let area_key = area_id.key(&content).to_owned();
        let Some(mut row) = editor.row_value("area", &area_key) else {
            continue;
        };
        let mut touched = false;
        for module in row["modules"].as_array_mut().into_iter().flatten() {
            if !module.is_module(AreaModuleKind::Blueprint) || module["blueprint"] != key {
                continue;
            }
            for variable in made {
                if module["values"].get(&variable.name).is_none() {
                    module["values"][&variable.name] = variable.kept.clone();
                    touched = true;
                }
            }
        }
        if touched {
            out.push((area_key, row));
        }
    }
    out
}
