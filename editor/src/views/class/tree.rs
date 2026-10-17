use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, CursorIcon, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use engine::systems::class::{BackdropFit, NODE_EXTENT};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{TABLE, node_of, select_node, write};
use crate::app::Editor;
use crate::forms::FormCx;
use crate::project::{ProjectFiles, row_file};
use crate::ui::assets::AssetKind;
use crate::ui::{inspector, theme, widgets};
use crate::{picker, thumbs};

pub const NODE_RADIUS: f32 = 24.0;
const ICON: f32 = 28.0;
const GRID: f32 = 20.0;
const MARGIN: f32 = 80.0;
const ARROW_HEAD: f32 = 7.0;
const GHOST_OFFSET: Vec2 = Vec2::new(-150.0, -40.0);
const FIRST_SPOT: Point = Point { x: 80.0, y: 60.0 };
const SPOT_STEP: Vec2 = Vec2::new(80.0, 140.0);
const SPOTS_PER_ROW: u32 = 8;
const HANDLE: f32 = 10.0;
const FRAME_AROUND_NODES: f32 = 40.0;
const DEFAULT_FRAME: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));
const FITS: [(BackdropFit, &str); 3] = [
    (BackdropFit::Contain, "contain"),
    (BackdropFit::Cover, "cover"),
    (BackdropFit::Stretch, "stretch"),
];

/// A node of a class tree as the canvas reads it from the row.
#[derive(Clone, Deserialize)]
pub struct Node {
    pub skill: String,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    pub at: Point,
}

#[derive(Clone, Copy, Deserialize, Default, PartialEq, Debug)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Requirement {
    Skill {
        node: String,
        #[serde(default = "first_rank")]
        rank: u8,
    },
    Level {
        level: u32,
    },
    Condition {
        #[serde(default)]
        condition: Value,
    },
}

pub type Nodes = BTreeMap<String, Node>;

/// The art behind the tree as the canvas reads it from the row.
#[derive(Clone, Deserialize)]
pub struct Backdrop {
    pub image: String,
    pub frame: Frame,
    #[serde(default)]
    pub fit: BackdropFit,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Debug)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Frame {
    fn rect(self) -> Rect {
        Rect::from_min_size(
            Pos2::new(self.x, self.y),
            Vec2::new(self.width, self.height),
        )
    }

    fn json(rect: Rect) -> Value {
        json!({ "x": rect.min.x, "y": rect.min.y, "width": rect.width(), "height": rect.height() })
    }
}

/// The backdrop's frame taken hold of, by a corner or its middle, until it is let go of.
pub struct FrameDrag {
    grip: FrameGrip,
    start: Rect,
    grab: Pos2,
    now: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameGrip {
    Move,
    Corner(usize),
}

pub fn backdrop_of(row: &Value) -> Option<Backdrop> {
    serde_json::from_value(row["backdrop"].clone()).ok()
}

/// A node picked up off the canvas, drawn under the pointer until it is let go of.
pub struct NodeDrag {
    pub node: String,
    offset: Vec2,
    at: Point,
}

/// The payload of a node drag, which the node inspector takes as a skill requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarriedNode(pub String);

pub fn nodes_of(row: &Value) -> Nodes {
    row["tree"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, node)| {
            serde_json::from_value(node.clone())
                .ok()
                .map(|node| (key.clone(), node))
        })
        .collect()
}

