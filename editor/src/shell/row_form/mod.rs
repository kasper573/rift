use std::collections::BTreeMap;
pub mod preview;
pub mod quests;
pub mod rows;
pub mod scripts;
pub mod settings;
pub mod tune;
pub mod usages;

use engine::core::audio::playback::SfxDef;
use engine::core::content::serial::TypeTag;
use engine::core::content::{ContentFile, MODULES, ModuleRule, fields_file, name_of};
use engine::core::vfx::VfxShaderDef;
use engine::systems::actor::ModelDef;
use engine::systems::actor::model::ModelModule;
use engine::systems::class::ClassDef;
use engine::systems::item::ItemDef;
use engine::systems::quest::{ObjectiveKind, QuestDef};
use engine::systems::registry;
use engine::systems::script::{ListedScriptModule, RunnerKind, ScriptDef, ScriptId, ScriptModule};
use engine::systems::skill::SkillDef;
use serde_json::Value;
use ui::controls;
use ui::egui;
use ui::egui::{Align, Layout, RichText, Sense, Ui, Vec2};
use ui::form;
use ui::glyph::{self, Mark};
use ui::input;
use ui::menu;
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use self::preview::PreviewPlace;
use self::rows::{fieldless_note, free_singles, show_rows};
use self::scripts::{module_lines, script_module_body, script_summary, speaker_of, summary_of};
use super::{breadcrumb, widgets};
use crate::app::{
    Clipboard, ConfirmAction, ConfirmDialog, Editor, ModalDialog, ModuleClip, PlacePick,
    TextComparison,
};
use crate::forms::{self, FormAction, FormChange, FormCx};
use crate::keymap::EditorKey;
use crate::project::{ContentEdit, Project, RowSelection};
use crate::rows::new_file_into;
use crate::shell::browser::{self, RowMenu};
use crate::tabs::{file_title, shown_path};
use crate::views::{class, model, skill, vfx};
use crate::{make, schema, stage};
use ui::input::{clear_draft, draft_of, set_draft};

const WIDE: f32 = 760.0;
pub(super) const LEFT_COLUMN: f32 = 390.0;
const SINGLE_COLUMN: f32 = 560.0;
const COLUMN_GAP: f32 = 16.0;

pub fn show_row_tab(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str) {
    let place = preview::preview_place(table);
    if place != Some(PreviewPlace::Beside) {
        egui::ScrollArea::vertical()
            .id_salt(("row-tab", table, key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let wide = ui.available_width() >= WIDE;
                row_form(editor, ui, table, key, wide);
                if place == Some(PreviewPlace::Below) {
                    if wide {
                        ui.set_max_width(SINGLE_COLUMN);
                    }
                    ui.add_space(COLUMN_GAP);
                    sheet::section(ui, "preview");
                    preview::show(editor, ui, table, key);
                }
            });
        return;
    }
    let preview_width = stage::preview_width(ui.available_width() / 3.0);
    let form_width = ui.available_width() - preview_width - COLUMN_GAP;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(form_width);
            egui::ScrollArea::vertical()
                .id_salt(("row-tab", table, key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    row_form(editor, ui, table, key, form_width >= WIDE);
                });
        });
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(preview_width);
            sheet::section(ui, "preview");
            preview::show(editor, ui, table, key);
        });
    });
}

pub fn show_rows_tab(editor: &mut Editor, ui: &mut Ui, table: &str, keys: &[String]) {
    egui::ScrollArea::vertical()
        .id_salt(("rows-tab", table))
        .show(ui, |ui| {
            let selection = RowSelection::rows(table, keys.to_vec());
            show_rows(editor, ui, &selection, ui.available_width() >= WIDE);
        });
}

pub fn show_row(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str) {
    row_form(editor, ui, table, key, false);
}

