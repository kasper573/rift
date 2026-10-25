use std::collections::{BTreeMap, HashSet};
use ui::chip;
use ui::controls;
use ui::egui::Ui;
use ui::form;
use ui::glyph::Mark;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rng};
use engine::core::script::VariableType;
use engine::core::terrain::TerrainSize;
use engine::core::units::{Meters, TileSize};
use engine::systems::area::prop::PropDef;
use strum::VariantArray;

use super::brush::{Brush, LARGEST_RADIUS, SMALLEST_RADIUS, SculptMode};
use engine::field;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::object::ObjectDef;
use engine::systems::area::scene::{self, SceneNode, scene_file};
use engine::systems::area::{AreaDef, AreaModule, SoundscapeDef};
use engine::systems::script::{RunnerKind, ScriptModule, VarValue, Variable};
use serde_json::Value;

use super::nodes::SceneDraft;
use super::placement::{self, placement_extras};
use super::population::from_npc;
use super::preview::{clipboard_panel, pick_preview};
use super::soundscape::soundscape_extras;
use super::{AreaTool, AreaViewState, nodes, scene_tree, stamp};
use crate::app::Editor;
use crate::forms::{self, FieldHole, FormAction, FormCx, FormHoles, HoleRequest};
use crate::keymap::EditorKey;
use crate::project::{ContentEdit, IdPaths, RowSelection};
use crate::shell::{row_form, widgets};
use crate::tabs::file_title;
use crate::{picker, rows, schema};

/// The view's inspector: a paste or a pick under way, the selected nodes, the blueprint parts
/// edited in place, or else the card of the tool in hand.
pub fn sidebar(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    if !editor.views.area.views.contains_key(view_key) {
        return;
    }
    if editor.views.area.view(view_key).paste.is_some() {
        clipboard_panel(editor, ui);
        return;
    }
    if editor.pick.is_some() {
        pick_preview(editor, ui);
        return;
    }
    let Some(snapshot) = super::AreaSnapshot::for_view(editor, view_key) else {
        return;
    };
    let selection = editor.selection.clone();
    let in_place = snapshot
        .in_place
        .last()
        .map(|place| place.blueprint.clone());
    if in_place.is_some()
        && selection.keys.first() == in_place.as_ref()
        && !selection.parts.is_empty()
    {
        show_parts(editor, ui, view_key, &selection);
        return;
    }
    let parts: Vec<String> = super::selected_parts_of(editor, &snapshot.bundle)
        .into_iter()
        .cloned()
        .collect();
    let node = |part: &String| nodes::node(snapshot.nodes(), part);
    match parts.as_slice() {
        [folder] if node(folder).is_some_and(nodes::is_folder) => {
            folder_card(editor, ui, &snapshot, folder);
        }
        [] => tool(editor, ui, view_key, &snapshot),
        _ => show_parts(editor, ui, view_key, &selection),
    }
}

fn tool(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &super::AreaSnapshot) {
    let content = snapshot.content.clone();
    match editor.views.area.view(view_key).tool {
        AreaTool::Sculpt => sculpt_card(editor, ui, view_key),
        AreaTool::Paint => paint_card(editor, ui, view_key, snapshot),
        AreaTool::Place => match editor.views.area.view(view_key).placeable.clone() {
            Some(super::Placeable::Prop(prop)) => prop_card(editor, ui, view_key, &prop),
            _ => stamp_card(editor, ui, view_key, &content),
        },
        AreaTool::Scatter => scatter_card(editor, ui, view_key),
        AreaTool::Select | AreaTool::Module => form::nothing_selected(
            ui,
            "Select nodes in the scene or on the map, or take a tool with settings, to inspect them here.",
        ),
    }
}

fn brush_fields(ui: &mut Ui, brush: &mut Brush) {
    form::kv(ui, "radius", |ui| {
        let mut radius = brush.radius.0;
        if controls::slider(ui, &mut radius, SMALLEST_RADIUS.0..=LARGEST_RADIUS.0).changed() {
            brush.radius = Meters(radius);
        }
        controls::key_hint_text(ui, &EditorKey::BrushLarger.name(), "grows");
    });
    form::kv(ui, "strength", |ui| {
        controls::slider(ui, &mut brush.strength, 0.0..=1.0);
    });
    form::kv(ui, "falloff", |ui| {
        controls::slider(ui, &mut brush.falloff, 0.0..=1.0);
    });
}

