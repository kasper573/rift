use std::collections::HashSet;
use std::str::FromStr;

use egui::{Align, Layout, RichText, Sense, Ui, Vec2};
use engine::core::content::Content;
use engine::core::math::Rng;
use engine::core::script::VariableType;
use engine::core::tiling::{Cell, CellPos, TileSize};
use engine::systems::area::{self, AreaId, AreaModuleKind};
use serde_json::Value;

use super::pins::INITIAL_SPAWN;
use super::placement::placement_extras;
use super::population::from_npc;
use super::preview::{clipboard_panel, pick_preview};
use super::scenery::{scenery_extras, scenery_rows};
use super::soundscape::soundscape_extras;
use super::{AreaTool, pins};
use super::{AreaViewState, grid, terrain};
use super::{brush, placement, resize, stamp};
use crate::app::{Editor, ModalDialog, PlacePick};
use crate::drafts::{clear_draft, draft_or, set_draft};
use crate::forms::FormCx;
use crate::project::{ContentEdit, GAME_FILE, RowSelection, row_file};
use crate::tabs::EditorTab;
use crate::ui::{inspector, patterns, theme, widgets};
use crate::user::BrushSettings;
use crate::value::RowValue;
use crate::{forms, picker, schema, value, views};

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let view_key = super::area_view_key(key);
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
    else {
        return;
    };
    let Some(area_id) = content.by_key::<area::AreaDef>(key) else {
        return;
    };
    if !editor.views.area.views.contains_key(&view_key) {
        return;
    }
    let tool = editor.views.area.view(&view_key).tool;
    let walk = editor.views.area.view(&view_key).walk_overlay;
    if editor.views.area.view(&view_key).paste.is_some() {
        clipboard_panel(editor, ui);
        return;
    }
    if editor.pick.is_some() {
        pick_preview(editor, ui);
        return;
    }
    ui.label(theme::muted(format!("area/{key}")).monospace().size(13.0));
    ui.add_space(4.0);
    match tool {
        AreaTool::Paint if walk => {
            walk_card(editor, ui, &view_key);
            overlays_legend(ui);
        }
        AreaTool::Paint | AreaTool::Fill | AreaTool::Erase | AreaTool::Eyedropper => {
            paint_card(editor, ui, &view_key, &content, area_id);
            palette_summary(ui, &content, area_id);
        }
        AreaTool::Terrain => {
            terrain_card(editor, ui, &view_key, &content, area_id);
        }
        AreaTool::Stamp => {
            stamp_card(editor, ui, &view_key, &content);
        }
        AreaTool::Module => {
            area_card(editor, ui, key, &content, area_id);
            module_counts(editor, ui, &view_key, key);
        }
        AreaTool::Select => {
            area_card(editor, ui, key, &content, area_id);
            spawn_card(editor, ui, key, &content);
            ways_in(editor, ui, key);
        }
    }
}

fn tool_card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::strong(title).monospace());
        });
        ui.add_space(4.0);
        body(ui);
    });
}

fn paint_card(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    content: &Content,
    area_id: AreaId,
) {
    let title = match editor.views.area.view(view_key).tool {
        AreaTool::Fill => "Fill",
        AreaTool::Erase => "Erase",
        AreaTool::Eyedropper => "Eyedropper",
        _ => "Paint",
    };
    let layers = area::load(content, area_id).grids.layer_names();
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
        let view = editor.views.area.view_mut(view_key);
        widgets::kv(ui, "layer", |ui| {
            layer_combo(ui, view, &layers, (view_key, "layer"))
        });
        widgets::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, true);
        });
        widgets::kv(ui, "random", |ui| {
            changed |= widgets::toggle(ui, &mut brush.random).changed();
            ui.label(
                theme::muted(if brush.random {
                    format!("from the {picked} selected tiles")
                } else {
                    "the picked tile or block".to_owned()
                })
                .size(12.5),
            );
        });
        widgets::kv(ui, "flip", |ui| {
            if flip_chip(ui, "X", brush.flip_x) {
                brush.flip_x = !brush.flip_x;
                changed = true;
            }
            if flip_chip(ui, "Y", brush.flip_y) {
                brush.flip_y = !brush.flip_y;
                changed = true;
            }
        });
    });
    if changed {
        editor.set_brush(brush);
    }
}

