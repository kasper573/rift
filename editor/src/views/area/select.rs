use engine::core::content::table_of;
use engine::core::math::{Offset, Pos, Rect};
use engine::core::units::{Meters, TilePos};
use engine::systems::area::AreaModule;
use engine::systems::area::scene::SceneNode;
use engine::systems::script::Variable;
use std::collections::BTreeMap;
use ui::egui;
use ui::egui::{Pos2, Ui};
use ui::words;

use super::flags::RowFlags;
use super::placement::{self, AreaPlacement, PartView};
use super::{
    AreaDrag, AreaFrame, AreaSnapshot, AreaTool, AreaViewState, PasteGhost, ResizeCorner,
    blueprint, nodes, pick, pins, scene_tree, shapes,
};
use crate::app::{Clipboard, Editor, SceneClip};
use crate::keymap::EditorKey;
use crate::project::RowSelection;

/// The modules of the scene of `bundle` the selection moves, in tree order: those selected and
/// what selected folders hold.
pub(super) fn selected_nodes(editor: &Editor, frame: &AreaSnapshot) -> Vec<String> {
    frame.selected_modules(&super::selected_parts(editor, &frame.bundle))
}

pub(super) fn select_input(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    frame: &AreaFrame,
    modifiers: egui::Modifiers,
) {
    let scope = match frame.in_place.last() {
        Some(place) => SelectScope::Blueprint {
            place,
            parts: placement::parts_in_place(frame, place),
        },
        None => SelectScope::Area,
    };
    let response = frame.response;
    let Some(tile) = scope.tile(frame) else {
        if response.drag_stopped() {
            finish_select_drag(editor, view_key, &scope, frame, modifiers);
        }
        return;
    };
    if scope.is_area() {
        if EditorKey::Listen.held_among(modifiers) && response.clicked() {
            editor.views.area.view_mut(view_key).listener = Some(tile);
            pick::listen(editor, frame, tile);
            return;
        }
        if let Some(ghost) = editor.views.area.view(view_key).paste.clone() {
            if response.clicked() || ui.input(|i| EditorKey::Confirm.pressed(i)) {
                finish_paste(editor, view_key, frame, ghost, tile);
            }
            return;
        }
    }
    let hit = scope.hit(frame, tile);
    if response.drag_started_by(egui::PointerButton::Primary) {
        start_select_drag(editor, view_key, frame, &scope, tile, hit);
        return;
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        track_move(editor.views.area.view_mut(view_key), tile);
        return;
    }
    if response.drag_stopped() {
        finish_select_drag(editor, view_key, &scope, frame, modifiers);
        return;
    }
    if response.double_clicked() && scope.double_click(editor, view_key, frame, hit.clone()) {
        return;
    }
    if response.clicked() {
        if scope.is_area()
            && let Some(pin) = pins::pin_at(editor, frame, frame.pointer.unwrap_or_default())
        {
            pins::open_pin(editor, pin);
            return;
        }
        match hit {
            Some(node) => {
                let parts = toggled(
                    editor,
                    scope.table(frame),
                    scope.key(frame),
                    node,
                    modifiers,
                );
                editor.selection = RowSelection::parts(scope.table(frame), scope.key(frame), parts);
            }
            None if !EditorKey::ExtendSelection.held_among(modifiers)
                && !EditorKey::ToggleSelection.held_among(modifiers) =>
            {
                scope.clear_selection(editor, frame)
            }
            None => {}
        }
        if scope.is_area() {
            editor.views.area.view_mut(view_key).cursor_cell = Some(tile.cell());
        }
    }
}

enum SelectScope<'a> {
    Area,
    Blueprint {
        place: &'a AreaPlacement,
        parts: Vec<PartView>,
    },
}