fn sculpt_card(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let view = editor.views.area.view_mut(view_key);
    tool_card(ui, "Sculpt", |ui| {
        form::kv(ui, "mode", |ui| {
            for mode in SculptMode::VARIANTS {
                if chip::mode(ui, mode.as_ref(), view.sculpt == *mode).clicked() {
                    view.sculpt = *mode;
                }
            }
        });
        form::hint(ui, "Ctrl smooths, Shift inverts while you drag.");
        brush_fields(ui, &mut view.brush);
        if view.sculpt == SculptMode::Level {
            form::kv(ui, "level", |ui| {
                let mut level = view.level.0;
                if controls::slider(ui, &mut level, -8.0..=16.0).changed() {
                    view.level = Meters(level);
                }
            });
        }
    });
}

fn paint_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &super::AreaSnapshot) {
    let area = snapshot.area();
    let materials: Vec<(String, ui::egui::Color32, bool)> = area
        .terrain
        .materials
        .iter()
        .map(|id| {
            let ground = id.get(&snapshot.content);
            let [r, g, b, _] = ground.color.0.to_be_bytes();
            (
                ground.name.clone(),
                ui::egui::Color32::from_rgb(r, g, b),
                ground.walk == engine::systems::area::ground::Walk::Blocks,
            )
        })
        .collect();
    let view = editor.views.area.view_mut(view_key);
    tool_card(ui, "Paint", |ui| {
        form::kv(ui, "material", |ui| {
            for (slot, (name, color, blocks)) in materials.iter().enumerate() {
                let ink = if color.r() as u32 + color.g() as u32 + color.b() as u32 > 380 {
                    ui::egui::Color32::BLACK
                } else {
                    ui::egui::Color32::WHITE
                };
                let on = view.paint_slot == slot;
                let response = chip::colored(ui, name.clone(), *color, ink);
                if *blocks {
                    ui::paint::hatched(ui.painter(), response.rect, ui::ink::wash(ink, 0.35), 8.0);
                    response.clone().on_hover_text("blocks walking");
                }
                if on {
                    ui.painter().rect_stroke(
                        response.rect.expand(1.0),
                        3.0,
                        ui::egui::Stroke::new(1.5, theme::ACCENT),
                        ui::egui::StrokeKind::Outside,
                    );
                }
                if response.clicked() {
                    view.paint_slot = slot;
                }
            }
        });
        brush_fields(ui, &mut view.brush);
    });
}

fn prop_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, prop: &str) {
    let def = editor.read::<PropDef>(prop).and_then(Result::ok);
    let view = editor.views.area.view_mut(view_key);
    tool_card(ui, "Place", |ui| {
        form::kv_text(ui, "prop", prop);
        if let Some(def) = def
            && let Some(bounds) = def.footprint.bounds(Pos::new(0.0, 0.0), Degrees(0.0), 1.0)
        {
            form::kv_text(
                ui,
                "footprint",
                format!(
                    "{} × {} m",
                    crate::forms::metres(bounds.size.width),
                    crate::forms::metres(bounds.size.height)
                ),
            );
        }
        form::kv(ui, "turned", |ui| {
            ui.label(theme::mono(format!("{}°", view.place_yaw.0.round())));
            controls::key_hint_text(ui, &EditorKey::TurnClockwise.name(), "turns");
        });
        form::kv(ui, "snap", |ui| {
            ui.label(theme::muted("half metres").size(12.5));
            controls::key_hint_text(ui, &EditorKey::Unsnapped.name(), "for 1/16");
        });
        form::hint(
            ui,
            "Click the ground to place one. Pick another prop in the palette below.",
        );
    });
}

fn scatter_card(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let view = editor.views.area.view(view_key);
    tool_card(ui, "Scatter", |ui| {
        match view.prop() {
            Some(prop) => form::kv_text(ui, "prop", prop),
            None => form::hint(ui, "Pick a prop in the palette below."),
        }
        form::hint(
            ui,
            "Drag a region on the ground to scatter copies of the prop over it. Reseed a scatter from its card to lay them out anew.",
        );
    });
}