fn row_form(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str, wide: bool) {
    let file = fields_file(key);
    let Some(mut value) = editor.fields_json(key) else {
        ui.label(theme::muted(format!("{file} is gone")));
        return;
    };
    let fixture = editor.shapes.is_fixture(table, key);
    let selected_module: Option<usize> = editor
        .selection
        .parts
        .first()
        .and_then(|part| part.strip_prefix(&modules_pointer()))
        .and_then(|index| index.strip_prefix('/')?.parse().ok());
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    let has_modules = registry().holds_modules(table);
    let Some(reader) = editor.reader() else {
        return;
    };
    let mut modules = if has_modules {
        reader.modules_in(table, &value)
    } else {
        Ok(Vec::new())
    };
    let mut modules_changed = FormChange::default();
    if fixture {
        editor.status.left.push(shown_path(key).to_owned());
        editor
            .status
            .left
            .push("fixture · a row every game has".to_owned());
    }
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    let mut typed: Option<TypedWrite> = None;
    let mut module_ops: Vec<ModuleOp> = Vec::new();
    let mut status: Option<String> = None;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx =
            FormCx::new(project, root, shapes, form, &mut actions).editing(table, Some(key));
        cx.speaker = speaker_of(table, key);
        ui.push_id(("row", table, key), |ui| {
            if wide && has_modules {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(LEFT_COLUMN);
                        changed |= row_card(
                            ui, &mut cx, table, key, fixture, root, &mut value, &mut typed,
                        );
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width().min(420.0));
                        modules_changed |= modules_column(
                            ui,
                            &mut cx,
                            table,
                            &mut modules,
                            &mut module_ops,
                            selected_module,
                            &mut status,
                        );
                    });
                });
            } else {
                if wide {
                    ui.set_max_width(SINGLE_COLUMN);
                }
                changed |= row_card(
                    ui, &mut cx, table, key, fixture, root, &mut value, &mut typed,
                );
                if has_modules {
                    ui.add_space(10.0);
                    modules_changed |= modules_column(
                        ui,
                        &mut cx,
                        table,
                        &mut modules,
                        &mut module_ops,
                        selected_module,
                        &mut status,
                    );
                }
            }
        });
    }
    if let Some(status) = status {
        editor.status.left.push(shown_path(key).to_owned());
        editor.status.left.push(status);
    }
    if !editor.form.status.is_empty() {
        editor.status.left.push(format!(
            "{} › {}",
            shown_path(key),
            editor.form.status.remove(0)
        ));
        editor.status.left.append(&mut editor.form.status);
    }
    if let Ok(modules) = &mut modules {
        for op in module_ops {
            match op {
                ModuleOp::Copy(index) => {
                    editor.clipboard = Some(Clipboard::Modules(ModuleClip {
                        from: key.to_owned(),
                        modules: vec![modules[index].clone()],
                    }));
                    editor.toast("Module copied");
                }
                ModuleOp::Paste => paste_modules(editor, table, key, modules, &mut modules_changed),
            }
        }
    }
    if let (Ok(modules), true) = (&modules, modules_changed.is_some()) {
        match reader.with_modules_in(table, &value, modules) {
            Ok(with) => {
                value = with;
                changed |= modules_changed;
            }
            Err(error) => editor.toast(error),
        }
    }
    if let Some(write) = typed {
        write(editor);
    } else if let Some(label) = changed.label(key) {
        let mut edit = ContentEdit::new(label);
        if table == SkillDef::TABLE
            && let Some(project) = editor.project.as_ref()
            && let Ok(before) = project.read::<SkillDef>(key)
            && let Ok(mut skill) = reader.from_json::<SkillDef>(&value)
        {
            skill::paired_targeting(&before, &mut skill);
            if let Ok(paired) = reader.to_json(&skill) {
                value = paired;
            }
        }
        if table == ItemDef::TABLE
            && let Some(project) = editor.project.as_ref()
            && let Ok(item) = reader.from_json::<ItemDef>(&value)
            && make::usable_without_used(project, &reader, &item)
        {
            match with_used_script(&mut edit, project, key, &value) {
                Ok(with) => value = with,
                Err(error) => editor.toast(error),
            }
        }
        edit.write_json(&file, &value);
        if table == ModelDef::TABLE
            && let Ok(model) = reader.from_json::<ModelDef>(&value)
            && let Some(project) = editor.project.as_ref()
        {
            for face in model::busts::faces_left(&project.files, key, &model) {
                edit.remove(face);
            }
        }
        editor.apply(edit);
    }
    for action in actions {
        act(editor, action, table, key);
    }
}

/// A change a row's own body made to the file as its type, written once the form is drawn.
type TypedWrite = Box<dyn FnOnce(&mut Editor) -> bool>;