/// Whether making `node` require `named` would close a loop: `named` already needs `node`.
pub fn would_cycle(nodes: &Nodes, node: &str, named: &str) -> bool {
    let mut stack = vec![named.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(key) = stack.pop() {
        if key == node {
            return true;
        }
        let Some(next) = nodes.get(&key) else {
            continue;
        };
        for requirement in &next.requires {
            if let Requirement::Skill { node: further, .. } = requirement
                && seen.insert(further.clone())
            {
                stack.push(further.clone());
            }
        }
    }
    false
}

pub fn skill_ranks(files: &ProjectFiles, skill: &str) -> u8 {
    files
        .json(&row_file("skill", skill))
        .and_then(|row| row["levels"].as_array().map(Vec::len))
        .unwrap_or(1)
        .max(1) as u8
}

pub fn skill_cost(files: &ProjectFiles, skill: &str) -> u32 {
    files
        .json(&row_file("skill", skill))
        .and_then(|row| {
            row["levels"].as_array().map(|levels| {
                levels
                    .iter()
                    .map(|level| level["cost"].as_u64().unwrap_or(0) as u32)
                    .sum()
            })
        })
        .unwrap_or(0)
}

pub fn granted_points(row: &Value) -> u32 {
    row["levels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|level| level["skill_points"].as_u64().unwrap_or(0) as u32)
        .sum()
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(mut row) = editor.row_value(TABLE, key) else {
        return;
    };
    let nodes = nodes_of(&row);
    let mut changed: Option<&str> = None;
    toolbar(editor, ui, key, &mut row, &nodes, &mut changed);
    ui.add_space(4.0);
    if changed.is_none() {
        canvas(editor, ui, key, &mut row, &nodes, &mut changed);
    }
    if let Some(label) = changed {
        write(editor, key, label, &row);
    }
}

pub fn remove_node(row: &mut Value, node: &str) {
    let Some(tree) = row["tree"].as_object_mut() else {
        return;
    };
    tree.remove(node);
    for other in tree.values_mut() {
        if let Some(requires) = other["requires"].as_array_mut() {
            requires.retain(|requirement| {
                !(requirement["type"] == "skill" && requirement["node"] == node)
            });
            if requires.is_empty() {
                other.as_object_mut().expect("object").remove("requires");
            }
        }
    }
}

fn toolbar(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    row: &mut Value,
    nodes: &Nodes,
    changed: &mut Option<&'static str>,
) {
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(TABLE);
    let mut actions = Vec::new();
    let mut snap = editor.views.class.snaps();
    let mut picked = String::new();
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx =
            FormCx::new(project, root, shapes, form, &mut actions).owned_by(TABLE, Some(key));
        ui.horizontal(|ui| {
            ui.checkbox(&mut snap, "snap to grid")
                .on_hover_text("A dragged node lands on the grid under the pointer");
            ui.add_space(12.0);
            ui.label(theme::muted("add node").size(12.5));
            ui.scope(|ui| {
                ui.set_width(220.0);
                picker::row(ui, &mut cx, "skill", &mut picked, "/tree/new/skill");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let granted = granted_points(row);
                let cost: u32 = nodes
                    .values()
                    .map(|node| skill_cost(cx.files, &node.skill))
                    .sum();
                let line = format!(
                    "levels grant {granted} point{} · the tree costs {cost}",
                    widgets::plural(granted)
                );
                if granted > cost && cost > 0 {
                    ui.label(theme::warn(format!(
                        "{line} · every node can be maxed with {} to spare",
                        granted - cost
                    )));
                } else if granted == 0 && cost > 0 {
                    ui.label(theme::warn(format!("{line} · no level grants a point")));
                } else {
                    ui.label(theme::muted(line).size(12.5));
                }
            });
        });
    }
    editor.views.class.snap = Some(snap);
    backdrop_bar(editor, ui, key, row, nodes, changed);
    if !picked.is_empty() {
        let node = fresh_key(nodes, &picked);
        let at = free_spot(nodes);
        if !row["tree"].is_object() {
            row["tree"] = Value::Object(Map::new());
        }
        row["tree"][&node] = serde_json::json!({ "skill": picked, "at": { "x": at.x, "y": at.y } });
        *changed = Some("Add node");
        select_node(editor, key, Some(&node));
    }
    for action in actions {
        inspector::act(editor, action, TABLE, key);
    }
}

fn backdrop_bar(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    row: &mut Value,
    nodes: &Nodes,
    changed: &mut Option<&'static str>,
) {
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(TABLE);
    let mut actions = Vec::new();
    let backdrop = backdrop_of(row);
    let mut image = backdrop
        .as_ref()
        .map(|backdrop| backdrop.image.clone())
        .unwrap_or_default();
    let mut fit = backdrop
        .as_ref()
        .map_or(BackdropFit::Contain, |backdrop| backdrop.fit);
    let mut framed = false;
    let mut removed = false;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx =
            FormCx::new(project, root, shapes, form, &mut actions).owned_by(TABLE, Some(key));
        ui.horizontal(|ui| {
            ui.label(theme::muted("backdrop").size(12.5));
            ui.scope(|ui| {
                ui.set_width(260.0);
                picker::asset_of(ui, &mut cx, &[AssetKind::Image], &mut image, "/backdrop/image");
            });
            if backdrop.is_some() {
                design_tokens::widgets::combo_box(ui.id().with("backdrop-fit"))
                    .selected_text(fit_label(fit))
                    .show_ui(ui, |ui| {
                        for (option, label) in FITS {
                            ui.selectable_value(&mut fit, option, label);
                        }
                    })
                    .response
                    .on_hover_text("How the image meets its frame: shown whole, cropped to fill it, or stretched");
                framed = ui
                    .button("frame the nodes")
                    .on_hover_text("Sizes the frame to the nodes, with room around them")
                    .clicked();
                removed = ui.button("remove").clicked();
            }
        });
    }
    for action in actions {
        inspector::act(editor, action, TABLE, key);
    }
    match &backdrop {
        None if !image.is_empty() => {
            row["backdrop"] = json!({
                "image": image,
                "frame": Frame::json(nodes_frame(nodes)),
                "fit": "contain",
            });
            *changed = Some("Add backdrop");
        }
        Some(_) if removed => {
            row.as_object_mut().expect("object").remove("backdrop");
            *changed = Some("Remove backdrop");
        }
        Some(backdrop) if image != backdrop.image => {
            row["backdrop"]["image"] = Value::String(image);
            *changed = Some("Change backdrop image");
        }
        Some(backdrop) if fit != backdrop.fit => {
            row["backdrop"]["fit"] = Value::String(fit_label(fit).to_owned());
            *changed = Some("Change backdrop fit");
        }
        Some(_) if framed => {
            row["backdrop"]["frame"] = Frame::json(nodes_frame(nodes));
            *changed = Some("Frame the nodes");
        }
        _ => {}
    }
}