fn folder_card(editor: &mut Editor, ui: &mut Ui, snapshot: &super::AreaSnapshot, pointer: &str) {
    let Some(node) = nodes::node(snapshot.nodes(), pointer).cloned() else {
        return;
    };
    let label = node.label().unwrap_or_default().to_owned();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    nodes::walk(nodes::children_of(&node), &mut |_, child| {
        if !nodes::is_folder(child) {
            *counts.entry(nodes::kind_tag(child)).or_default() += 1;
        }
    });
    let flags = snapshot.flags.row(pointer);
    let mut action = None;
    ui.label(
        theme::muted(format!("{} › {label}", file_title(&snapshot.bundle)))
            .monospace()
            .size(13.0),
    );
    ui.add_space(4.0);
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state = if flags.hidden.on() { "hidden" } else { "" };
        panel::header(ui, &format!("folder {label}"), state, |_| {});
        ui.add_space(4.0);
        if counts.is_empty() {
            ui.label(theme::muted("Empty").size(12.5));
        }
        for (kind, count) in &counts {
            ui.horizontal(|ui| {
                widgets::badge(ui, kind);
                ui.label(theme::mono(format!("{count} × {}", schema::title(kind))).size(13.0));
            });
        }
        ui.add_space(4.0);
        form::kv(ui, "shown", |ui| {
            if ui.checkbox(&mut !flags.hidden.on(), "").clicked() {
                action = Some(FolderAction::Toggle(scene_tree::Toggle::Hide));
            }
        });
        form::kv(ui, "locked", |ui| {
            if ui.checkbox(&mut flags.locked.on(), "").clicked() {
                action = Some(FolderAction::Toggle(scene_tree::Toggle::Lock));
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if controls::marked(ui, Mark::Copy, "Make blueprint").clicked() {
            action = Some(FolderAction::MakeBlueprint);
        }
        if ui.button("Ungroup").clicked() {
            action = Some(FolderAction::Ungroup);
        }
        if ui.button("Select what's inside").clicked() {
            action = Some(FolderAction::SelectInside);
        }
    });
    let bundle = snapshot.bundle.clone();
    match action {
        Some(FolderAction::Toggle(toggle)) => {
            scene_tree::toggle(editor, &bundle, &[pointer.to_owned()], toggle);
        }
        Some(FolderAction::MakeBlueprint) => {
            if let Some(snapshot) = super::AreaSnapshot::for_bundle(editor, &bundle) {
                super::run_action(editor, &snapshot, super::AreaAction::MakeBlueprint);
            }
        }
        Some(FolderAction::Ungroup) => scene_tree::ungroup(editor, &bundle, &[pointer.to_owned()]),
        Some(FolderAction::SelectInside) => {
            let mut inside = Vec::new();
            nodes::walk(nodes::children_of(&node), &mut |at, child| {
                if matches!(child, SceneNode::Module(_)) {
                    inside.push(format!("{pointer}/children{at}"));
                }
            });
            nodes::select(editor, &bundle, inside);
        }
        None => {}
    }
}

enum FolderAction {
    Toggle(scene_tree::Toggle),
    MakeBlueprint,
    Ungroup,
    SelectInside,
}

fn tool_card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::strong(title).monospace());
        });
        ui.add_space(4.0);
        body(ui);
    });
}

