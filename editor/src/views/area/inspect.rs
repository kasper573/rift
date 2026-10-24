use std::collections::{BTreeMap, HashSet};
use ui::chip;
use ui::controls;
use ui::egui;
use ui::egui::{Align, Layout, RichText, Sense, Ui, Vec2};
use ui::form;
use ui::glyph::{self, Mark};
use ui::paint;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::math::{Pos, Rect, Rng, Size};
use engine::core::script::VariableType;
use engine::core::units::Meters;
use engine::field;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::object::ObjectDef;
use engine::systems::area::scene::{self, SceneNode, scene_file};
use engine::systems::area::{self, AreaDef, AreaModule, MapSize, Scenery, SoundscapeDef};
use engine::systems::script::{RunnerKind, ScriptModule, VarValue, Variable};
use serde_json::Value;

use super::placement::{self, placement_extras};
use super::population::from_npc;
use super::preview::{clipboard_panel, pick_preview};
use super::scenery::{scenery_extras, scenery_rows};
use super::soundscape::soundscape_extras;
use super::{AreaTool, AreaViewState, brush, grid, nodes, scene_tree, stamp, terrain};
use crate::app::Editor;
use crate::forms::{self, FieldHole, FormAction, FormCx, FormHoles, HoleRequest};
use crate::keymap::EditorKey;
use crate::project::{ContentEdit, IdPaths, RowSelection};
use crate::shell::{row_form, widgets};
use crate::tabs::file_title;
use crate::user::BrushSettings;
use crate::{picker, rows, schema};
use ui::input::InputField;

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
        [walk] if walk == super::WALK_ROW => {
            walk_card(editor, ui, view_key, &snapshot);
            overlays_legend(ui);
        }
        [layer] if node(layer).is_some_and(nodes::is_layer) => {
            layer_card(editor, ui, &snapshot, layer);
            ui.add_space(6.0);
            tool(editor, ui, view_key, &snapshot);
        }
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
        AreaTool::Paint | AreaTool::Fill | AreaTool::Erase | AreaTool::Eyedropper => {
            paint_card(editor, ui, view_key);
        }
        AreaTool::Terrain => terrain_card(editor, ui, view_key, snapshot),
        AreaTool::Stamp => stamp_card(editor, ui, view_key, &content),
        AreaTool::Select | AreaTool::Module => form::nothing_selected(
            ui,
            "Select nodes in the scene or on the map, or take a tool with settings, to inspect them here.",
        ),
    }
}

/// A ground layer's card: its name, where it draws among the others, what it holds, and its eye
/// and padlock.
fn layer_card(editor: &mut Editor, ui: &mut Ui, snapshot: &super::AreaSnapshot, pointer: &str) {
    let Some(name) = nodes::node(snapshot.nodes(), pointer)
        .and_then(nodes::layer_name)
        .map(str::to_owned)
    else {
        return;
    };
    let order = super::layers::names(snapshot.nodes());
    let at = order.iter().position(|layer| *layer == name).unwrap_or(0);
    let (under, over) = (&order[..at], &order[at + 1..]);
    let painted = grid::GridDraft::own(editor, &snapshot.bundle)
        .and_then(|draft| {
            let layer = draft.layer_index(&name)?;
            Some(draft.ground[layer].cells.iter().flatten().count())
        })
        .unwrap_or(0);
    let flags = snapshot.flags.row(pointer);
    let mut toggled = None;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state = if flags.hidden.on() { "hidden" } else { "" };
        panel::header(ui, &format!("ground layer {name}"), state, |_| {});
        ui.add_space(4.0);
        form::kv_text(ui, "file", format!("layers/{name}.csv"));
        let listed = |names: &[String]| match names {
            [] => "nothing".to_owned(),
            names => names.join(" and "),
        };
        form::kv_text(
            ui,
            "draws",
            format!("over {}, under {}", listed(under), listed(over)),
        );
        form::kv_text(ui, "painted", format!("{painted} cells"));
        form::kv(ui, "shown", |ui| {
            if ui.checkbox(&mut !flags.hidden.on(), "").clicked() {
                toggled = Some(scene_tree::Toggle::Hide);
            }
        });
        form::kv(ui, "locked", |ui| {
            if ui.checkbox(&mut flags.locked.on(), "").clicked() {
                toggled = Some(scene_tree::Toggle::Lock);
            }
        });
    });
    if let Some(toggle) = toggled {
        scene_tree::toggle(editor, &snapshot.bundle, &[pointer.to_owned()], toggle);
    }
}