fn flip_chip(ui: &mut Ui, axis: &str, on: bool) -> bool {
    let fill = if on { theme::ACCENT } else { theme::INPUT };
    let ink = if on {
        egui::Color32::WHITE
    } else {
        theme::TEXT
    };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(
            1.0,
            if on { theme::ACCENT } else { theme::BORDER },
        ))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(axis).color(ink).size(13.0));
                widgets::key_hint(ui, axis);
            });
        })
        .response
        .interact(Sense::click());
    response.clicked()
}

fn palette_summary(ui: &mut Ui, content: &Content, area_id: AreaId) {
    let grids = &area::load(content, area_id).grids;
    let tilesets: HashSet<&String> = grids.palette_keys.iter().map(|row| &row.tileset).collect();
    widgets::section(ui, "Palette of this area");
    widgets::hint(
        ui,
        &format!(
            "{} tiles from {} tilesets",
            widgets::thousands(grids.palette.len()),
            tilesets.len()
        ),
    );
}

fn terrain_card(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    content: &Content,
    area_id: AreaId,
) {
    let tile_px = area::load(content, area_id).tile;
    let layers = area::load(content, area_id).grids.layer_names();
    let sets = terrain::sets_for(content, tile_px);
    let mut brush = editor.settings.brush.clone();
    let mut changed = false;
    tool_card(ui, "Terrain brush", |ui| {
        let view = editor.views.area.view_mut(view_key);
        widgets::kv(ui, "layer", |ui| {
            layer_combo(ui, view, &layers, (view_key, "terrain-layer"))
        });
        widgets::kv(ui, "terrains", |ui| {
            let current = view
                .terrain
                .set
                .clone()
                .unwrap_or_else(|| "pick a set".to_owned());
            widgets::combo_box((view_key, "terrain-set"))
                .selected_text(theme::mono(current))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for set in &sets {
                        let picked = view.terrain.tileset.as_deref() == Some(&set.tileset)
                            && view.terrain.set.as_deref() == Some(&set.set);
                        if ui
                            .selectable_label(
                                picked,
                                theme::mono(format!(
                                    "{} · {} · {} of {}",
                                    set.set, set.tileset, set.have, set.total
                                )),
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
            widgets::kv(ui, "paint", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (index, terrain) in picked.terrains.iter().enumerate() {
                        let on = view.terrain.terrain == index;
                        if widgets::mode_chip(ui, terrain, on).clicked() {
                            view.terrain.terrain = index;
                        }
                    }
                });
            });
        }
        widgets::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, false);
        });
    });
    if changed {
        editor.set_brush(brush);
    }
    widgets::section(ui, "Terrains of this area's tilesets");
    let used = area::load(content, area_id).grids.tilesets();
    let mut own: Vec<&terrain::TerrainSetCoverage> = sets
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
            ui.label(theme::faint("▣").size(12.0));
            ui.label(theme::mono(&set.set).size(13.0));
            ui.label(theme::muted(set.terrains.join(" · ")).size(12.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    theme::muted(format!("{} of {}", set.have, set.total))
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
        view.terrain
            .tileset
            .clone()
            .zip(view.terrain.set.clone())
            .map(|(tileset, set)| (tileset, set, view.layer))
    };
    if let Some((tileset, set, layer)) = picked {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("▣ Repaint with {set}"))
                .on_hover_text("Re-picks a tile for every cell of this layer painted from the set, as the set is now")
                .clicked()
            {
                repaint_layer(editor, content, area_id, &tileset, &set, layer);
            }
        });
    }
}

fn repaint_layer(
    editor: &mut Editor,
    content: &Content,
    area_id: AreaId,
    tileset: &str,
    set: &str,
    layer: usize,
) {
    let Some(def) = content
        .by_key::<area::tileset::TilesetDef>(tileset)
        .map(|id| id.get(content))
    else {
        return;
    };
    let area = area::load(content, area_id);
    let key = area_id.key(content).to_owned();
    let mut draft = grid::GridDraft::of(&key, &area.grids);
    let mut rng = Rng::new(editor.frame);
    let (changed, kept) = super::terrain::repaint(&mut draft, layer, def, tileset, set, &mut rng);
    if changed == 0 {
        editor.toast(format!(
            "Nothing to repaint: no cell of this layer is painted from {set}"
        ));
        return;
    }
    let mut edit = ContentEdit::new(format!("Repaint {changed} cells with {set} in {key}"));
    draft.write_into(&mut edit);
    editor.apply(edit);
    if kept > 0 {
        editor.toast(format!("{kept} cells kept: no tile for their mix"));
    }
}