fn stamp_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, content: &Content) {
    let stamp = editor.views.area.view(view_key).stamp().map(str::to_owned);
    let viewed = super::viewed_bundle(view_key)
        .expect("an area or blueprint view")
        .to_owned();
    let mut save = false;
    tool_card(ui, "Stamp", |ui| {
        let mut picked = stamp.clone().unwrap_or_default();
        let mut actions = Vec::new();
        {
            let Editor {
                project,
                form,
                shapes,
                ..
            } = editor;
            let project = project.as_ref().expect("project");
            let root = Value::Null;
            let mut cx = FormCx::new(project, &root, shapes, form, &mut actions)
                .editing(table_of(&viewed).unwrap_or_default(), Some(&viewed));
            form::kv(ui, "blueprint", |ui| {
                if picker::row(
                    ui,
                    &mut cx,
                    BlueprintDef::TABLE,
                    &mut picked,
                    "/stamp/blueprint",
                ) && !picked.is_empty()
                {
                    editor.views.area.view_mut(view_key).placeable =
                        Some(super::Placeable::Blueprint(picked.clone()));
                }
            });
        }
        for action in actions {
            row_form::act(editor, action, AreaDef::TABLE, "");
        }
        let Some(stamp) = editor.views.area.view(view_key).stamp().map(str::to_owned) else {
            form::hint(
                ui,
                "Pick a blueprint below, then click to place one or drag a line.",
            );
            return;
        };
        let Some(reader) = editor.reader() else {
            return;
        };
        let footprint = {
            let values = stamp::stamp_values(editor, Some(&stamp));
            let size = stamp::stamp_footprint(editor, content, &reader, &stamp, &values);
            (
                size.width.round().max(1.0) as u32,
                size.height.round().max(1.0) as u32,
            )
        };
        let variables = editor
            .read::<BlueprintDef>(&stamp)
            .and_then(Result::ok)
            .map(|def| def.variables.clone())
            .unwrap_or_default();
        let settings = editor.settings.stamps.entry(stamp.clone()).or_default();
        form::kv(ui, "turned", |ui| {
            for quarters in 0..4u8 {
                let on = settings.quarter_turns == quarters;
                if chip::mode(ui, format!("{}°", u32::from(quarters) * 90), on).clicked() {
                    settings.quarter_turns = quarters;
                    save = true;
                }
            }
            controls::key_hint_text(ui, &EditorKey::TurnClockwise.name(), "turns");
        });
        form::kv(ui, "spacing", |ui| {
            let label = match settings.spacing {
                None => format!("footprint · {} × {}", footprint.0, footprint.1),
                Some(n) => format!("{n} m"),
            };
            controls::combo((view_key, "spacing"))
                .selected_text(theme::mono(label))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            settings.spacing.is_none(),
                            theme::mono(format!("footprint · {} × {}", footprint.0, footprint.1)),
                        )
                        .clicked()
                    {
                        settings.spacing = None;
                        save = true;
                    }
                    for n in [1u32, 2, 4, 8, 16, 32] {
                        if ui
                            .selectable_label(
                                settings.spacing == Some(n),
                                theme::mono(format!("{n} m")),
                            )
                            .clicked()
                        {
                            settings.spacing = Some(n);
                            save = true;
                        }
                    }
                });
        });
        form::kv(ui, "snap", |ui| {
            ui.label(theme::muted("half metres").size(12.5));
            controls::key_hint_text(ui, &EditorKey::Unsnapped.name(), "for 1/16");
        });
        if variables.is_empty() {
            form::kv_text(ui, "values", "none: no variables");
        } else {
            let mut values = settings.values.clone();
            let mut values_changed = false;
            {
                let Editor {
                    project,
                    form,
                    shapes,
                    ..
                } = editor;
                let project = project.as_ref().expect("project");
                let mut actions = Vec::new();
                let root = Value::Null;
                let mut cx = FormCx::new(project, &root, shapes, form, &mut actions)
                    .editing(table_of(&viewed).unwrap_or_default(), Some(&viewed));
                for (name, variable) in &variables {
                    form::kv(ui, name, |ui| {
                        let mut value = values.get(name).cloned().unwrap_or_else(|| {
                            placement::variable_default(&reader, variable).unwrap_or_else(|| {
                                placement::variable_template(variable, cx.nearby())
                            })
                        });
                        if variable.ty == VariableType::Name {
                            ui.label(theme::muted("named per copy").size(12.5));
                            return;
                        }
                        let edited = placement::variable_field(
                            ui,
                            &mut cx,
                            variable,
                            &mut value,
                            &format!("/stamp/{stamp}/{name}"),
                        );
                        if edited || (!values.contains_key(name) && variable.default.is_none()) {
                            values.insert(name.clone(), value);
                            values_changed = true;
                        }
                    });
                }
            }
            if values_changed {
                let settings = editor.settings.stamps.entry(stamp.clone()).or_default();
                settings.values = values;
                save = true;
            }
        }
    });
    if save {
        editor.save_settings();
    }
}