impl SelectScope<'_> {
    fn is_area(&self) -> bool {
        matches!(self, SelectScope::Area)
    }

    fn tile(&self, frame: &AreaFrame) -> Option<Pos<Meters>> {
        match self {
            SelectScope::Area => frame.pointer_tile,
            SelectScope::Blueprint { .. } => frame.drag_tile(),
        }
    }

    fn table<'a>(&'a self, frame: &'a AreaFrame) -> &'a str {
        table_of(self.key(frame)).unwrap_or_default()
    }

    fn key<'a>(&'a self, frame: &'a AreaFrame) -> &'a str {
        match self {
            SelectScope::Area => &frame.bundle,
            SelectScope::Blueprint { place, .. } => &place.blueprint,
        }
    }

    fn selected(&self, editor: &Editor, frame: &AreaFrame) -> Vec<String> {
        match self {
            SelectScope::Area => selected_nodes(editor, frame),
            SelectScope::Blueprint { place, .. } => {
                placement::selected_in_place(editor, &place.blueprint)
            }
        }
    }

    fn hit(&self, frame: &AreaFrame, tile: Pos<Meters>) -> Option<String> {
        match self {
            SelectScope::Area => frame.hit(tile),
            SelectScope::Blueprint { parts, .. } => parts
                .iter()
                .filter(|part| !part.locked && part.bounds.is_some_and(|b| b.contains(tile)))
                .min_by(|a, b| {
                    a.bounds
                        .map_or(0.0, |b| b.area())
                        .total_cmp(&b.bounds.map_or(0.0, |b| b.area()))
                })
                .map(|part| part.part.clone()),
        }
    }

    /// What a box over `rect` takes, and how many it passes over because they're hidden or locked.
    fn contained(&self, frame: &AreaFrame, rect: Rect<Meters>) -> (Vec<String>, Skipped) {
        let mut skipped = Skipped::default();
        let taken = match self {
            SelectScope::Area => frame
                .modules
                .iter()
                .filter(|module| {
                    super::is_selectable(module.module())
                        && frame
                            .bounds_of(module)
                            .is_some_and(|bounds| rect.contains_rect(&bounds))
                })
                .filter(|module| skipped.passes(&frame.flags.row(&module.pointer)))
                .map(|module| module.pointer.clone())
                .collect(),
            SelectScope::Blueprint { parts, .. } => parts
                .iter()
                .filter(|part| part.bounds.is_some_and(|b| rect.contains_rect(&b)))
                .filter(|part| {
                    skipped.locked += usize::from(part.locked);
                    !part.locked
                })
                .map(|part| part.part.clone())
                .collect(),
        };
        (taken, skipped)
    }

    fn move_selected(&self, editor: &mut Editor, frame: &AreaFrame, dx: f32, dy: f32) {
        match self {
            SelectScope::Area => move_selection(editor, frame, dx, dy),
            SelectScope::Blueprint { place, .. } => {
                let parts = self.selected(editor, frame);
                if !parts.is_empty() {
                    placement::move_parts_in_place(editor, frame, place, &parts, dx, dy);
                }
            }
        }
    }

    fn clear_selection(&self, editor: &mut Editor, frame: &AreaFrame) {
        match self {
            SelectScope::Area => {
                editor.selection = RowSelection::row(self.table(frame), &frame.bundle)
            }
            SelectScope::Blueprint { .. } => editor.selection.parts.clear(),
        }
    }

    fn double_click(
        &self,
        editor: &mut Editor,
        view_key: &str,
        frame: &AreaFrame,
        hit: Option<String>,
    ) -> bool {
        match (self, hit) {
            (SelectScope::Area, Some(node)) => {
                if frame
                    .module(&node)
                    .is_some_and(|module| matches!(module, AreaModule::Blueprint(_)))
                {
                    placement::edit_in_place(editor, view_key, frame, &node);
                }
                true
            }
            (SelectScope::Blueprint { parts, .. }, Some(node)) => {
                let nested = parts
                    .iter()
                    .find(|part| part.part == node)
                    .and_then(|part| part.nested.as_deref());
                match nested {
                    Some(blueprint) => {
                        placement::enter_nested(editor, view_key, &node, blueprint);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }
}

fn start_select_drag(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    scope: &SelectScope,
    tile: Pos<Meters>,
    hit: Option<String>,
) {
    if scope.is_area() {
        let on_handle = frame.pointer.and_then(|pointer| {
            scope
                .selected(editor, frame)
                .into_iter()
                .find_map(|node| handle_at(frame, &node, pointer))
        });
        if let Some((part, corner, start)) = on_handle {
            set_drag(
                editor,
                view_key,
                AreaDrag::Resize {
                    part,
                    corner,
                    start,
                },
            );
            return;
        }
        if let Some(pin) = pins::pin_at(editor, frame, frame.pointer.unwrap_or_default()) {
            set_drag(
                editor,
                view_key,
                match pin {
                    pins::PinHit::InitialSpawn => AreaDrag::InitialSpawn,
                    pins::PinHit::Place { file, pointer } => AreaDrag::Pin { file, pointer },
                },
            );
            return;
        }
    }
    let in_folder_box = scope.is_area()
        && frame
            .folder_box(&super::selected_parts(editor, &frame.bundle))
            .is_some_and(|(bounds, _)| bounds.contains(tile));
    let hit = match hit {
        None if in_folder_box => {
            set_drag(
                editor,
                view_key,
                AreaDrag::Move {
                    from: tile,
                    moved: false,
                },
            );
            return;
        }
        hit => hit,
    };
    match hit {
        Some(node) => {
            if !scope.selected(editor, frame).contains(&node) {
                editor.selection =
                    RowSelection::parts(scope.table(frame), scope.key(frame), vec![node]);
            }
            set_drag(
                editor,
                view_key,
                AreaDrag::Move {
                    from: tile,
                    moved: false,
                },
            );
        }
        None => set_drag(editor, view_key, AreaDrag::Box { from: tile }),
    }
}

fn set_drag(editor: &mut Editor, view_key: &str, drag: AreaDrag) {
    editor.views.area.view_mut(view_key).set_drag(drag);
}

pub(super) fn track_move(view: &mut AreaViewState, tile: Pos<Meters>) {
    if let Some(AreaDrag::Move { from, moved }) = view.drag_mut()
        && (tile.x - from.x).abs() + (tile.y - from.y).abs() > 0.25
    {
        *moved = true;
    }
}

pub(super) fn toggled(
    editor: &Editor,
    table: &str,
    key: &str,
    part: String,
    modifiers: egui::Modifiers,
) -> Vec<String> {
    let toggle = EditorKey::ToggleSelection.held_among(modifiers);
    let extend = EditorKey::ExtendSelection.held_among(modifiers);
    let mut parts = kept_parts(editor, table, key, extend || toggle);
    if toggle && parts.contains(&part) {
        parts.retain(|p| *p != part);
    } else if !parts.contains(&part) {
        parts.push(part);
    }
    parts
}

pub(super) fn boxed(
    editor: &Editor,
    table: &str,
    key: &str,
    hits: impl Iterator<Item = String>,
    modifiers: egui::Modifiers,
) -> Vec<String> {
    let mut parts = kept_parts(
        editor,
        table,
        key,
        EditorKey::ExtendSelection.held_among(modifiers),
    );
    for part in hits {
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts
}

fn kept_parts(editor: &Editor, table: &str, key: &str, keep: bool) -> Vec<String> {
    let same = editor.selection.table == table && editor.selection.keys.as_slice() == [key];
    if keep && same {
        editor.selection.parts.clone()
    } else {
        Vec::new()
    }
}

pub(super) fn handle_at(
    frame: &AreaFrame,
    node: &str,
    pointer: Pos2,
) -> Option<(String, ResizeCorner, Rect<Meters>)> {
    let module = frame.module(node)?;
    if !shapes::has_size(module) {
        return None;
    }
    let bounds = frame.module_bounds(node)?;
    let rect = frame.canvas.rect(bounds);
    let corners = [
        (ResizeCorner::TopLeft, rect.min),
        (ResizeCorner::TopRight, Pos2::new(rect.max.x, rect.min.y)),
        (ResizeCorner::BottomLeft, Pos2::new(rect.min.x, rect.max.y)),
        (ResizeCorner::BottomRight, rect.max),
    ];
    corners
        .into_iter()
        .find(|(_, at)| (*at - pointer).length() <= 7.0)
        .map(|(corner, _)| (node.to_owned(), corner, bounds))
}

fn finish_select_drag(
    editor: &mut Editor,
    view_key: &str,
    scope: &SelectScope,
    frame: &AreaFrame,
    modifiers: egui::Modifiers,
) {
    let Some(drag) = editor.views.area.view_mut(view_key).take_drag() else {
        return;
    };
    let Some(tile) = frame.drag_tile() else {
        return;
    };
    match drag {
        AreaDrag::Box { from } => {
            let rect = Rect::from_points([from, tile]);
            let (hits, skipped) = scope.contained(frame, rect);
            let count = hits.len();
            let parts = boxed(
                editor,
                scope.table(frame),
                scope.key(frame),
                hits.into_iter(),
                modifiers,
            );
            editor.selection = RowSelection::parts(scope.table(frame), scope.key(frame), parts);
            skipped.tell(editor, count);
        }
        AreaDrag::Move { from, moved } => {
            if !moved {
                return;
            }
            let (dx, dy) = snapped_delta(from, tile, EditorKey::Unsnapped.held_among(modifiers));
            if dx == 0.0 && dy == 0.0 {
                return;
            }
            scope.move_selected(editor, frame, dx, dy);
        }
        AreaDrag::Resize {
            part,
            corner,
            start,
        } => {
            let (min, max) = (start.min(), start.max());
            let edge = snap_to(
                tile + Offset::new(0.5, 0.5),
                EditorKey::Unsnapped.held_among(modifiers),
            ) - Offset::new(0.5, 0.5);
            let (new_min, new_max) = match corner {
                ResizeCorner::TopLeft => (edge, max),
                ResizeCorner::TopRight => (Pos::new(min.x, edge.y), Pos::new(edge.x, max.y)),
                ResizeCorner::BottomLeft => (Pos::new(edge.x, min.y), Pos::new(max.x, edge.y)),
                ResizeCorner::BottomRight => (min, edge),
            };
            let w = (new_max.x - new_min.x).max(1.0);
            let h = (new_max.y - new_min.y).max(1.0);
            let reader = frame.reader.clone();
            nodes::change_scene(editor, &frame.bundle, |scene| {
                let module = &mut nodes::module_node_mut(scene, &part)?.module;
                shapes::shift(module, new_min.x - min.x, new_min.y - min.y);
                shapes::resize(module, w, h);
                Some(format!("Resize {}", shapes::label(module, reader.as_ref())))
            });
        }
        AreaDrag::Pin { file, pointer } => {
            let at = snap_to(tile, EditorKey::Unsnapped.held_among(modifiers));
            pins::move_pin(editor, frame, &file, &pointer, at);
        }
        AreaDrag::InitialSpawn => {
            let at = snap_to(tile, EditorKey::Unsnapped.held_among(modifiers));
            pins::set_spawn(editor, &frame.bundle, at);
        }
        AreaDrag::Stamp { .. } => {}
    }
}

pub fn snap_to(p: Pos<Meters>, fine: bool) -> Pos<Meters> {
    if fine {
        Pos::new(sixteenth(p.x), sixteenth(p.y))
    } else {
        p.snap()
    }
}

pub(super) fn snapped_delta(from: Pos<Meters>, to: Pos<Meters>, fine: bool) -> (f32, f32) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if fine {
        (sixteenth(dx), sixteenth(dy))
    } else {
        (dx.round(), dy.round())
    }
}

fn sixteenth(v: f32) -> f32 {
    (v * 16.0).round() / 16.0
}

pub fn move_selection(editor: &mut Editor, frame: &AreaFrame, dx: f32, dy: f32) {
    if let Some(place) = frame.in_place.last().cloned() {
        let parts = placement::selected_in_place(editor, &place.blueprint);
        if !parts.is_empty() {
            placement::move_parts_in_place(editor, frame, &place, &parts, dx, dy);
        }
        return;
    }
    let moved = selected_nodes(editor, frame);
    if moved.is_empty() {
        return;
    }
    let reader = frame.reader.clone();
    nodes::change_scene(editor, &frame.bundle, |scene| {
        for node in &moved {
            shapes::shift(&mut nodes::module_node_mut(scene, node)?.module, dx, dy);
        }
        Some(match moved.as_slice() {
            [one] => format!(
                "Move {}",
                shapes::label(&nodes::module_node(scene, one)?.module, reader.as_ref())
            ),
            many => format!("Move {} modules", many.len()),
        })
    });
}

pub fn copy_selection(editor: &mut Editor, frame: &AreaSnapshot) {
    let ids = scene_tree::selected_ids(frame, &editor.selection);
    scene_tree::copy_rows(editor, frame, &ids);
}

pub fn start_paste(editor: &mut Editor, view_key: &str) {
    let Some(clip) = editor
        .clipboard
        .as_ref()
        .and_then(Clipboard::scene)
        .cloned()
    else {
        editor.toast("Nothing copied");
        return;
    };
    let bundle = editor.views.area.view(view_key).bundle.clone();
    let Some((nodes, _)) = pasted(editor, &clip, &bundle) else {
        return;
    };
    let view = editor.views.area.view_mut(view_key);
    view.set_tool(AreaTool::Select);
    view.paste = Some(PasteGhost { clip, nodes });
}

pub fn paste_in_place(editor: &mut Editor, frame: &AreaFrame) {
    let Some(clip) = editor
        .clipboard
        .as_ref()
        .and_then(Clipboard::scene)
        .cloned()
    else {
        return;
    };
    paste_at(editor, frame, &clip, clip.origin);
}

pub(super) fn finish_paste(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    ghost: PasteGhost,
    tile: Pos<Meters>,
) {
    editor.views.area.view_mut(view_key).paste = None;
    paste_at(editor, frame, &ghost.clip, paste_anchor(tile));
}

pub(super) fn paste_anchor(tile: Pos<Meters>) -> Pos<Meters> {
    Pos::new(tile.x.round() - 0.5, tile.y.round() - 0.5)
}

pub(super) fn paste_at(editor: &mut Editor, frame: &AreaFrame, clip: &SceneClip, at: Pos<Meters>) {
    let Some((pasted, open)) = pasted(editor, clip, &frame.bundle) else {
        return;
    };
    let mut taken = frame.object_names();
    let placed: Vec<SceneNode> = pasted
        .into_iter()
        .map(|mut node| {
            blueprint::shift_tree(&mut node, at.x, at.y);
            scene_tree::rename_objects(&mut node, &mut taken);
            node
        })
        .collect();
    let count = placed.len();
    if nodes::add(
        editor,
        &frame.bundle,
        placed,
        &format!("Paste {count} row{}", words::plural(count)),
    ) && open > 0
    {
        toast_open(editor, open);
    }
}

/// The nodes of `clip` as they paste into `bundle`, read among the files there are now, and how
/// many holes of the blueprint they were copied from came along without a default. Pasted into
/// another bundle, a hole keeps the value it was read with: its variable's default, or else a
/// stand-in to fill in.
pub(super) fn pasted(
    editor: &mut Editor,
    clip: &SceneClip,
    bundle: &str,
) -> Option<(Vec<SceneNode>, usize)> {
    let reader = editor.reader()?;
    let variables = nodes::bundle_of(editor, &clip.from)
        .map(|from| from.variables().clone())
        .unwrap_or_default();
    let mut read = match clip.nodes.read(&reader, &variables) {
        Ok(read) => read,
        Err(problems) => {
            let said = problems
                .first()
                .map(|(pointer, problem)| format!("{pointer}: {problem}"))
                .unwrap_or_default();
            editor.toast(format!("What was copied doesn't read any more: {said}"));
            return None;
        }
    };
    let open = if clip.from == bundle {
        0
    } else {
        settle_holes(&mut read, &variables)
    };
    Some((read, open))
}

pub(super) fn toast_open(editor: &mut Editor, open: usize) {
    editor.toast(format!(
        "{open} value{} the blueprint left open came along as a stand-in: fill {} in",
        words::plural(open),
        if open == 1 { "it" } else { "them" }
    ));
}

/// Fixes every hole of `nodes` to the value it was read with; answers how many had no default.
fn settle_holes(nodes: &mut [SceneNode], variables: &BTreeMap<String, Variable>) -> usize {
    let mut open = 0;
    for node in nodes {
        match node {
            SceneNode::Folder(folder) => open += settle_holes(&mut folder.children, variables),
            SceneNode::Module(module) => {
                open += module
                    .holes
                    .0
                    .values()
                    .filter(|name| variables.get(*name).is_none_or(|v| v.default.is_none()))
                    .count();
                module.holes.0.clear();
            }
        }
    }
    open
}

pub fn duplicate_selection(editor: &mut Editor, frame: &AreaSnapshot) {
    let ids = scene_tree::selected_ids(frame, &editor.selection);
    scene_tree::duplicate_rows(editor, frame, &ids);
}

pub fn delete_selection(editor: &mut Editor, frame: &AreaSnapshot) {
    let ids = scene_tree::selected_ids(frame, &editor.selection);
    scene_tree::delete_rows(editor, frame, &ids);
}

/// Selects every module of the scene the map lets the pointer at, passing over what is hidden or
/// locked.
pub fn select_all(editor: &mut Editor, frame: &AreaFrame) {
    let mut skipped = Skipped::default();
    let (bundle, parts): (String, Vec<String>) = match frame.in_place.last() {
        Some(place) => (
            place.blueprint.clone(),
            placement::parts_in_place(frame, place)
                .into_iter()
                .filter(|part| {
                    skipped.locked += usize::from(part.locked);
                    !part.locked
                })
                .map(|part| part.part)
                .collect(),
        ),
        None => (
            frame.bundle.clone(),
            frame
                .modules
                .iter()
                .filter(|module| super::is_selectable(module.module()))
                .filter(|module| skipped.passes(&frame.flags.row(&module.pointer)))
                .map(|module| module.pointer.clone())
                .collect(),
        ),
    };
    let count = parts.len();
    nodes::select(editor, &bundle, parts);
    skipped.tell(editor, count);
}

/// What a selection over many modules passed over.
#[derive(Default)]
struct Skipped {
    hidden: usize,
    locked: usize,
}

impl Skipped {
    fn passes(&mut self, flags: &RowFlags) -> bool {
        if flags.hidden.on() {
            self.hidden += 1;
        } else if flags.locked.on() {
            self.locked += 1;
        }
        !flags.hidden.on() && !flags.locked.on()
    }

    fn tell(&self, editor: &mut Editor, selected: usize) {
        let said: Vec<String> = [(self.locked, "locked"), (self.hidden, "hidden")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, what)| format!("{count} {what}"))
            .collect();
        if !said.is_empty() {
            editor.toast(format!(
                "Selected {selected}, {} skipped",
                said.join(" and ")
            ));
        }
    }
}