/// A folder's card: what it holds by kind, its eye and padlock, and what can be made of it.
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

fn paint_card(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let title = match editor.views.area.view(view_key).tool {
        AreaTool::Fill => "Fill",
        AreaTool::Erase => "Erase",
        AreaTool::Eyedropper => "Eyedropper",
        _ => "Paint",
    };
    let mut brush = editor.settings.brush.clone();
    let mut changed = false;
    let picked = {
        let view = editor.views.area.view(view_key);
        if view.pick.random.is_empty() {
            view.pick.block.iter().map(Vec::len).sum::<usize>()
        } else {
            view.pick.random.len()
        }
    };
    tool_card(ui, title, |ui| {
        form::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, true);
        });
        form::kv(ui, "random", |ui| {
            changed |= ui.checkbox(&mut brush.random, "").changed();
            ui.label(
                theme::muted(if brush.random {
                    format!("from the {picked} selected tiles")
                } else {
                    "the picked tile or block".to_owned()
                })
                .size(12.5),
            );
        });
        form::kv(ui, "flip", |ui| {
            if flip_chip(ui, "X", EditorKey::FlipHorizontal, brush.flip_x) {
                brush.flip_x = !brush.flip_x;
                changed = true;
            }
            if flip_chip(ui, "Y", EditorKey::FlipVertical, brush.flip_y) {
                brush.flip_y = !brush.flip_y;
                changed = true;
            }
        });
    });
    if changed {
        editor.set_brush(brush);
    }
}

fn flip_chip(ui: &mut Ui, axis: &str, key: EditorKey, on: bool) -> bool {
    let (fill, border) = if on {
        (theme::ACCENT_SOFT, theme::ACCENT)
    } else {
        (theme::RAISED, theme::BORDER)
    };
    let response = chip::holding(ui, fill, border, |ui| {
        ui.label(RichText::new(axis).color(theme::TEXT).size(13.0));
        controls::key_hint(ui, &key.name());
    })
    .response
    .interact(Sense::click());
    response.clicked()
}