fn walk_card(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let mut brush = editor.settings.brush.clone();
    let mut changed = false;
    tool_card(ui, "Paint walkability", |ui| {
        let view = editor.views.area.view_mut(view_key);
        widgets::kv(ui, "paint", |ui| {
            for (value, label, color) in [
                (Some(false), "blocked", theme::DANGER),
                (Some(true), "walkable", theme::OK),
                (None, "clear", theme::MUTED),
            ] {
                let on = view.walk_value == value;
                if widgets::chip_colored(
                    ui,
                    label,
                    if on { color } else { theme::INPUT },
                    if on { egui::Color32::WHITE } else { color },
                )
                .clicked()
                {
                    view.walk_value = value;
                }
            }
        });
        widgets::kv(ui, "brush", |ui| {
            changed |= brush_size_field(ui, &mut brush, true);
        });
        widgets::kv(ui, "overlay", |ui| {
            widgets::toggle(ui, &mut view.walk_overlay);
            widgets::key_hint_text(ui, "W", "toggles");
        });
    });
    if changed {
        editor.set_brush(brush);
    }
}

fn overlays_legend(ui: &mut Ui) {
    widgets::section(ui, "Overlays");
    let swatch = |ui: &mut Ui, color: egui::Color32, hatch: bool| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
        if hatch {
            patterns::hatch(ui.painter(), rect, color, 4.0);
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
        ui.label(RichText::new("– ·").color(theme::SORT_LINE).size(12.5));
        ui.label(theme::muted("sort lines").size(12.5));
    });
}

fn stamp_card(editor: &mut Editor, ui: &mut Ui, view_key: &str, content: &Content) {
    let stamp = editor.views.area.view(view_key).stamp.clone();
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
            let mut cx =
                FormCx::new(project, &root, shapes, form, &mut actions).owned_by("area", None);
            widgets::kv(ui, "blueprint", |ui| {
                if picker::row(ui, &mut cx, "blueprint", &mut picked, "/stamp/blueprint")
                    && !picked.is_empty()
                {
                    editor.views.area.view_mut(view_key).stamp = Some(picked.clone());
                }
            });
        }
        for action in actions {
            inspector::act(editor, action, "area", "");
        }
        let Some(stamp) = editor.views.area.view(view_key).stamp.clone() else {
            widgets::hint(
                ui,
                "Pick a blueprint below, then click to place one or drag a line.",
            );
            return;
        };
        let footprint = {
            let values = stamp::stamp_values(editor, Some(&stamp));
            let size = stamp::stamp_footprint(editor, content, &stamp, &values);
            (
                size.width.round().max(1.0) as u32,
                size.height.round().max(1.0) as u32,
            )
        };
        let settings = editor.settings.stamps.entry(stamp.clone()).or_default();
        widgets::kv(ui, "flip", |ui| {
            if flip_chip(ui, "X", settings.flip_x) {
                settings.flip_x = !settings.flip_x;
                save = true;
            }
            if flip_chip(ui, "Y", settings.flip_y) {
                settings.flip_y = !settings.flip_y;
                save = true;
            }
        });
        widgets::kv(ui, "spacing", |ui| {
            let label = match settings.spacing {
                None => format!("footprint · {} × {}", footprint.0, footprint.1),
                Some(n) => format!("{n} tiles"),
            };
            widgets::combo_box((view_key, "spacing"))
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
        widgets::kv(ui, "snap", |ui| {
            ui.label(theme::muted("whole tiles").size(12.5));
            widgets::key_hint_text(ui, "Alt", "for 1/16");
        });
        let variables = content
            .by_key::<area::blueprint::BlueprintDef>(&stamp)
            .map(|id| id.get(content).variables.clone())
            .unwrap_or_default();
        if variables.is_empty() {
            widgets::kv_text(ui, "values", "none: no variables");
        } else {
            let mut values = Value::Object(settings.values.clone().into_iter().collect());
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
                let mut cx =
                    FormCx::new(project, &root, shapes, form, &mut actions).owned_by("area", None);
                for (name, variable) in &variables {
                    widgets::kv(ui, name, |ui| {
                        let mut value = values.get(name).cloned().unwrap_or_else(|| {
                            variable
                                .default
                                .as_ref()
                                .and_then(|d| value::json(d).ok())
                                .unwrap_or_else(|| placement::variable_template(variable))
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
                        if edited || (values.get(name).is_none() && variable.default.is_none()) {
                            values[name] = value;
                            values_changed = true;
                        }
                    });
                }
            }
            if values_changed {
                let settings = editor.settings.stamps.entry(stamp.clone()).or_default();
                settings.values = values
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                save = true;
            }
        }
    });
    if save {
        editor.save_settings();
    }
}