pub fn show_parts(editor: &mut Editor, ui: &mut Ui, view_key: &str, selection: &RowSelection) {
    let Some(bundle) = selection.keys.first().cloned() else {
        return;
    };
    let Some(read) = nodes::bundle_of(editor, &bundle) else {
        return;
    };
    let mut draft = read.draft();
    let size = read.size();
    let selected: Vec<String> = selection
        .parts
        .iter()
        .filter(|part| nodes::module_node(&draft.nodes, part).is_some())
        .cloned()
        .collect();
    if selected.is_empty() {
        return;
    }
    let shapes = editor.shapes.clone();
    let table = table_of(&bundle).unwrap_or(AreaDef::TABLE).to_owned();
    let mut changed = false;
    let mut actions = Vec::new();
    let mut out = Followups::default();
    let mut group = false;
    let label;
    {
        let Editor {
            project,
            form,
            shapes: editor_shapes,
            views,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let reader = project.reader();
        let mut cx = FormCx::new(
            project,
            &shapes.scene_module,
            editor_shapes,
            form,
            &mut actions,
        )
        .editing(&table, Some(&bundle))
        .errors_of(&scene_file(&bundle));
        if let [node] = selected.as_slice() {
            let view = views.area.views.get_mut(&super::view_key(&bundle));
            label = nodes::module_node(&draft.nodes, node)
                .map(|module| super::shapes::label(&module.module, reader.as_ref()))
                .unwrap_or_default();
            changed =
                module_inspector(ui, &mut cx, view, &bundle, size, &mut draft, node, &mut out);
        } else {
            label = format!("{} modules", selected.len());
            ui.label(
                theme::muted(format!(
                    "{} › {} modules",
                    file_title(&bundle),
                    selected.len()
                ))
                .monospace()
                .size(13.0),
            );
            ui.label(theme::muted(spans(&draft.nodes, &selected, reader.as_ref())).size(12.5));
            ui.add_space(4.0);
            panel::card(true).show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, &format!("{} modules", selected.len()), "", |ui| {
                    if ui.button("Copy").clicked() {
                        out.actions.push(super::AreaAction::Copy);
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        out.actions.push(super::AreaAction::Duplicate);
                        ui.close();
                    }
                    if ui
                        .add(widgets::shortcut_button("Remove all", EditorKey::Delete))
                        .clicked()
                    {
                        out.actions.push(super::AreaAction::Delete);
                        ui.close();
                    }
                });
                ui.add_space(4.0);
                changed |= placement::shared_values(ui, &mut cx, &mut draft.nodes, &selected);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(widgets::shortcut_button(
                        "Group into folder",
                        EditorKey::Group,
                    ))
                    .clicked()
                {
                    group = true;
                }
                if controls::marked(ui, Mark::Copy, "Make blueprint").clicked() {
                    out.actions.push(super::AreaAction::MakeBlueprint);
                }
            });
            controls::key_hint_text(
                ui,
                &EditorKey::MakeBlueprint.name(),
                "makes a blueprint of the selection",
            );
        }
    }
    if changed && let Some(project) = editor.project.as_ref() {
        let mut edit = ContentEdit::new(format!("Edit {label}"));
        match read.write(&mut edit, project, &bundle, |own| *own = draft) {
            Ok(()) => {
                editor.apply(edit);
            }
            Err(error) => editor.toast(error),
        }
    }
    for action in actions {
        row_form::act(editor, action, &table, &bundle);
    }
    if group {
        let ids = super::AreaSnapshot::for_view(editor, view_key)
            .map(|snapshot| scene_tree::selected_ids(&snapshot, &editor.selection))
            .unwrap_or_default();
        editor.views.area.view_mut(view_key).scene.queued =
            Some(scene_tree::SceneAction::Group(ids));
    }
    for action in out.actions {
        if let Some(snapshot) = super::AreaSnapshot::for_bundle(editor, &bundle) {
            super::run_action(editor, &snapshot, action);
        }
    }
    if editor.pick.is_some() {
        ui.add_space(6.0);
        pick_preview(editor, ui);
    }
}