#[allow(clippy::too_many_arguments)]
fn row_card(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    key: &str,
    fixture: bool,
    schema: &Value,
    value: &mut Value,
    typed: &mut Option<TypedWrite>,
) -> FormChange {
    let mut changed = FormChange::default();
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut open_folder = None;
        panel::header_with(
            ui,
            |ui| open_folder = breadcrumb::show(ui, key),
            if fixture { "fixture" } else { "" },
            |ui| {
                if ui.button("Copy as JSON").clicked() {
                    ui.ctx()
                        .copy_text(serde_json::to_string_pretty(&*value).unwrap_or_default());
                    ui.close();
                }
                let menu = RowMenu::of(key);
                if let Some(action) = browser::row_menu(ui, &menu, false) {
                    cx.actions.push(FormAction::RowAction(action));
                }
            },
        );
        if let Some(folder) = open_folder {
            cx.actions.push(FormAction::Reveal { path: folder });
        }
        ui.add_space(4.0);
        forms::error_line(ui, cx, "");
        if table == ModelDef::TABLE {
            if let Ok(model) = cx.project.read::<ModelDef>(key) {
                let mut model = (*model).clone();
                if let Some(what) = model::inspect::row_body(ui, &mut model).0 {
                    let (label, key) = (format!("Edit {} {what}", shown_path(key)), key.to_owned());
                    *typed = Some(Box::new(move |editor| {
                        editor.write_content(&label, &key, &model)
                    }));
                }
            }
            return;
        }
        let hidden: &[&str] = match table {
            ClassDef::TABLE => &["modules", "levels", "tree", "backdrop"],
            SkillDef::TABLE => &["modules", "levels"],
            VfxShaderDef::TABLE => &["modules", "params"],
            SfxDef::TABLE => &["modules", "volume", "pitch"],
            _ => &["modules"],
        };
        changed = forms::object_without(ui, cx, schema, value, "", hidden);
        if table == SfxDef::TABLE
            && let Ok(sound) = cx.project.read::<SfxDef>(key)
        {
            let mut sound = (*sound).clone();
            if let Some(field) = tune::show(ui, &mut sound) {
                let label = format!("Edit {field} of {}", file_title(key));
                let key = key.to_owned();
                *typed = Some(Box::new(move |editor| {
                    editor.write_content_as(ContentEdit::new(label).merging(), &key, &sound)
                }));
            }
        }
        if table == ClassDef::TABLE {
            changed |= class::levels(ui, cx, value);
        }
        if table == SkillDef::TABLE {
            changed |= skill::ranks(ui, cx, value);
        }
        if table == VfxShaderDef::TABLE {
            changed |= vfx::shader_params(ui, cx, value);
        }
    });
    changed
}

enum ModuleOp {
    Copy(usize),
    Paste,
}