fn area_card(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, area_id: AreaId) {
    let Some(mut row) = editor.row_value("area", key) else {
        return;
    };
    let area = area::load(content, area_id);
    let name_id = ui.id().with(("area-name", key));
    let mut draft = draft_or(
        ui,
        name_id,
        row["name"].as_str().unwrap_or_default().to_owned(),
    );
    let mut rename = None;
    let mut menu_action: Option<AreaMenu> = None;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, row["name"].as_str().unwrap_or(key), "", |ui| {
            if ui.button("Resize…").clicked() {
                menu_action = Some(AreaMenu::Resize);
                ui.close();
            }
            if ui
                .add(egui::Button::new("Play here").shortcut_text("F6"))
                .clicked()
            {
                menu_action = Some(AreaMenu::Play);
                ui.close();
            }
            if ui.button("Open as row").clicked() {
                menu_action = Some(AreaMenu::OpenRow);
                ui.close();
            }
        });
        ui.add_space(4.0);
        widgets::kv(ui, "name", |ui| {
            let response = forms::boxed_input(ui, None, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                )
            });
            if response.changed() {
                set_draft(ui, name_id, &draft.clone());
            }
            if forms::committed(ui, &response) {
                clear_draft(ui, name_id);
                if row["name"].as_str() != Some(draft.as_str()) {
                    rename = Some(draft.clone());
                }
            }
        });
        widgets::kv(ui, "size", |ui| {
            forms::boxed_input(ui, Some("tiles"), |ui| {
                ui.label(theme::mono(format!(
                    "{} × {}",
                    area.size.width, area.size.height
                )))
            });
        });
        widgets::kv(ui, "tile", |ui| {
            forms::boxed_input(ui, Some("px"), |ui| {
                ui.label(theme::mono(area.tile.to_string()))
            });
        });
        widgets::kv(ui, "ground", |ui| {
            forms::boxed_input(ui, None, |ui| {
                ui.label(theme::mono(format!(
                    "{} layers · under everyone",
                    area.grids.ground.len()
                )))
            });
        });
        widgets::kv_text(
            ui,
            "palette",
            format!(
                "{} tiles · kept by the editor",
                widgets::thousands(area.grids.palette.len())
            ),
        );
        widgets::kv_text(
            ui,
            "walk",
            if area.grids.walk.iter().any(Option::is_some) {
                "its own grid · painted"
            } else {
                "its own grid · nothing painted"
            },
        );
    });
    if let Some(name) = rename {
        row["name"] = Value::String(name);
        let mut edit = ContentEdit::new(format!("Rename area {key}"));
        edit.write_json(row_file("area", key), &row);
        editor.apply(edit);
    }
    match menu_action {
        Some(AreaMenu::Resize) => editor
            .dialogs
            .push(ModalDialog::Resize(resize::ResizeDialog {
                area: key.to_owned(),
                width: area.size.width as u32,
                height: area.size.height as u32,
                anchor: (-1, -1),
                error: None,
            })),
        Some(AreaMenu::Play) => views::play_here(editor),
        Some(AreaMenu::OpenRow) => editor.open_tab(EditorTab::Row {
            table: "area".to_owned(),
            key: key.to_owned(),
        }),
        None => {}
    }
}

enum AreaMenu {
    Resize,
    Play,
    OpenRow,
}