/// The card of the module at `node` of the scene `draft` of the area or blueprint at `bundle`: its
/// fields as the game reads them, a blueprint's holes among them, and what its kind adds.
#[allow(clippy::too_many_arguments)]
fn module_inspector(
    ui: &mut Ui,
    cx: &mut FormCx,
    view: Option<&mut AreaViewState>,
    bundle: &str,
    size: TerrainSize,
    draft: &mut SceneDraft,
    node: &str,
    out: &mut Followups,
) -> bool {
    let path = node.to_owned();
    let reader = cx.project.reader();
    let Some(module) = nodes::module_node(&draft.nodes, node) else {
        return false;
    };
    let kind = module.module.kind();
    let shown_kind = kind.tag().replace('_', " ");
    let crumbs: Vec<String> = std::iter::once(file_title(bundle).to_owned())
        .chain(folders_above(&draft.nodes, node, reader.as_ref()))
        .chain(std::iter::once(shown_kind.clone()))
        .collect();
    ui.label(theme::muted(crumbs.join(" › ")).monospace().size(13.0));
    ui.add_space(4.0);
    let holes_here = table_of(bundle) == Some(BlueprintDef::TABLE);
    if holes_here {
        cx.holes = Some(form_holes(
            &reader,
            &draft.variables,
            &module.module,
            &module.holes,
            &path,
        ));
    }
    let Some(module) = nodes::module_node_mut(&mut draft.nodes, node) else {
        return false;
    };
    let mut changed = false;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, &shown_kind, "", |ui| {
            if ui.button("Copy").clicked() {
                out.actions.push(super::AreaAction::Copy);
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                out.actions.push(super::AreaAction::Duplicate);
                ui.close();
            }
            ui.separator();
            if ui
                .add(widgets::shortcut_button("Remove", EditorKey::Delete))
                .clicked()
            {
                out.actions.push(super::AreaAction::Delete);
                ui.close();
            }
        });
        ui.add_space(4.0);
        let roamed = matches!(&module.module, AreaModule::Population(p) if p.roams.is_some());
        changed |= module_fields(ui, cx, &mut module.module, &path);
        if let AreaModule::Population(population) = &mut module.module
            && !roamed
            && let Some(roams) = &mut population.roams
        {
            *roams = size.meters().bounds();
        }
        if let (AreaModule::Population(_), Some(view)) = (&mut module.module, view) {
            let preview = view.population_previews.entry(path.clone()).or_default();
            form::kv(ui, "preview", |ui| {
                let mut shown = !preview.hidden;
                if ui.checkbox(&mut shown, "").changed() {
                    preview.hidden = !shown;
                }
                if controls::marked(ui, Mark::Reload, "Re-roll").clicked() {
                    preview.seed = preview.seed.wrapping_add(1);
                }
            });
        }
    });
    match &mut module.module {
        AreaModule::Blueprint(placement) => {
            let placed = reader
                .path_of(placement.blueprint)
                .map(str::to_owned)
                .and_then(|path| Some((cx.project.read::<BlueprintDef>(&path).ok()?, path)));
            if let Some((def, placed)) = placed
                && !def.variables.is_empty()
            {
                sheet::section(ui, &format!("Values · {}", file_title(&placed)));
                panel::card(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    panel::title(ui, "values");
                    ui.add_space(4.0);
                    changed |= placement::placement_card(ui, cx, placement, &def, &path);
                });
            }
            placement_extras(ui, cx, placement, out, node);
        }
        AreaModule::Population(population) => {
            if let Some(npc) = reader.path_of(population.npc) {
                from_npc(ui, cx, npc);
            }
        }
        AreaModule::Resident(resident) => {
            if let Some(npc) = reader.path_of(resident.npc) {
                from_npc(ui, cx, npc);
            }
        }
        AreaModule::Object(object) => {
            ui.add_space(4.0);
            changed |= object_scripts(ui, cx, object, node);
            object_extras(ui, object, reader.as_ref(), out, node);
        }
        AreaModule::Soundscape(soundscape) => {
            changed |= soundscape_extras(ui, cx, soundscape, &path)
        }
        AreaModule::Scatter(scatter) => {
            ui.add_space(4.0);
            if ui
                .button("Reseed")
                .on_hover_text("lays the copies out anew")
                .clicked()
            {
                scatter.seed = Rng::random_seed();
                changed = true;
            }
        }
        AreaModule::Script(_) if holes_here => {
            form::hint(
                ui,
                "In a blueprint, this script runs once for each placement.",
            );
        }
        _ => {}
    }
    if let Some(holes) = cx.holes.take() {
        for request in holes.requests {
            changed |= apply_hole(&reader, draft, node, &path, request);
        }
    }
    changed
}