fn fit_label(fit: BackdropFit) -> &'static str {
    FITS.iter()
        .find(|(option, _)| *option == fit)
        .map_or("contain", |(_, label)| label)
}

fn nodes_frame(nodes: &Nodes) -> Rect {
    nodes
        .values()
        .map(|node| {
            Rect::from_center_size(
                Pos2::new(node.at.x, node.at.y),
                Vec2::splat(NODE_EXTENT * 2.0),
            )
        })
        .reduce(|all, next| all.union(next))
        .map_or(DEFAULT_FRAME, |rect| rect.expand(FRAME_AROUND_NODES))
}

fn canvas(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    row: &mut Value,
    nodes: &Nodes,
    changed: &mut Option<&'static str>,
) {
    let selected = node_of(&editor.selection)
        .filter(|_| editor.selection.keys.first().is_some_and(|k| k == key))
        .map(str::to_owned);
    let snap = editor.views.class.snaps();
    let backdrop = backdrop_of(row);
    let saved_frame = backdrop.as_ref().map(|backdrop| backdrop.frame.rect());
    // The canvas keeps the extent of the saved frame while one is dragged, so what is under
    // the pointer stays put until it is let go of.
    let frame = editor
        .views
        .class
        .frame_drag
        .as_ref()
        .map(|drag| drag.now)
        .or(saved_frame);
    let (low, high) = nodes
        .values()
        .map(|node| Vec2::new(node.at.x, node.at.y))
        .chain(
            saved_frame
                .into_iter()
                .flat_map(|frame| [frame.min.to_vec2(), frame.max.to_vec2()]),
        )
        .fold((Vec2::ZERO, Vec2::ZERO), |(low, high), at| {
            (low.min(at), high.max(at))
        });
    let wanted = high - low + Vec2::splat(MARGIN * 2.0);
    let mut released: Option<(String, Point)> = None;
    let mut clicked: Option<Option<String>> = None;
    let mut removed: Option<String> = None;
    let mut started: Option<NodeDrag> = None;
    let mut frame_started: Option<FrameDrag> = None;
    let mut frame_released: Option<Rect> = None;
    let mut hovering_canvas = false;
    egui::ScrollArea::both()
        .id_salt(("class-tree", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = wanted.max(ui.available_size());
            let (rect, background) = ui.allocate_exact_size(size, Sense::click());
            hovering_canvas = ui.rect_contains_pointer(rect);
            let origin = rect.min + Vec2::splat(MARGIN) - low;
            let to_screen = |at: Point| origin + Vec2::new(at.x, at.y);
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, theme::CANVAS);
            if let (Some(backdrop), Some(frame)) = (&backdrop, frame) {
                let on_screen = frame.translate(origin.to_vec2());
                let files = editor.project.as_ref().map(|project| &project.files);
                let image = files.and_then(|files| {
                    editor
                        .form
                        .thumbnails
                        .image(ui.ctx(), files, &backdrop.image)
                });
                if let Some(image) = image {
                    let drawn = backdrop.fit.place(
                        on_screen,
                        Vec2::new(image.size[0] as f32, image.size[1] as f32),
                    );
                    painter.with_clip_rect(on_screen.intersect(rect)).image(
                        image.texture.id(),
                        drawn,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                let holding = editor.views.class.frame_drag.as_ref();
                frame_handles(
                    ui,
                    &painter,
                    frame,
                    origin,
                    holding.map(|drag| drag.grip),
                    &mut frame_started,
                    &mut clicked,
                );
                if let Some(drag) = holding
                    && let Some(pointer) = ui.input(|input| input.pointer.latest_pos())
                {
                    let mut now = dragged_frame(drag, pointer);
                    if snap {
                        now = snapped_rect(now);
                    }
                    frame_released = ui
                        .input(|input| input.pointer.any_released())
                        .then_some(now);
                    editor.views.class.frame_drag = Some(FrameDrag { now, ..*drag });
                }
            }
            dots(&painter, rect, origin);
            if background.clicked() {
                clicked = Some(None);
            }
            let dragging = editor.views.class.drag.as_ref();
            let placed = |node: &str| -> Option<Point> {
                match dragging {
                    Some(drag) if drag.node == node => Some(drag.at),
                    _ => nodes.get(node).map(|node| node.at),
                }
            };
            let cycles = cycles(nodes);
            for (to, node) in nodes {
                for (index, requirement) in node.requires.iter().enumerate() {
                    let Requirement::Skill { node: from, .. } = requirement else {
                        continue;
                    };
                    let head = to_screen(placed(to).unwrap_or(node.at));
                    match placed(from) {
                        Some(at) => {
                            let on_cycle =
                                cycles.contains(to.as_str()) && cycles.contains(from.as_str());
                            let color = if on_cycle {
                                theme::DANGER
                            } else {
                                theme::MUTED
                            };
                            arrow(&painter, to_screen(at), head, color, false);
                        }
                        None => {
                            let ghost = head + GHOST_OFFSET + Vec2::new(0.0, index as f32 * 30.0);
                            dashed_circle(
                                &painter,
                                ghost,
                                NODE_RADIUS * 0.6,
                                Stroke::new(1.0, theme::DANGER),
                            );
                            painter.text(
                                ghost + Vec2::new(0.0, NODE_RADIUS * 0.6 + 4.0),
                                egui::Align2::CENTER_TOP,
                                format!("{from}?"),
                                egui::FontId::monospace(10.5),
                                theme::DANGER,
                            );
                            arrow(&painter, ghost, head, theme::DANGER, true);
                        }
                    }
                }
            }
            let overlapping: BTreeSet<&str> = nodes
                .iter()
                .flat_map(|(a, node)| {
                    nodes.iter().filter_map(move |(b, other)| {
                        (a != b && overlaps(node.at, other.at)).then_some(a.as_str())
                    })
                })
                .collect();
            let files = editor.project.as_ref().map(|project| &project.files);
            for (node_key, node) in nodes {
                let at = placed(node_key).unwrap_or(node.at);
                let center = to_screen(at);
                let node_rect = Rect::from_center_size(center, Vec2::splat(NODE_RADIUS * 2.0));
                let response = ui.interact(
                    node_rect,
                    ui.id().with(("node", node_key)),
                    Sense::click_and_drag(),
                );
                let look = NodeLook {
                    selected: selected.as_deref() == Some(node_key),
                    hovered: response.hovered(),
                    overlapping: overlapping.contains(node_key.as_str()),
                    lifted: dragging.is_some_and(|drag| drag.node == *node_key),
                };
                let icon = files.and_then(|files| {
                    let skill = files.json(&row_file("skill", &node.skill))?;
                    let path = skill["icon"].as_str()?.to_owned();
                    let mut budget = u32::MAX;
                    editor
                        .form
                        .thumbnails
                        .icon(ui.ctx(), files, &path, &mut budget)
                });
                draw_node(&painter, center, look, icon.as_ref());
                let name = files
                    .and_then(|files| files.json(&row_file("skill", &node.skill)))
                    .and_then(|skill| skill["name"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| node.skill.clone());
                painter.text(
                    center + Vec2::new(0.0, NODE_RADIUS + 4.0),
                    egui::Align2::CENTER_TOP,
                    &name,
                    egui::FontId::proportional(11.5),
                    if look.selected {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    },
                );
                if look.lifted && snap {
                    dashed_circle(
                        &painter,
                        to_screen(snapped(at)),
                        NODE_RADIUS,
                        Stroke::new(1.0, theme::ACCENT),
                    );
                }
                let response = response.on_hover_ui(|ui| {
                    ui.label(theme::strong(&name));
                    ui.label(
                        theme::muted(format!("node {node_key} · skill/{}", node.skill)).size(12.0),
                    );
                    for requirement in &node.requires {
                        ui.label(theme::muted(describe(requirement, nodes, files)).size(12.0));
                    }
                    ui.label(
                        theme::faint("drag to move · drop on the inspector's list to require it")
                            .size(11.5),
                    );
                });
                response.context_menu(|ui| {
                    if ui.button("Remove node").clicked() {
                        removed = Some(node_key.clone());
                        ui.close();
                    }
                });
                if response.clicked() {
                    clicked = Some(Some(node_key.clone()));
                }
                if response.drag_started()
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    started = Some(NodeDrag {
                        node: node_key.clone(),
                        offset: pointer - center,
                        at,
                    });
                    egui::DragAndDrop::set_payload(ui.ctx(), CarriedNode(node_key.clone()));
                }
            }
            if let Some(drag) = editor.views.class.drag.as_mut() {
                let pointer =
                    ui.input(|input| (input.pointer.latest_pos(), input.pointer.any_released()));
                if let Some(at) = pointer.0 {
                    let canvas_at = at - drag.offset - origin.to_vec2();
                    drag.at = Point {
                        x: canvas_at.x.max(0.0),
                        y: canvas_at.y.max(0.0),
                    };
                }
                if pointer.1 {
                    let inside = pointer.0.is_some_and(|at| rect.contains(at));
                    if inside {
                        let at = if snap { snapped(drag.at) } else { drag.at };
                        released = Some((drag.node.clone(), at));
                    }
                }
            }
        });
    if let Some(drag) = started {
        editor.views.class.drag = Some(drag);
    }
    if let Some(drag) = frame_started {
        editor.views.class.frame_drag = Some(drag);
    }
    if ui.input(|input| input.pointer.any_released()) {
        editor.views.class.drag = None;
        editor.views.class.frame_drag = None;
    }
    if let Some(now) = frame_released
        && backdrop
            .as_ref()
            .is_some_and(|backdrop| backdrop.frame.rect() != now)
    {
        row["backdrop"]["frame"] = Frame::json(now);
        *changed = Some("Reframe backdrop");
    }
    if let Some((node, at)) = released {
        let blocked = nodes
            .iter()
            .any(|(other, placed)| *other != node && overlaps(at, placed.at));
        if blocked {
            editor.toast("Nodes can't overlap");
        } else if nodes.get(&node).is_some_and(|node| node.at != at) {
            row["tree"][&node]["at"] = serde_json::json!({ "x": at.x, "y": at.y });
            *changed = Some("Move node");
        }
    }
    if let Some(node) = clicked {
        select_node(editor, key, node.as_deref());
    }
    if hovering_canvas
        && let Some(node) = selected.clone()
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Delete))
    {
        removed = Some(node);
    }
    if let Some(node) = removed {
        remove_node(row, &node);
        *changed = Some("Remove node");
        select_node(editor, key, None);
    }
    editor.status.left.push(format!("class {key} › tree"));
    editor.status.left.push(
        "click a node to inspect it · drag to move · right-click or Delete removes it".to_owned(),
    );
    if backdrop.is_some() {
        editor
            .status
            .left
            .push("drag the backdrop's corners to size it, its middle to move it".to_owned());
    }
}