fn modules_column(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    modules: &mut Result<Vec<Value>, String>,
    ops: &mut Vec<ModuleOp>,
    selected: Option<usize>,
    status: &mut Option<String>,
) -> FormChange {
    let modules = match modules {
        Ok(modules) => modules,
        Err(error) => {
            sheet::section(ui, "Modules");
            form::hint(ui, &format!("The modules show once the row reads: {error}"));
            return FormChange::default();
        }
    };
    let variants = cx.shapes.module_variants(table);
    let mut changed = FormChange::default();
    let count = modules.len();
    sheet::section_with(ui, &format!("Modules · {count}"), |ui| {
        let button = controls::dropdown_primary(ui, "+ Add module");
        let popup = egui::Popup::from_toggle_button_response(&button)
            .id(ui.id().with("add-module"))
            .width(330.0);
        popup.show(|ui| {
            if let Some(picked) = add_module_popup(ui, cx, table, &variants, modules, status) {
                if gives_bust(cx, table, &picked.template) {
                    cx.actions.push(FormAction::GiveBust);
                    ui.close();
                    return;
                }
                let Some(template) = fitting_module(cx, table, picked) else {
                    cx.actions.push(FormAction::NewScriptModule);
                    ui.close();
                    return;
                };
                modules.push(template);
                let path = format!("{table}{}/{}", modules_pointer(), modules.len() - 1);
                cx.state.folded.remove(&path);
                changed = FormChange::at(&modules_pointer());
                ui.close();
            }
        });
        if ui
            .small_button("Paste")
            .on_hover_text("Paste copied modules")
            .clicked()
        {
            ops.push(ModuleOp::Paste);
        }
    });
    let free = free_singles(cx, table, &variants, modules);
    if !free.is_empty() {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::faint(format!("{} still free", free.join(", "))).size(12.0));
            });
        });
    }
    let mut remove = None;
    let mut swap = None;
    let mut duplicate = None;
    let mut objectives = 0;
    let singles: Vec<bool> = modules
        .iter()
        .map(|module| registry().module_rule(table, module) == Some(ModuleRule::AtMostOne))
        .collect();
    for (index, module) in modules.iter_mut().enumerate() {
        let path = format!("{}/{index}", modules_pointer());
        let tag = TypeTag::of_json(module).unwrap_or_default();
        let is_script = tag == ScriptModule::TAG;
        let objective =
            (table == QuestDef::TABLE && ObjectiveKind::of(&tag).is_some()).then(|| {
                objectives += 1;
                objectives
            });
        let folded_key = format!("{table}{path}");
        let folded = match (
            cx.state.folded.contains(&folded_key),
            cx.state.unfolded.contains(&folded_key),
        ) {
            (true, _) => true,
            (false, true) => false,
            (false, false) => is_script && selected != Some(index),
        };
        let is_selected = selected == Some(index);
        panel::in_view(ui, ui.id().with(&folded_key), |ui| {
            let summary = if is_script {
                cx.project
                    .reader()
                    .from_json::<ListedScriptModule>(module)
                    .map_or_else(
                        |error| error,
                        |ListedScriptModule::Script(typed)| script_summary(cx.project, &typed),
                    )
            } else if let Some(objective) = objective {
                format!("objective {objective}")
            } else {
                summary_of(cx, module)
            };
            panel::card(is_selected).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let header = ui
                    .horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
                                if ui.button("Copy").clicked() {
                                    ops.push(ModuleOp::Copy(index));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(!singles[index], egui::Button::new("Duplicate"))
                                    .on_disabled_hover_text(format!(
                                        "A row holds at most one {} module",
                                        tag.replace('_', " ")
                                    ))
                                    .clicked()
                                {
                                    duplicate = Some(index);
                                    ui.close();
                                }
                                ui.separator();
                                swap = menu::move_entries(ui, index, count).or(swap);
                                ui.separator();
                                if ui.button("Remove").clicked() {
                                    remove = Some(index);
                                    ui.close();
                                }
                            });
                            glyph::show(
                                ui,
                                if folded {
                                    Mark::CaretRight
                                } else {
                                    Mark::CaretDown
                                },
                                11.0,
                                theme::FAINT,
                            );
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.label(theme::strong(&tag).monospace());
                                if folded || !is_script {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(summary.to_uppercase())
                                                .size(11.0)
                                                .color(theme::MUTED),
                                        )
                                        .truncate(),
                                    );
                                }
                            });
                        });
                    })
                    .response
                    .interact(Sense::click());
                if header.clicked() {
                    if folded {
                        cx.state.folded.remove(&folded_key);
                        cx.state.unfolded.insert(folded_key.clone());
                    } else {
                        cx.state.unfolded.remove(&folded_key);
                        cx.state.folded.insert(folded_key.clone());
                    }
                }
                if !folded {
                    changed |= card_body(ui, cx, table, &variants, module, &path);
                }
            });
        });
        ui.add_space(2.0);
    }
    if let Some(index) = remove {
        modules.remove(index);
        changed = FormChange::at(&modules_pointer());
    }
    if let Some((a, b)) = swap {
        modules.swap(a, b);
        changed = FormChange::at(&modules_pointer());
    }
    if let Some(index) = duplicate {
        let copy = modules[index].clone();
        modules.insert(index + 1, copy);
        changed = FormChange::at(&modules_pointer());
    }
    changed
}

/// Where a row's modules are, as a pointer into its file.
pub(super) fn modules_pointer() -> String {
    format!("/{MODULES}")
}

pub(super) fn card_body(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    module: &mut Value,
    path: &str,
) -> FormChange {
    ui.add_space(4.0);
    let mut changed = FormChange::default();
    match schema::variant_of(variants, module) {
        Some(variant) if variant.tag == ScriptModule::TAG => {
            changed |= forms::as_type(
                ui,
                cx,
                module,
                path,
                |ui, cx, ListedScriptModule::Script(typed)| {
                    script_module_body(ui, cx, table, typed, path)
                },
            );
        }
        Some(variant) => {
            if schema::is_fieldless(variant.schema) {
                form::hint(ui, &fieldless_note(variant.schema));
            } else {
                changed |= forms::object_without(ui, cx, variant.schema, module, path, &[]);
            }
        }
        None => changed |= forms::raw_json(ui, cx, module, path),
    }
    changed
}

/// The module `picked` as it is added: a script module names the nearest script run by rows of
/// `table`'s kind, and is none when there is no such script, so one is made for it.
fn fitting_module(cx: &FormCx, table: &str, picked: PickedModule) -> Option<Value> {
    let (true, Ok(kind)) = (picked.script, table.parse::<RunnerKind>()) else {
        return Some(picked.template);
    };
    let script = runnable_script(cx, kind)?;
    let module = ScriptModule {
        script,
        values: scripts::required_values(cx, script),
    };
    cx.project
        .reader()
        .to_json(&ListedScriptModule::Script(module))
        .ok()
}