fn terrain_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &super::AreaSnapshot) {
    let content = &snapshot.content;
    let tile_px = snapshot.area().tile;
    let sets = terrain::sets_for(content, tile_px);
    let mut brush = editor.settings.brush.clone();
    let mut changed = false;
    tool_card(ui, "Terrain brush", |ui| {
        let view = editor.views.area.view_mut(view_key);
        form::kv(ui, "terrains", |ui| {
            let current = view
                .terrain
                .set
                .clone()
                .unwrap_or_else(|| "pick a set".to_owned());
            controls::combo((view_key, "terrain-set"))
                .selected_text(theme::mono(current))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for set in &sets {
                        let picked = view.terrain.tileset.as_deref() == Some(&set.tileset)
                            && view.terrain.set.as_deref() == Some(&set.set);
                        if ui
                            .selectable_label(
                                picked,
                                theme::mono({
                                    let mixes = set.coverage(content);
                                    format!(
                                        "{} · {} · {} of {}",
                                        set.set, set.tileset, mixes.have, mixes.total
                                    )
                                }),
                            )
                            .clicked()
                        {
                            view.terrain = brush::TerrainPick {
                                tileset: Some(set.tileset.clone()),
                                set: Some(set.set.clone()),
                                terrain: 0,
                            };
                        }
                    }
                });
        });
        if let Some(picked) = sets.iter().find(|set| {
            view.terrain.tileset.as_deref() == Some(&set.tileset)
                && view.terrain.set.as_deref() == Some(&set.set)
        }) {
            form::kv(ui, "paint", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (index, terrain) in picked.terrains.iter().enumerate() {
                        let on = view.terrain.terrain == index;
                        if chip::mode(ui, terrain, on).clicked() {
                            view.terrain.terrain = index;
                        }
                    }
                });
            });
        }
        form::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, false);
        });
    });
    if changed {
        editor.set_brush(brush);
    }
    sheet::section(ui, "Terrains of these tilesets");
    let used = snapshot.picture.ground.tilesets();
    let mut own: Vec<&terrain::TerrainSet> = sets
        .iter()
        .filter(|set| used.contains(&set.tileset))
        .collect();
    let others = sets.len() - own.len();
    if own.is_empty() {
        ui.label(theme::muted("None of this area's tilesets names terrains.").size(12.5));
    }
    own.truncate(6);
    for set in own {
        ui.horizontal(|ui| {
            glyph::show(ui, Mark::Grid, 12.0, theme::FAINT);
            ui.label(theme::mono(&set.set).size(13.0));
            ui.label(theme::muted(set.terrains.join(" · ")).size(12.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mixes = set.coverage(content);
                ui.label(
                    theme::muted(format!("{} of {}", mixes.have, mixes.total))
                        .monospace()
                        .size(12.0),
                );
            });
        })
        .response
        .on_hover_text(&set.tileset);
    }
    if others > 0 {
        ui.label(theme::faint(format!("… {others} more in other tilesets")).size(12.5));
    }
    let picked = {
        let view = editor.views.area.view(view_key);
        view.terrain.tileset.clone().zip(view.terrain.set.clone())
    };
    let layer = grid::GridDraft::own(editor, &snapshot.bundle)
        .and_then(|draft| brush::target_layer(editor, snapshot, &draft));
    if let (Some((tileset, set)), Some(layer)) = (picked, layer) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("Repaint with {set}"))
                .on_hover_text("Re-picks a tile for every cell of this layer painted from the set, as the set is now")
                .clicked()
            {
                repaint_layer(editor, content, &snapshot.bundle, &tileset, &set, layer);
            }
        });
    }
}

fn repaint_layer(
    editor: &mut Editor,
    content: &Content,
    key: &str,
    tileset: &str,
    set: &str,
    layer: usize,
) {
    let Some(def) = content
        .by_path::<area::tileset::TilesetDef>(tileset)
        .map(|id| id.get(content))
    else {
        return;
    };
    let Some(mut draft) = grid::GridDraft::own(editor, key) else {
        return;
    };
    let mut rng = Rng::new(editor.frame);
    let (changed, kept) = super::terrain::repaint(&mut draft, layer, def, tileset, set, &mut rng);
    if changed == 0 {
        editor.toast(format!(
            "Nothing to repaint: no cell of this layer is painted from {set}"
        ));
        return;
    }
    draft.commit(
        editor,
        format!("Repaint {changed} cells with {set} in {key}"),
    );
    if kept > 0 {
        editor.toast(format!("{kept} cells kept: no tile for their mix"));
    }
}

fn walk_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &super::AreaSnapshot) {
    let mut brush = editor.settings.brush.clone();
    let mut changed = false;
    let forced = snapshot
        .picture
        .ground
        .walk
        .iter()
        .filter(|cell| cell.is_some())
        .count();
    tool_card(ui, "Paint walkability", |ui| {
        form::kv_text(ui, "forced", format!("{forced} cells"));
        let view = editor.views.area.view_mut(view_key);
        form::kv(ui, "paint", |ui| {
            for (value, label, color) in [
                (Some(false), "blocked", theme::DANGER),
                (Some(true), "walkable", theme::OK),
                (None, "clear", theme::MUTED),
            ] {
                let on = view.walk_value == value;
                if chip::colored(
                    ui,
                    label,
                    if on {
                        ui::ink::wash(color, 0.2)
                    } else {
                        theme::INPUT
                    },
                    color,
                )
                .clicked()
                {
                    view.walk_value = value;
                }
            }
        });
        form::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, true);
        });
        form::kv(ui, "overlay", |ui| {
            ui.checkbox(&mut view.walk_overlay, "");
            controls::key_hint_text(ui, &EditorKey::WalkOverlay.name(), "toggles");
        });
        form::kv(ui, "locked", |ui| {
            ui.checkbox(&mut view.walk_locked, "");
        });
    });
    if changed {
        editor.set_brush(brush);
    }
}