/// The frame's outline and grips on the canvas: `frame` is in canvas coordinates, drawn at
/// `origin`. Registered before the nodes, so a node over the frame still takes the pointer.
fn frame_handles(
    ui: &mut Ui,
    painter: &egui::Painter,
    frame: Rect,
    origin: Pos2,
    held: Option<FrameGrip>,
    started: &mut Option<FrameDrag>,
    clicked: &mut Option<Option<String>>,
) {
    let on_screen = frame.translate(origin.to_vec2());
    let color = if held.is_some() {
        theme::ACCENT
    } else {
        theme::MUTED
    };
    painter.rect_stroke(on_screen, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    let middle = ui.interact(
        on_screen.shrink(HANDLE),
        ui.id().with("backdrop-frame"),
        Sense::click_and_drag(),
    );
    if middle.clicked() {
        *clicked = Some(None);
    }
    let mut take = |response: &egui::Response, grip: FrameGrip| {
        if response.drag_started()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            *started = Some(FrameDrag {
                grip,
                start: frame,
                grab: pointer,
                now: frame,
            });
        }
    };
    take(&middle, FrameGrip::Move);
    let corners = [
        on_screen.left_top(),
        on_screen.right_top(),
        on_screen.right_bottom(),
        on_screen.left_bottom(),
    ];
    for (index, corner) in corners.into_iter().enumerate() {
        let handle = Rect::from_center_size(corner, Vec2::splat(HANDLE));
        let cursor = if index % 2 == 0 {
            CursorIcon::ResizeNwSe
        } else {
            CursorIcon::ResizeNeSw
        };
        let response = ui
            .interact(
                handle,
                ui.id().with(("backdrop-corner", index)),
                Sense::drag(),
            )
            .on_hover_cursor(cursor);
        take(&response, FrameGrip::Corner(index));
        let lit = response.hovered() || held == Some(FrameGrip::Corner(index));
        painter.rect_filled(handle, 1.0, if lit { theme::ACCENT } else { theme::RAISED });
        painter.rect_stroke(handle, 1.0, Stroke::new(1.0, color), StrokeKind::Inside);
    }
}