/// The script nearest the row being edited that its owner, of `kind`, can run: one run by `kind`,
/// one named for the row first among those as near.
pub(crate) fn runnable_script(cx: &FormCx, kind: RunnerKind) -> Option<ScriptId> {
    let index = cx.project.files.index();
    let row = cx.row_key.as_deref().map(name_of).unwrap_or_default();
    let nearest = index
        .paths_of(ScriptDef::TABLE)
        .filter(|path| {
            cx.project
                .read::<ScriptDef>(path)
                .is_ok_and(|script| script.runs_on == kind)
        })
        .min_by_key(|path| {
            let named_for_row = !row.is_empty() && name_of(path).starts_with(row);
            (cx.seen_from_here(path).0, !named_for_row, path.to_owned())
        })?;
    cx.project.reader().id_of(nearest)
}

/// Whether `template` is a model's bust, which can't be written without its faces, so is given
/// with them as one step.
fn gives_bust(cx: &FormCx, table: &str, template: &Value) -> bool {
    table == ModelDef::TABLE
        && cx
            .project
            .reader()
            .from_json::<ModelModule>(template)
            .is_ok_and(|module| matches!(module, ModelModule::Bust(_)))
}

/// A module type picked to add, as its template.
pub(super) struct PickedModule {
    pub template: Value,
    /// Whether it names a script, whose template can't name one that fits before it is added.
    pub script: bool,
}

pub(super) fn add_module_popup(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    list: &[Value],
    status: &mut Option<String>,
) -> Option<PickedModule> {
    let registry = registry();
    let id = ui.id().with("add-module-filter");
    let mut filter = draft_of(ui, id).unwrap_or_default();
    let response = input::search(ui, &mut filter, "filter", None);
    if response.changed() {
        set_draft(ui, id, &filter.clone());
    }
    input::default_focus(ui, &response);
    let needle = filter.to_lowercase();
    let mut picked = None;
    let mut first = None;
    let mut blocked_count = 0;
    let mut offered = 0;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for variant in variants {
                let is_script = variant.tag == ScriptModule::TAG;
                offered += 1;
                let template = schema::variant_template(cx.root, variant, cx.nearby());
                let at_most_one =
                    registry.module_rule(table, &template) == Some(ModuleRule::AtMostOne);
                let blocked = !registry.takes_module(table, list, &template);
                if blocked {
                    blocked_count += 1;
                }
                let title = variant.tag.replace('_', " ");
                if !needle.is_empty() && !variant.tag.contains(&needle) && !title.contains(&needle)
                {
                    continue;
                }
                let description = schema::description(variant.schema)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        if is_script {
                            "any number · names a script row".to_owned()
                        } else if at_most_one {
                            "at most one".to_owned()
                        } else {
                            "any number".to_owned()
                        }
                    });
                let line = if blocked {
                    "at most one · already added".to_owned()
                } else {
                    description
                };
                let full = ui.available_rect_before_wrap();
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(full.width(), 38.0),
                    if blocked {
                        Sense::hover()
                    } else {
                        Sense::click()
                    },
                );
                if response.hovered() && !blocked {
                    paint::row_wash(ui.painter(), rect);
                }
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(Vec2::new(6.0, 3.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                );
                if blocked {
                    child.add_enabled_ui(false, |ui| widgets::badge(ui, &variant.tag));
                } else {
                    widgets::badge(&mut child, &variant.tag);
                }
                child.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(title).monospace().color(if blocked {
                        theme::FAINT
                    } else {
                        theme::TEXT
                    }));
                    ui.add(
                        egui::Label::new(RichText::new(line).size(11.5).color(if blocked {
                            theme::FAINT
                        } else {
                            theme::MUTED
                        }))
                        .truncate(),
                    );
                });
                let module = PickedModule {
                    template,
                    script: is_script,
                };
                if !blocked && first.is_none() {
                    first = Some(PickedModule {
                        template: module.template.clone(),
                        script: is_script,
                    });
                }
                if response.clicked() {
                    picked = Some(module);
                }
            }
        });
    if picked.is_none() && ui.input(|i| EditorKey::Confirm.pressed(i)) {
        picked = first;
    }
    *status = Some(format!(
        "{offered} module types for {table}{}",
        if blocked_count > 0 {
            format!(" · {blocked_count} at most one, already used")
        } else {
            String::new()
        }
    ));
    if picked.is_some() || ui.input(|i| EditorKey::Cancel.pressed(i)) {
        clear_draft(ui, id);
        if picked.is_none() {
            ui.close();
        }
    }
    picked
}