/// The fields of `module` as the game reads them, without those its kind's own parts of the card
/// edit.
fn module_fields(ui: &mut Ui, cx: &mut FormCx, module: &mut AreaModule, path: &str) -> bool {
    match module {
        AreaModule::Object(object) => {
            forms::of_type_without(ui, cx, object, path, &[field!(ObjectDef, scripts)])
        }
        AreaModule::SafeZone(zone) => forms::of_type(ui, cx, zone, path),
        AreaModule::Soundscape(soundscape) => {
            forms::of_type_without(ui, cx, soundscape, path, &[field!(SoundscapeDef, channels)])
        }
        AreaModule::Prop(prop) => forms::of_type(ui, cx, prop, path),
        AreaModule::Scatter(scatter) => forms::of_type(ui, cx, scatter, path),
        AreaModule::Light(light) => forms::of_type(ui, cx, light, path),
        AreaModule::Blueprint(placement) => forms::of_type_without(
            ui,
            cx,
            placement,
            path,
            &[field!(engine::systems::area::Placement, values)],
        ),
        AreaModule::Population(population) => forms::of_type(ui, cx, population, path),
        AreaModule::Resident(resident) => forms::of_type(ui, cx, resident, path),
        AreaModule::Script(script) => forms::of_type(ui, cx, script, path),
    }
    .is_some()
}

/// The scripts of the object at the scene node `node`, each as a card.
fn object_scripts(ui: &mut Ui, cx: &mut FormCx, object: &mut ObjectDef, node: &str) -> bool {
    let path = format!("{node}{}", field!(ObjectDef, scripts));
    let held = cx.runner.replace(RunnerKind::Object);
    let changed = crate::shell::row_form::scripts::script_cards(
        ui,
        cx,
        AreaDef::TABLE,
        RunnerKind::Object,
        &mut object.scripts,
        &path,
        FormAction::NewObjectScript {
            node: node.to_owned(),
        },
    );
    cx.runner = held;
    changed.is_some()
}

/// Makes a script run by the object at the scene node `node` of `bundle`, beside it and named after
/// the object, and names it in the object's scripts, as one step.
pub fn new_object_script(editor: &mut Editor, bundle: &str, node: &str) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let read = nodes::bundle_of(editor, bundle)
        .ok_or_else(|| format!("{} doesn't read", file_title(bundle)))?;
    let Some(AreaModule::Object(object)) =
        nodes::module_node(read.nodes(), node).map(|module| &module.module)
    else {
        return Err(format!("no object at {node} in {}", file_title(bundle)));
    };
    let name = object.name.clone().unwrap_or_else(|| "object".to_owned());
    let mut edit = ContentEdit::new(String::new());
    let answering = rows::answering_first(project, bundle, RunnerKind::Object, &HashSet::new())?;
    let (path, reader, script) = rows::new_script(
        &mut edit,
        project,
        bundle,
        &name,
        RunnerKind::Object,
        answering,
    )?;
    edit.label = format!("New {} run by {name}", file_title(&path));
    read.write_in(&mut edit, &reader, bundle, |draft| {
        if let Some(AreaModule::Object(object)) =
            nodes::module_node_mut(&mut draft.nodes, node).map(|module| &mut module.module)
        {
            object.scripts.push(ScriptModule {
                script,
                values: BTreeMap::new(),
            });
        }
    })?;
    editor.try_apply(edit)
}

/// The holes of a blueprint's module as its form shows them: each filled by its variable, and
/// every other field a variable of its type could fill.
fn form_holes(
    reader: &FolderReader,
    variables: &BTreeMap<String, Variable>,
    module: &AreaModule,
    holes: &scene::Holes,
    path: &str,
) -> FormHoles {
    let mut fields: std::collections::HashMap<String, FieldHole> = Default::default();
    if let Ok(json) = reader.to_json(module) {
        for field in schema::holeable_fields(schema::of_type::<AreaModule>(), &json) {
            fields.insert(
                format!("{path}{}", field.pointer),
                FieldHole::Open(field.kind),
            );
        }
    }
    for (pointer, name) in &holes.0 {
        fields.insert(format!("{path}{pointer}"), FieldHole::Filled(name.clone()));
    }
    FormHoles {
        fields,
        variables: variables
            .iter()
            .map(|(name, variable)| (name.clone(), variable.ty))
            .collect(),
        requests: Vec::new(),
    }
}