fn spawn_card(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content) {
    let Some((spawn_area, at)) = editor
        .project
        .as_ref()
        .and_then(|project| pins::initial_spawn_place(&project.files))
    else {
        return;
    };
    widgets::section(ui, "The game's spawn");
    let mut picked = spawn_area.clone();
    let mut actions = Vec::new();
    let mut go = false;
    let mut pick = false;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, "initial spawn");
        ui.add_space(4.0);
        {
            let Editor {
                project,
                form,
                shapes,
                ..
            } = editor;
            let project = project.as_ref().expect("project");
            let root = Value::Null;
            let mut cx = FormCx::new(project, &root, shapes, form, &mut actions);
            widgets::kv(ui, "initial spawn", |ui| {
                picker::row(
                    ui,
                    &mut cx,
                    "area",
                    &mut picked,
                    &format!("{INITIAL_SPAWN}/area"),
                );
            });
        }
        widgets::kv(ui, "at", |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button("⌖ pick")
                    .on_hover_text("Click a walkable cell on the area")
                    .clicked()
                {
                    pick = true;
                }
                forms::boxed_input(ui, Some("tiles"), |ui| {
                    ui.label(theme::mono(format!(
                        "{}, {}",
                        forms::tiles(at.x),
                        forms::tiles(at.y)
                    )))
                })
            });
        });
        if spawn_area != key && ui.button("→ Go to").clicked() {
            go = true;
        }
    });
    if picked != spawn_area
        && let Some(target) = content.by_key::<area::AreaDef>(&picked)
    {
        let grid = area::load(content, target).size.grid();
        let middle = CellPos::new((grid.width / 2) as i32, (grid.height / 2) as i32).center();
        pins::set_spawn(editor, &picked, middle);
    }
    for action in actions {
        inspector::act(editor, action, "area", key);
    }
    if pick {
        editor.pick = Some(PlacePick {
            file: GAME_FILE.to_owned(),
            pointer: INITIAL_SPAWN.to_owned(),
            area: Some(key.to_owned()),
        });
        editor.toast("Click a walkable cell to put the spawn there (Alt-click for any point)");
    }
    if go {
        super::open_area_at(editor, &spawn_area, Some(at), None);
    }
}

fn ways_in(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let arrivals: Vec<pins::ArrivalPin> = pins::arrivals(editor, key)
        .into_iter()
        .filter(|arrival| !arrival.initial_spawn)
        .collect();
    let name = editor
        .row_value("area", key)
        .and_then(|row| row["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| key.to_owned());
    widgets::section_count(ui, &format!("Ways into {name}"), arrivals.len());
    let mut open: Option<pins::ArrivalPin> = None;
    for arrival in arrivals.iter().take(3) {
        let (owner, spot) = arrival.names(editor);
        let spot = spot.unwrap_or_default();
        let response = ui
            .horizontal(|ui| {
                widgets::badge(ui, "place");
                ui.label(theme::mono(&owner).size(13.0).color(theme::CODE));
                if !spot.is_empty() {
                    ui.label(theme::faint("·"));
                    ui.label(theme::mono(&spot).size(13.0));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        theme::muted(format!(
                            "{}, {}",
                            forms::tiles(arrival.at.x),
                            forms::tiles(arrival.at.y)
                        ))
                        .monospace()
                        .size(12.0),
                    );
                });
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            open = Some(arrival.clone());
        }
    }
    if arrivals.len() > 3 {
        ui.label(
            theme::faint(format!("… {} more", arrivals.len() - 3))
                .monospace()
                .size(12.5),
        );
    }
    if let Some(arrival) = open {
        pins::open_pin(
            editor,
            pins::PinHit::Place {
                file: arrival.file,
                pointer: arrival.pointer,
            },
        );
    }
}

fn module_counts(editor: &mut Editor, ui: &mut Ui, view_key: &str, key: &str) {
    let Some(row) = editor.row_value("area", key) else {
        return;
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for module in row["modules"].as_array().into_iter().flatten() {
        let kind = module.kind_or("module").to_owned();
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((kind, 1)),
        }
    }
    counts.sort_by_key(|(kind, _)| {
        AreaModuleKind::from_str(kind).map_or((true, 0), |kind| (false, kind as usize))
    });
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    widgets::section_count(ui, "Modules", total);
    for (kind, count) in &counts {
        ui.horizontal(|ui| {
            widgets::badge(ui, kind);
            ui.label(theme::mono(kind).size(13.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::muted(count.to_string()).monospace().size(12.0));
            });
        });
    }
    ui.add_space(4.0);
    let response = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::strong("+ Add module"));
                widgets::key_hint(ui, "M");
            });
        })
        .response
        .interact(Sense::click());
    if response.clicked() {
        let view = editor.views.area.view_mut(view_key);
        view.set_tool(AreaTool::Module);
        editor.toast("Click the map to add a module there, or right-click anywhere");
    }
}

