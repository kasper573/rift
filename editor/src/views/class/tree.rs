use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use ui::controls;
use ui::drag;
use ui::egui;
use ui::paint;
use ui::theme;
use ui::words;

use engine::core::content::ContentFile;
use engine::core::content::{AssetKind, ImageRef};
use engine::systems::class::{
    AccessRequirement, BackdropFit, CanvasPoint, CanvasRect, ClassDef, NodeKey, SkillNode,
    SkillTree, TreeBackdrop,
};
use engine::systems::skill::{Rank, SkillDef};
use strum::VariantArray;
use ui::egui::{Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::{glyph, ink};

use super::{edit, node_of, select_node, write};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::Project;
use crate::tabs::shown_path;
use crate::{picker, rows, thumbs, views};

pub const NODE_RADIUS: f32 = 24.0;
const ICON: f32 = 28.0;
const GRID: f32 = 20.0;
const MARGIN: f32 = 80.0;
const GHOST_OFFSET: Vec2 = Vec2::new(-150.0, -40.0);
const FIRST_SPOT: CanvasPoint = CanvasPoint { x: 80.0, y: 60.0 };
const SPOT_STEP: Vec2 = Vec2::new(80.0, 140.0);
const SPOTS_PER_ROW: u32 = 8;
const HANDLE: f32 = 10.0;
const FRAME_AROUND_NODES: f32 = 40.0;
const DEFAULT_FRAME: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));

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

/// A node picked up off the canvas, drawn under the pointer until it is let go of.
pub struct NodeDrag {
    pub node: NodeKey,
    offset: Vec2,
    at: CanvasPoint,
}

/// The payload of a node drag, which the node inspector takes as a skill requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CarriedNode(pub NodeKey);

/// What the canvas shows of a node's skill, read from the files before the tree is drawn.
pub(crate) struct NodeSkill {
    pub def: Option<Arc<SkillDef>>,
    pub path: String,
    pub needs: Vec<String>,
}

impl NodeSkill {
    pub fn name(&self) -> &str {
        self.def
            .as_ref()
            .map_or_else(|| shown_path(&self.path), |def| &def.name)
    }

    pub fn ranks(&self) -> u8 {
        self.def.as_ref().map_or(1, |def| def.rank_count())
    }

    fn full_cost(&self) -> u32 {
        self.def
            .as_ref()
            .map_or(0, |def| def.cost_to(Rank(def.rank_count())).0)
    }
}

pub(crate) fn node_skills(project: &Project, tree: &SkillTree) -> BTreeMap<NodeKey, NodeSkill> {
    let reader = project.reader();
    tree.0
        .iter()
        .map(|(key, node)| {
            let shown = NodeSkill {
                def: project.read_id(node.skill),
                path: reader.path_of(node.skill).unwrap_or_default().to_owned(),
                needs: node
                    .requires
                    .iter()
                    .map(|requirement| requirement.describe(project, tree))
                    .collect(),
            };
            (key.clone(), shown)
        })
        .collect()
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(read) = views::read_shown::<ClassDef>(editor, ui, key) else {
        return;
    };
    let Some(project) = &editor.project else {
        return;
    };
    let mut class = (*read).clone();
    let tree = read.tree.clone();
    let skills = node_skills(project, &tree);
    let mut changed: Option<&str> = None;
    toolbar(editor, ui, key, &mut class, &skills, &mut changed);
    backdrop_bar(
        editor,
        ui,
        key,
        &mut class,
        read.backdrop.as_ref(),
        &mut changed,
    );
    ui.add_space(4.0);
    if changed.is_none() {
        let shown = TreeShown {
            tree: &tree,
            backdrop: read.backdrop.as_ref(),
            skills: &skills,
        };
        canvas(editor, ui, key, &mut class, shown, &mut changed);
    }
    if let Some(label) = changed {
        write(editor, key, edit(key, label), &class);
    }
}

/// Takes `node` out of `tree`, and every requirement of another node on it.
pub(crate) fn remove_node(tree: &mut SkillTree, node: &NodeKey) {
    tree.0.remove(node);
    for other in tree.0.values_mut() {
        other.requires.retain(|requirement| {
            !matches!(requirement, AccessRequirement::Skill { node: needed, .. } if needed == node)
        });
    }
}