fn overlays_legend(ui: &mut Ui) {
    sheet::section(ui, "Overlays");
    let swatch = |ui: &mut Ui, color: egui::Color32, hatch: bool| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
        if hatch {
            paint::hatched(ui.painter(), rect, color, 4.0);
        } else {
            ui.painter().rect_filled(rect.shrink(3.0), 1.0, color);
        }
    };
    ui.horizontal(|ui| {
        swatch(ui, theme::DANGER, true);
        ui.label(theme::muted("blocked").size(12.5));
    });
    ui.horizontal(|ui| {
        swatch(ui, theme::OK, false);
        ui.label(theme::muted("painted walkable over a blocking tile").size(12.5));
    });
    ui.horizontal(|ui| {
        swatch(ui, theme::WARN, false);
        ui.label(theme::muted("surface: a band per footstep sound").size(12.5));
    });
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("– ·")
                .color(super::overlays::SORT_LINE)
                .size(12.5),
        );
        ui.label(theme::muted("sort lines").size(12.5));
    });
}

fn stamp_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, content: &Content) {
    let stamp = editor.views.area.view(view_key).stamp.clone();
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
                    editor.views.area.view_mut(view_key).stamp = Some(picked.clone());
                }
            });
        }
        for action in actions {
            row_form::act(editor, action, AreaDef::TABLE, "");
        }
        let Some(stamp) = editor.views.area.view(view_key).stamp.clone() else {
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
        form::kv(ui, "flip", |ui| {
            if flip_chip(ui, "X", EditorKey::FlipHorizontal, settings.flip_x) {
                settings.flip_x = !settings.flip_x;
                save = true;
            }
            if flip_chip(ui, "Y", EditorKey::FlipVertical, settings.flip_y) {
                settings.flip_y = !settings.flip_y;
                save = true;
            }
        });
        form::kv(ui, "spacing", |ui| {
            let label = match settings.spacing {
                None => format!("footprint · {} × {}", footprint.0, footprint.1),
                Some(n) => format!("{n} tiles"),
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
                                theme::mono(format!("{n} tiles")),
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
            ui.label(theme::muted("whole tiles").size(12.5));
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
    let mut blueprint = read.blueprint().clone();
    let selected: Vec<String> = selection
        .parts
        .iter()
        .filter(|part| nodes::module_node(&blueprint.scene.nodes, part).is_some())
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
            label = nodes::module_node(&blueprint.scene.nodes, node)
                .map(|module| super::shapes::label(&module.module, reader.as_ref()))
                .unwrap_or_default();
            changed = module_inspector(ui, &mut cx, view, &bundle, &mut blueprint, node, &mut out);
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
            ui.label(
                theme::muted(spans(&blueprint.scene.nodes, &selected, reader.as_ref())).size(12.5),
            );
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
                changed |=
                    placement::shared_values(ui, &mut cx, &mut blueprint.scene.nodes, &selected);
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
        match read.write(&mut edit, project, &bundle, |own| *own = blueprint) {
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

/// The card of the module at `node` of `blueprint`, the area's or blueprint's at `bundle`: its
/// fields as the game reads them, a blueprint's holes among them, and what its kind adds.
fn module_inspector(
    ui: &mut Ui,
    cx: &mut FormCx,
    view: Option<&mut AreaViewState>,
    bundle: &str,
    blueprint: &mut BlueprintDef,
    node: &str,
    out: &mut Followups,
) -> bool {
    let path = node.to_owned();
    let reader = cx.project.reader();
    let Some(module) = nodes::module_node(&blueprint.scene.nodes, node) else {
        return false;
    };
    let kind = module.module.kind();
    let shown_kind = kind.tag().replace('_', " ");
    let crumbs: Vec<String> = std::iter::once(file_title(bundle).to_owned())
        .chain(folders_above(&blueprint.scene.nodes, node, reader.as_ref()))
        .chain(std::iter::once(shown_kind.clone()))
        .collect();
    ui.label(theme::muted(crumbs.join(" › ")).monospace().size(13.0));
    ui.add_space(4.0);
    let holes_here = table_of(bundle) == Some(BlueprintDef::TABLE);
    if holes_here {
        cx.holes = Some(form_holes(
            &reader,
            &blueprint.variables,
            &module.module,
            &module.holes,
            &path,
        ));
    }
    let map = blueprint.map.size;
    let Some(module) = nodes::module_node_mut(&mut blueprint.scene.nodes, node) else {
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
            *roams = whole_map(map);
        }
        match &mut module.module {
            AreaModule::Scenery(scenery) => changed |= scenery_rows(ui, scenery, &path),
            AreaModule::Population(_) => {
                if let Some(view) = view {
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
            }
            _ => {}
        }
    });
    match &mut module.module {
        AreaModule::Scenery(scenery) => scenery_extras(ui, cx, scenery, out, node),
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
            changed |= apply_hole(&reader, blueprint, node, &path, request);
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
        AreaModule::Scenery(scenery) => forms::of_type_without(
            ui,
            cx,
            scenery,
            path,
            &[
                field!(Scenery, tiles),
                field!(Scenery, sort),
                field!(Scenery, blocks),
            ],
        ),
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
    read.write_in(&mut edit, &reader, bundle, |blueprint| {
        if let Some(AreaModule::Object(object)) =
            nodes::module_node_mut(&mut blueprint.scene.nodes, node)
                .map(|module| &mut module.module)
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

/// Makes a field of the module at `node` of `blueprint` a hole, fills it from another variable, or
/// fixes it to a value again, as `request` asks.
fn apply_hole(
    reader: &FolderReader,
    blueprint: &mut BlueprintDef,
    node: &str,
    path: &str,
    request: HoleRequest,
) -> bool {
    let relative = |field: &str| field.strip_prefix(path).unwrap_or(field).to_owned();
    let Some(module) = nodes::module_node_mut(&mut blueprint.scene.nodes, node) else {
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
            let name = rows::free_name(base, &blueprint.variables.keys().cloned().collect());
            module.holes.0.insert(pointer, name.clone());
            blueprint.variables.insert(
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
            if let Some(default) = blueprint
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
            if !scene::holes(&blueprint.scene.nodes).contains(&name) {
                blueprint.variables.remove(&name);
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

fn brush_size_field(ui: &mut Ui, brush: &mut BrushSettings, square: bool) -> bool {
    let mut changed = false;
    let unit = match (square, brush.size) {
        (true, _) => "tiles",
        (false, 1) => "corner",
        (false, _) => "corners",
    };
    let mut text = if square {
        format!("{} × {}", brush.size, brush.size)
    } else {
        brush.size.to_string()
    };
    let response = InputField::new(&mut text).unit(unit).show(ui);
    if response.changed()
        && let Some(first) = text
            .split(['×', 'x', ' '])
            .next()
            .and_then(|n| n.trim().parse::<u32>().ok())
    {
        let size = first.clamp(1, 9);
        if size != brush.size {
            brush.size = size;
            changed = true;
        }
    }
    changed
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

/// The rect of every cell of a map of `size`: where a population roams when it is given no rect,
/// so the one it is first given.
fn whole_map(size: MapSize) -> Rect<Meters> {
    Rect::new(
        Pos::new(-0.5, -0.5),
        Size::new(size.width as f32, size.height as f32),
    )
}