pub fn with_row_form(
    editor: &mut Editor,
    table: &str,
    key: &str,
    show: impl FnOnce(&mut FormCx) -> bool,
) -> bool {
    let Some(root) = editor.fields_json(key) else {
        return false;
    };
    editor
        .with_form(table, Some(key), &root, show)
        .unwrap_or(false)
}

pub fn act(editor: &mut Editor, action: FormAction, table: &str, key: &str) {
    match action {
        FormAction::OpenRow { table, key } => {
            editor.open_row(&table, &key);
        }
        FormAction::OpenNode { .. } => {}
        FormAction::PickPlace { path } => {
            editor.pick = Some(PlacePick {
                file: fields_file(key),
                pointer: path,
                area: None,
            });
            editor.toast(format!(
                "Click a walkable cell on an area to pick the place ({} click for any point)",
                EditorKey::Unsnapped.name()
            ));
        }
        FormAction::NewRow {
            table: into_table,
            path,
        } => {
            if let Err(error) = new_file_into(editor, &into_table, &fields_file(key), &path) {
                editor.toast(error);
            }
        }
        FormAction::RowAction(action) => browser::act(editor, table, key, action),
        FormAction::Reveal { path } => crate::shell::content_tree::reveal(editor, &path),
        FormAction::NewScriptModule => {
            if let Err(error) = crate::rows::new_script_module(editor, table, key) {
                editor.toast(error);
            }
        }
        FormAction::NewObjectScript { node } => {
            if let Err(error) = crate::views::area::inspect::new_object_script(editor, key, &node) {
                editor.toast(error);
            }
        }
        FormAction::GiveBust => {
            let faces = model::busts::browse_faces(key);
            if !faces.is_empty() {
                model::busts::give(editor, key, &faces);
            }
        }
    }
}

fn paste_modules(
    editor: &mut Editor,
    table: &str,
    key: &str,
    list: &mut Vec<Value>,
    changed: &mut FormChange,
) {
    let Some(Clipboard::Modules(clip)) = editor.clipboard.clone() else {
        editor.toast("Nothing copied");
        return;
    };
    let from = clip.from;
    let registry = registry();
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    for module in &clip.modules {
        let tag = TypeTag::of_json(module).unwrap_or_default();
        let Some(rule) = registry.module_rule(table, module) else {
            editor.toast(format!("a {table} row takes no {tag} module"));
            continue;
        };
        let existing = list
            .iter()
            .position(|known| TypeTag::of_json(known).as_ref() == Some(&tag));
        match (rule, existing) {
            (ModuleRule::AtMostOne, Some(index)) => {
                editor.dialogs.push(ModalDialog::Confirm(ConfirmDialog {
                    title: format!("{} already has {tag}", shown_path(key)),
                    body: format!(
                        "A {table} has at most one {tag} module. Replace it with {from}'s?"
                    ),
                    label: format!("Replace {tag}"),
                    compare: Some(TextComparison {
                        left_title: format!("{key} now"),
                        left: module_lines(root, &list[index]),
                        right_title: format!("copied from {from}"),
                        right: module_lines(root, module),
                    }),
                    action: ConfirmAction::ReplaceModule {
                        table: table.to_owned(),
                        key: key.to_owned(),
                        index,
                        module: module.clone(),
                    },
                }));
            }
            _ => {
                list.push(module.clone());
                *changed = FormChange::at(&modules_pointer());
            }
        }
    }
}

/// The usable item `value` at `key` with a script answering used made for it in `edit`, as the
/// game takes no usable item without one.
fn with_used_script(
    edit: &mut ContentEdit,
    project: &Project,
    key: &str,
    value: &Value,
) -> Result<Value, String> {
    let (reader, script) = make::answering_used(edit, project, key, RunnerKind::Item)?;
    let module = reader
        .to_json(&ListedScriptModule::Script(ScriptModule {
            script,
            values: BTreeMap::new(),
        }))
        .map_err(|error| error.to_string())?;
    let mut modules = reader.modules_in(ItemDef::TABLE, value)?;
    modules.push(module);
    reader.with_modules_in(ItemDef::TABLE, value, &modules)
}