#[derive(Clone, Copy)]
struct NodeLook {
    selected: bool,
    hovered: bool,
    overlapping: bool,
    lifted: bool,
}

fn draw_node(
    painter: &egui::Painter,
    center: Pos2,
    look: NodeLook,
    icon: Option<&thumbs::Thumbnail>,
) {
    let stroke = if look.overlapping {
        Stroke::new(2.0, theme::DANGER)
    } else if look.selected {
        Stroke::new(2.5, theme::ACCENT)
    } else {
        Stroke::new(1.5, theme::BORDER)
    };
    let fill = if look.lifted {
        theme::ACCENT_SOFT
    } else {
        theme::RAISED
    };
    painter.circle_filled(center, NODE_RADIUS, fill);
    painter.circle_stroke(center, NODE_RADIUS, stroke);
    if look.hovered {
        painter.circle_stroke(
            center,
            NODE_RADIUS + 2.0,
            Stroke::new(1.0, design_tokens::color::HOVER_LINE),
        );
    }
    match icon {
        Some(icon) => {
            painter.image(
                icon.texture.id(),
                Rect::from_center_size(center, Vec2::splat(ICON)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(16.0),
                theme::MUTED,
            );
        }
    }
}

fn describe(requirement: &Requirement, nodes: &Nodes, files: Option<&ProjectFiles>) -> String {
    match requirement {
        Requirement::Skill { node, rank } => {
            let name = nodes
                .get(node)
                .and_then(|node| files?.json(&row_file("skill", &node.skill)))
                .and_then(|skill| skill["name"].as_str().map(str::to_owned))
                .unwrap_or_else(|| node.clone());
            format!("needs {name} rank {rank}")
        }
        Requirement::Level { level } => format!("needs level {level}"),
        Requirement::Condition { .. } => "needs a condition".to_owned(),
    }
}

fn cycles(nodes: &Nodes) -> BTreeSet<&str> {
    let mut on_cycle = BTreeSet::new();
    for start in nodes.keys() {
        let mut stack = vec![start.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(key) = stack.pop() {
            let Some(node) = nodes.get(key) else {
                continue;
            };
            for requirement in &node.requires {
                if let Requirement::Skill { node: next, .. } = requirement {
                    if next == start {
                        on_cycle.insert(start.as_str());
                    } else if seen.insert(next.as_str()) {
                        stack.push(next.as_str());
                    }
                }
            }
        }
    }
    on_cycle
}

fn dragged_frame(drag: &FrameDrag, pointer: Pos2) -> Rect {
    let moved = pointer - drag.grab;
    let start = drag.start;
    let (left, top) = match drag.grip {
        FrameGrip::Move => return start.translate(moved),
        FrameGrip::Corner(corner) => (corner == 0 || corner == 3, corner == 0 || corner == 1),
    };
    let mut rect = start;
    if left {
        rect.min.x = (start.min.x + moved.x).min(start.max.x - GRID);
    } else {
        rect.max.x = (start.max.x + moved.x).max(start.min.x + GRID);
    }
    if top {
        rect.min.y = (start.min.y + moved.y).min(start.max.y - GRID);
    } else {
        rect.max.y = (start.max.y + moved.y).max(start.min.y + GRID);
    }
    rect
}

fn snapped_rect(rect: Rect) -> Rect {
    let snap = |value: f32| (value / GRID).round() * GRID;
    let min = Pos2::new(snap(rect.min.x), snap(rect.min.y));
    let max = Pos2::new(snap(rect.max.x), snap(rect.max.y));
    Rect::from_min_max(min, max.max(min + Vec2::splat(GRID)))
}

fn overlaps(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < NODE_EXTENT && (a.y - b.y).abs() < NODE_EXTENT
}

fn snapped(at: Point) -> Point {
    Point {
        x: (at.x / GRID).round() * GRID,
        y: (at.y / GRID).round() * GRID,
    }
}

fn free_spot(nodes: &Nodes) -> Point {
    (0..)
        .map(|index| Point {
            x: FIRST_SPOT.x + (index % SPOTS_PER_ROW) as f32 * SPOT_STEP.x,
            y: FIRST_SPOT.y + (index / SPOTS_PER_ROW) as f32 * SPOT_STEP.y,
        })
        .find(|spot| !nodes.values().any(|node| overlaps(*spot, node.at)))
        .unwrap_or(FIRST_SPOT)
}

fn fresh_key(nodes: &Nodes, skill: &str) -> String {
    if !nodes.contains_key(skill) {
        return skill.to_owned();
    }
    (2..)
        .map(|n| format!("{skill}_{n}"))
        .find(|key| !nodes.contains_key(key))
        .expect("an unused key")
}

fn dots(painter: &egui::Painter, rect: Rect, origin: Pos2) {
    let color = Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    let mut x = origin.x - ((origin.x - rect.min.x) / GRID).floor() * GRID;
    while x <= rect.max.x {
        let mut y = origin.y - ((origin.y - rect.min.y) / GRID).floor() * GRID;
        while y <= rect.max.y {
            painter.circle_filled(Pos2::new(x, y), 1.0, color);
            y += GRID;
        }
        x += GRID;
    }
}

fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32, dashed: bool) {
    let delta = to - from;
    let length = delta.length();
    if length <= NODE_RADIUS * 2.0 {
        return;
    }
    let dir = delta / length;
    let start = from + dir * (NODE_RADIUS + 4.0);
    let end = to - dir * (NODE_RADIUS + 6.0);
    let stroke = Stroke::new(2.0, color);
    if dashed {
        painter.add(egui::Shape::dashed_line(&[start, end], stroke, 6.0, 4.0));
    } else {
        painter.line_segment([start, end], stroke);
    }
    let side = Vec2::new(-dir.y, dir.x);
    let tip = end + dir * ARROW_HEAD;
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            end - dir * 2.0 + side * ARROW_HEAD * 0.6,
            end - dir * 2.0 - side * ARROW_HEAD * 0.6,
        ],
        color,
        Stroke::NONE,
    ));
}

fn dashed_circle(painter: &egui::Painter, center: Pos2, radius: f32, stroke: Stroke) {
    let points: Vec<Pos2> = (0..=48)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 48.0;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::dashed_line(&points, stroke, 4.0, 3.0));
}

fn first_rank() -> u8 {
    1
}