/// Makes a field of the module at `node` of `draft` a hole, fills it from another variable, or
/// fixes it to a value again, as `request` asks.
fn apply_hole(
    reader: &FolderReader,
    draft: &mut SceneDraft,
    node: &str,
    path: &str,
    request: HoleRequest,
) -> bool {
    let relative = |field: &str| field.strip_prefix(path).unwrap_or(field).to_owned();
    let Some(module) = nodes::module_node_mut(&mut draft.nodes, node) else {
        return false;
    };
    match request {
        HoleRequest::Make { field, kind } => {
            let pointer = relative(&field);
            let Ok(json) = reader.to_json(&module.module) else {
                return false;
            };
            let table = schema::holeable_fields(schema::of_type::<AreaModule>(), &json)
                .into_iter()
                .find(|holeable| holeable.pointer == pointer)
                .and_then(|holeable| holeable.table);
            let default = json
                .pointer(&pointer)
                .and_then(|value| reader.from_json::<VarValue>(value).ok())
                .map(|value| match (kind, value) {
                    (VariableType::Number, VarValue::Integer(whole)) => {
                        VarValue::Number(whole as f64)
                    }
                    (_, value) => value,
                });
            let base = pointer.rsplit('/').next().unwrap_or("value");
            let name = rows::free_name(base, &draft.variables.keys().cloned().collect());
            module.holes.0.insert(pointer, name.clone());
            draft.variables.insert(
                name,
                Variable {
                    ty: kind,
                    table,
                    default,
                },
            );
            true
        }
        HoleRequest::Use { field, name } => {
            module.holes.0.insert(relative(&field), name);
            true
        }
        HoleRequest::Fix { field, name } => {
            let pointer = relative(&field);
            module.holes.0.remove(&pointer);
            if let Some(default) = draft
                .variables
                .get(&name)
                .and_then(|variable| variable.default.as_ref())
                .and_then(|default| reader.to_json(default).ok())
                && let Ok(mut json) = reader.to_json(&module.module)
                && let Some(at) = json.pointer_mut(&pointer)
            {
                *at = default;
                if let Ok(fixed) = reader.from_json(&json) {
                    module.module = fixed;
                }
            }
            if !scene::holes(&draft.nodes).contains(&name) {
                draft.variables.remove(&name);
            }
            true
        }
    }
}

#[derive(Default)]
pub(super) struct Followups {
    pub actions: Vec<super::AreaAction>,
}

fn object_extras(
    ui: &mut Ui,
    object: &ObjectDef,
    paths: &impl IdPaths,
    out: &mut Followups,
    node: &str,
) {
    let destinations = placement::warp_destinations(object, paths);
    if destinations.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (area, at) in &destinations {
            let where_ = format!("{area} · {}, {}", forms::metres(at.x), forms::metres(at.y));
            if ui
                .button("→ Go to destination")
                .on_hover_text(where_)
                .clicked()
            {
                out.actions.push(super::AreaAction::GoTo(area.clone(), *at));
            }
        }
        let hint = "An object with script/warp leading back here, on the nearest walkable cell outside the destination";
        if ui.button("← Return door").on_hover_text(hint).clicked() {
            out.actions.push(super::AreaAction::ReturnDoor(node.to_owned()));
        }
    });
}

/// The labels of the folders above the node at `pointer`, outermost first.
fn folders_above(scene: &[SceneNode], pointer: &str, paths: &impl IdPaths) -> Vec<String> {
    let mut labels: Vec<String> = std::iter::successors(nodes::place_of(pointer), |(folder, _)| {
        nodes::place_of(folder)
    })
    .map(|(folder, _)| folder)
    .take_while(|folder| !folder.is_empty())
    .filter_map(|folder| {
        nodes::node(scene, folder).map(|node| super::shapes::node_label(node, paths))
    })
    .collect();
    labels.reverse();
    labels
}

/// Which folders the modules at `pointers` sit in, said in a line.
fn spans(scene: &[SceneNode], pointers: &[String], paths: &impl IdPaths) -> String {
    let mut folders: Vec<String> = Vec::new();
    for pointer in pointers {
        let folder = nodes::place_of(pointer).map_or("", |(folder, _)| folder);
        let label = match folder {
            "" => "the top level".to_owned(),
            folder => nodes::node(scene, folder)
                .map(|node| super::shapes::node_label(node, paths))
                .unwrap_or_default(),
        };
        if !folders.contains(&label) {
            folders.push(label);
        }
    }
    let shown = match folders.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] if folders.len() <= 3 => format!("{} and {last}", first.join(", ")),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more folders", rest.len()),
    };
    format!("In {shown}")
}