fn toolbar(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    class: &mut ClassDef,
    skills: &BTreeMap<NodeKey, NodeSkill>,
    changed: &mut Option<&'static str>,
) {
    let shapes = editor.shapes.clone();
    let mut snap = editor.views.class.snap;
    let mut picked = String::new();
    let granted: u32 = class.levels.iter().map(|level| level.skill_points.0).sum();
    let cost: u32 = skills.values().map(NodeSkill::full_cost).sum();
    editor.with_form(
        ClassDef::TABLE,
        Some(key),
        shapes.shape_of(ClassDef::TABLE),
        |cx| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut snap, "snap to grid")
                    .on_hover_text("A dragged node lands on the grid under the pointer");
                ui.add_space(12.0);
                ui.label(theme::muted("add node").size(12.5));
                ui.scope(|ui| {
                    ui.set_width(220.0);
                    picker::row(ui, cx, SkillDef::TABLE, &mut picked, "/tree/new/skill");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let line = format!(
                        "levels grant {granted} point{} · the tree costs {cost}",
                        words::plural(granted)
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
        },
    );
    editor.views.class.snap = snap;
    let Some(skill) = editor
        .reader()
        .and_then(|reader| reader.id_of::<SkillDef>(&picked))
    else {
        return;
    };
    let taken = class.tree.0.keys().map(|key| key.0.clone()).collect();
    let node = NodeKey::from(rows::free_name(&picked, &taken));
    let at = free_spot(&class.tree);
    class.tree.0.insert(
        node.clone(),
        SkillNode {
            skill,
            requires: Vec::new(),
            at,
        },
    );
    *changed = Some("Add node");
    select_node(editor, key, Some(node.as_str()));
}

fn backdrop_bar(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    class: &mut ClassDef,
    backdrop: Option<&TreeBackdrop>,
    changed: &mut Option<&'static str>,
) {
    let shapes = editor.shapes.clone();
    let mut image = backdrop.map_or(String::new(), |backdrop| backdrop.image.as_str().to_owned());
    let mut fit = backdrop.map_or(BackdropFit::Contain, |backdrop| backdrop.fit);
    let mut framed = false;
    let mut removed = false;
    editor.with_form(ClassDef::TABLE, Some(key), shapes.shape_of(ClassDef::TABLE), |cx| {
        ui.horizontal(|ui| {
            ui.label(theme::muted("backdrop").size(12.5));
            ui.scope(|ui| {
                ui.set_width(260.0);
                picker::asset_of(ui, cx, AssetKind::Image, &mut image, "/backdrop/image");
            });
            if backdrop.is_some() {
                controls::combo(ui.id().with("backdrop-fit"))
                    .selected_text(<&str>::from(fit))
                    .show_ui(ui, |ui| {
                        for &option in BackdropFit::VARIANTS {
                            ui.selectable_value(&mut fit, option, <&str>::from(option));
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
    });
    let frame = canvas_rect(nodes_frame(&class.tree));
    match (backdrop, class.backdrop.as_mut()) {
        (None, _) if !image.is_empty() => {
            class.backdrop = Some(TreeBackdrop {
                image: ImageRef::new(image),
                frame,
                fit: BackdropFit::Contain,
            });
            *changed = Some("Add backdrop");
        }
        (Some(_), _) if removed => {
            class.backdrop = None;
            *changed = Some("Remove backdrop");
        }
        (Some(held), Some(backdrop)) if image != held.image.as_str() => {
            backdrop.image = ImageRef::new(image);
            *changed = Some("Change backdrop image");
        }
        (Some(held), Some(backdrop)) if fit != held.fit => {
            backdrop.fit = fit;
            *changed = Some("Change backdrop fit");
        }
        (Some(_), Some(backdrop)) if framed => {
            backdrop.frame = frame;
            *changed = Some("Frame the nodes");
        }
        _ => {}
    }
}

fn nodes_frame(tree: &SkillTree) -> Rect {
    tree.bounds()
        .map_or(DEFAULT_FRAME, |nodes| nodes.expand(FRAME_AROUND_NODES))
}

/// What a class's skill tree shows: its nodes, the picture behind them and each node's skill.
#[derive(Clone, Copy)]
struct TreeShown<'a> {
    tree: &'a SkillTree,
    backdrop: Option<&'a TreeBackdrop>,
    skills: &'a BTreeMap<NodeKey, NodeSkill>,
}

fn canvas(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    class: &mut ClassDef,
    shown: TreeShown,
    changed: &mut Option<&'static str>,
) {
    let TreeShown {
        tree,
        backdrop,
        skills,
    } = shown;
    let selected = node_of(&editor.selection)
        .filter(|_| editor.selection.keys.first().is_some_and(|k| k == key))
        .map(NodeKey::from);
    let snap = editor.views.class.snap;
    let saved_frame = backdrop.map(|backdrop| backdrop.frame.rect());
    // The canvas keeps the extent of the saved frame while one is dragged, so what is under
    // the pointer stays put until it is let go of.
    let frame = editor
        .views
        .class
        .frame_drag
        .as_ref()
        .map(|drag| drag.now)
        .or(saved_frame);
    let (low, high) = tree
        .0
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
    let mut released: Option<(NodeKey, CanvasPoint)> = None;
    let mut clicked: Option<Option<NodeKey>> = None;
    let mut removed: Option<NodeKey> = None;
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
            let to_screen = |at: CanvasPoint| origin + Vec2::new(at.x, at.y);
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, theme::CANVAS);
            if let (Some(backdrop), Some(frame)) = (backdrop, frame) {
                let on_screen = frame.translate(origin.to_vec2());
                if let Some(image) = editor.thumb(ui.ctx(), backdrop.image.as_str()) {
                    let drawn = backdrop.fit.place(
                        on_screen,
                        Vec2::new(image.size[0] as f32, image.size[1] as f32),
                    );
                    painter.with_clip_rect(on_screen.intersect(rect)).image(
                        image.texture.id(),
                        drawn,
                        thumbs::FULL_UV,
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
            paint::dot_grid(&painter, rect, origin, GRID, ui::ink::ink_wash(0.25));
            if background.clicked() {
                clicked = Some(None);
            }
            let lifted = editor
                .views
                .class
                .drag
                .as_ref()
                .map(|drag| (drag.node.clone(), drag.at));
            let placed = |node: &NodeKey| -> Option<CanvasPoint> {
                match &lifted {
                    Some((dragged, at)) if dragged == node => Some(*at),
                    _ => tree.node(node).map(|node| node.at),
                }
            };
            let cycles = tree.cycles();
            for (to, node) in &tree.0 {
                let head = to_screen(placed(to).unwrap_or(node.at));
                let required = node
                    .requires
                    .iter()
                    .filter_map(|requirement| match requirement {
                        AccessRequirement::Skill { node, .. } => Some(node),
                        _ => None,
                    });
                for (index, from) in required.enumerate() {
                    match placed(from) {
                        Some(at) => {
                            let on_cycle = cycles.contains(to) && cycles.contains(from);
                            let color = if on_cycle {
                                theme::DANGER
                            } else {
                                ink::wash(ink::LIGHT, 0.7)
                            };
                            node_arrow(&painter, to_screen(at), head, color, false);
                        }
                        None => {
                            let ghost = head + GHOST_OFFSET + Vec2::new(0.0, index as f32 * 30.0);
                            glyph::dashed_circle(
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
                            node_arrow(&painter, ghost, head, theme::DANGER, true);
                        }
                    }
                }
            }
            let overlapping: BTreeSet<&NodeKey> = tree
                .0
                .iter()
                .filter(|(a, node)| {
                    tree.0
                        .iter()
                        .any(|(b, other)| a != &b && node.at.overlaps(other.at))
                })
                .map(|(key, _)| key)
                .collect();
            for (node_key, node) in &tree.0 {
                let at = placed(node_key).unwrap_or(node.at);
                let center = to_screen(at);
                let node_rect = Rect::from_center_size(center, Vec2::splat(NODE_RADIUS * 2.0));
                let response = ui.interact(
                    node_rect,
                    ui.id().with(("node", node_key)),
                    Sense::click_and_drag(),
                );
                let look = NodeLook {
                    selected: selected.as_ref() == Some(node_key),
                    hovered: response.hovered(),
                    overlapping: overlapping.contains(node_key),
                    lifted: lifted
                        .as_ref()
                        .is_some_and(|(dragged, _)| dragged == node_key),
                };
                let skill = &skills[node_key];
                let icon = skill
                    .def
                    .as_ref()
                    .and_then(|def| editor.thumb_icon(ui.ctx(), def.icon.as_str()));
                draw_node(&painter, center, look, icon.as_ref());
                glyph::haloed(
                    &painter,
                    center + Vec2::new(0.0, NODE_RADIUS + 4.0),
                    egui::Align2::CENTER_TOP,
                    skill.name(),
                    egui::FontId::proportional(11.5),
                    if look.selected {
                        ink::LIGHT
                    } else {
                        ink::wash(ink::LIGHT, 0.8)
                    },
                    glyph::Halo {
                        color: ink::INK,
                        width: 1.0,
                    },
                );
                if look.lifted && snap {
                    glyph::dashed_circle(
                        &painter,
                        to_screen(snapped(at)),
                        NODE_RADIUS,
                        Stroke::new(1.0, theme::ACCENT),
                    );
                }
                let response = response.on_hover_ui(|ui| {
                    ui.label(theme::strong(skill.name()));
                    ui.label(
                        theme::muted(format!("node {node_key} · {}", shown_path(&skill.path)))
                            .size(12.0),
                    );
                    for needs in &skill.needs {
                        ui.label(theme::muted(format!("needs {needs}")).size(12.0));
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
                    drag::carry(ui.ctx(), CarriedNode(node_key.clone()));
                }
            }
            if let Some(drag) = editor.views.class.drag.as_mut() {
                let pointer =
                    ui.input(|input| (input.pointer.latest_pos(), input.pointer.any_released()));
                if let Some(at) = pointer.0 {
                    let canvas_at = at - drag.offset - origin.to_vec2();
                    drag.at = CanvasPoint {
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
        && let Some(backdrop) = class.backdrop.as_mut()
        && backdrop.frame.rect() != now
    {
        backdrop.frame = canvas_rect(now);
        *changed = Some("Reframe backdrop");
    }
    if let Some((node, at)) = released {
        let blocked = tree
            .0
            .iter()
            .any(|(other, placed)| *other != node && at.overlaps(placed.at));
        if blocked {
            editor.toast("Nodes can't overlap");
        } else if let Some(moved) = class.tree.0.get_mut(&node)
            && moved.at != at
        {
            moved.at = at;
            *changed = Some("Move node");
        }
    }
    if let Some(node) = clicked {
        select_node(editor, key, node.as_ref().map(NodeKey::as_str));
    }
    if hovering_canvas
        && let Some(node) = selected.clone()
        && ui.input_mut(|input| EditorKey::Delete.consume(input))
    {
        removed = Some(node);
    }
    if let Some(node) = removed {
        remove_node(&mut class.tree, &node);
        *changed = Some("Remove node");
        select_node(editor, key, None);
    }
    editor
        .status
        .left
        .push(format!("{} › tree", shown_path(key)));
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
    clicked: &mut Option<Option<NodeKey>>,
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
        painter.circle_stroke(center, NODE_RADIUS + 2.0, Stroke::new(1.0, ink::GOLD));
    }
    match icon {
        Some(icon) => {
            painter.image(
                icon.texture.id(),
                Rect::from_center_size(center, Vec2::splat(ICON)),
                thumbs::FULL_UV,
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

/// An arrow between two node centres, trimmed to clear the circles.
fn node_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32, dashed: bool) {
    let span = to - from;
    if span.length() <= NODE_RADIUS * 2.0 {
        return;
    }
    let dir = span.normalized();
    glyph::arrow(
        painter,
        from + dir * (NODE_RADIUS + 4.0),
        to - dir * (NODE_RADIUS + 6.0),
        Stroke::new(2.0, color),
        dashed,
    );
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
    let min = Pos2::new(on_grid(rect.min.x), on_grid(rect.min.y));
    let max = Pos2::new(on_grid(rect.max.x), on_grid(rect.max.y));
    Rect::from_min_max(min, max.max(min + Vec2::splat(GRID)))
}

fn snapped(at: CanvasPoint) -> CanvasPoint {
    CanvasPoint {
        x: on_grid(at.x),
        y: on_grid(at.y),
    }
}

fn on_grid(value: f32) -> f32 {
    (value / GRID).round() * GRID
}

fn free_spot(tree: &SkillTree) -> CanvasPoint {
    (0..)
        .map(|index| CanvasPoint {
            x: FIRST_SPOT.x + (index % SPOTS_PER_ROW) as f32 * SPOT_STEP.x,
            y: FIRST_SPOT.y + (index / SPOTS_PER_ROW) as f32 * SPOT_STEP.y,
        })
        .find(|spot| !tree.0.values().any(|node| spot.overlaps(node.at)))
        .unwrap_or(FIRST_SPOT)
}

fn canvas_rect(rect: Rect) -> CanvasRect {
    CanvasRect {
        x: rect.min.x,
        y: rect.min.y,
        width: rect.width(),
        height: rect.height(),
    }
}