pub fn show_parts(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(area_key) = selection.keys.first().cloned() else {
        return;
    };
    let Some(mut row) = editor.row_value("area", &area_key) else {
        return;
    };
    let indices: Vec<usize> = selection
        .parts
        .iter()
        .filter_map(|part| super::module_index(part))
        .collect();
    if indices.is_empty() {
        return;
    }
    let shapes = editor.shapes.clone();
    let shape = shapes.shape_of("area");
    let mut changed = false;
    let mut actions = Vec::new();
    let mut out = Followups::default();
    let label;
    {
        let Editor {
            project,
            form,
            shapes,
            views,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, shape, shapes, form, &mut actions)
            .owned_by("area", Some(&area_key));
        if indices.len() == 1 {
            let index = indices[0];
            let view = views.area.views.get_mut(&super::area_view_key(&area_key));
            label = super::shapes::label(&row["modules"][index]);
            changed = module_inspector(ui, &mut cx, view, &area_key, &mut row, index, &mut out);
        } else {
            label = format!("{} modules", indices.len());
            ui.label(
                theme::muted(format!("area/{area_key} › {} modules", indices.len()))
                    .monospace()
                    .size(13.0),
            );
            ui.add_space(4.0);
            widgets::card_frame(true).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, &format!("{} modules", indices.len()), "", |ui| {
                    if ui.button("Copy as JSON").clicked() {
                        out.actions.push(super::AreaAction::Copy);
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        out.actions.push(super::AreaAction::Duplicate);
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Remove all").shortcut_text("Del"))
                        .clicked()
                    {
                        out.actions.push(super::AreaAction::Delete);
                        ui.close();
                    }
                });
                ui.add_space(4.0);
                changed |= placement::shared_values(ui, &mut cx, &mut row, &indices);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("⧉ Make blueprint…").clicked() {
                    out.actions.push(super::AreaAction::MakeBlueprint);
                }
            });
            widgets::key_hint_text(ui, "Ctrl Alt B", "makes a blueprint of the selection");
        }
    }
    if changed {
        let mut edit = ContentEdit::new(format!("Edit {label} of {area_key}"));
        edit.write_json(row_file("area", &area_key), &row);
        editor.apply(edit);
    }
    for action in actions {
        inspector::act(editor, action, "area", &area_key);
    }
    for action in out.actions {
        if let Some(snapshot) = super::AreaSnapshot::for_area(editor, &area_key) {
            super::run_action(editor, &snapshot, action);
        }
    }
    if editor.pick.is_some() {
        ui.add_space(6.0);
        pick_preview(editor, ui);
    }
    if editor
        .views
        .area
        .views
        .contains_key(&super::area_view_key(&area_key))
    {
        ui.add_space(8.0);
        module_counts(editor, ui, &super::area_view_key(&area_key), &area_key);
    }
}

fn module_inspector(
    ui: &mut Ui,
    cx: &mut FormCx,
    view: Option<&mut AreaViewState>,
    area_key: &str,
    row: &mut Value,
    index: usize,
    out: &mut Followups,
) -> bool {
    let variants = cx.shapes.module_variants("area");
    let path = super::module_part(index);
    let kind = row["modules"][index]["type"]
        .as_str()
        .unwrap_or("module")
        .to_owned();
    let module_kind = row["modules"][index].module_kind();
    let shown_kind = kind.replace('_', " ");
    ui.label(
        theme::muted(format!("area/{area_key} › {shown_kind}"))
            .monospace()
            .size(13.0),
    );
    ui.add_space(4.0);
    let mut changed = false;
    let content = cx.content.clone();
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, &shown_kind, "", |ui| {
            if ui.button("Copy as JSON").clicked() {
                out.actions.push(super::AreaAction::Copy);
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                out.actions.push(super::AreaAction::Duplicate);
                ui.close();
            }
            ui.separator();
            if ui
                .add(egui::Button::new("Remove").shortcut_text("Del"))
                .clicked()
            {
                out.actions.push(super::AreaAction::Delete);
                ui.close();
            }
        });
        ui.add_space(4.0);
        let module = &mut row["modules"][index];
        match schema::variant_of(&variants, module) {
            Some(variant) => {
                changed |= forms::object_without(
                    ui,
                    cx,
                    variant.schema,
                    module,
                    &path,
                    hidden_fields(module_kind),
                )
                .is_some();
            }
            None => {
                changed |= forms::raw_json(ui, cx, module, &path).is_some();
            }
        }
        match module_kind {
            Some(AreaModuleKind::Scenery) => changed |= scenery_rows(ui, module, &path),
            Some(AreaModuleKind::Population) => {
                if let Some(view) = view {
                    let preview = view
                        .population_previews
                        .entry(module.to_string())
                        .or_default();
                    widgets::kv(ui, "preview", |ui| {
                        let mut shown = !preview.hidden;
                        if widgets::toggle(ui, &mut shown).changed() {
                            preview.hidden = !shown;
                        }
                        if ui.button("↻ Re-roll").clicked() {
                            preview.seed = preview.seed.wrapping_add(1);
                        }
                    });
                }
            }
            _ => {}
        }
    });
    let module = &mut row["modules"][index];
    match module_kind {
        Some(AreaModuleKind::Scenery) => scenery_extras(ui, cx, module, out, index),
        Some(AreaModuleKind::Blueprint) => {
            if let Some(blueprint) = module["blueprint"].as_str().map(str::to_owned)
                && content
                    .as_ref()
                    .and_then(|c| {
                        c.by_key::<area::blueprint::BlueprintDef>(&blueprint)
                            .map(|id| !id.get(c).variables.is_empty())
                    })
                    .unwrap_or(false)
            {
                widgets::section(ui, &format!("Values · {blueprint}"));
                widgets::card_frame(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    widgets::card_title(ui, "values");
                    ui.add_space(4.0);
                    changed |= placement::placement_card(ui, cx, content.as_ref(), module, &path);
                });
            }
            placement_extras(ui, cx, module, out, index);
        }
        Some(AreaModuleKind::Population | AreaModuleKind::Resident) => from_npc(ui, cx, module),
        Some(AreaModuleKind::Object) => {
            ui.add_space(4.0);
            changed |= crate::ui::inspector::scripts::script_cards(
                ui, cx, "area", module, "scripts", &path,
            )
            .is_some();
            object_extras(ui, module, out, index);
        }
        Some(AreaModuleKind::Soundscape) => changed |= soundscape_extras(ui, cx, module, &path),
        _ => {}
    }
    changed
}

#[derive(Default)]
pub(super) struct Followups {
    pub actions: Vec<super::AreaAction>,
}

pub(super) fn hidden_fields(kind: Option<AreaModuleKind>) -> &'static [&'static str] {
    match kind {
        Some(AreaModuleKind::Scenery) => &["type", "tiles", "sort", "blocks"],
        Some(AreaModuleKind::Blueprint) => &["type", "values"],
        Some(AreaModuleKind::Object) => &["type", "scripts"],
        Some(AreaModuleKind::Soundscape) => &["type", "channels"],
        _ => &["type"],
    }
}

fn object_extras(ui: &mut Ui, module: &Value, out: &mut Followups, index: usize) {
    let destinations = placement::warp_destinations(module);
    if destinations.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (area, at) in &destinations {
            let where_ = format!("{area} · {}, {}", forms::tiles(at.x), forms::tiles(at.y));
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
            out.actions.push(super::AreaAction::ReturnDoor(index));
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
    forms::boxed_input(ui, Some(unit), |ui| {
        let mut text = if square {
            format!("{} × {}", brush.size, brush.size)
        } else {
            brush.size.to_string()
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .frame(egui::Frame::NONE)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
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
        response
    });
    changed
}

fn layer_combo(ui: &mut Ui, view: &mut AreaViewState, layers: &[String], salt: (&str, &str)) {
    let current = layers.get(view.layer).cloned().unwrap_or_default();
    widgets::combo_box(salt)
        .selected_text(theme::mono(current))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, name) in layers.iter().enumerate() {
                if ui
                    .selectable_label(view.layer == index, theme::mono(name))
                    .clicked()
                {
                    view.layer = index;
                }
            }
        });
}
