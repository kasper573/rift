//! A tree of rows that draws only the rows in view, however many it lists: a flat list of lines,
//! each knowing how deep it lies and whether it is an open or closed folder, laid out a row each.
//! The tree brings selection by click, range and toggle, keyboard navigation, opening and closing
//! folders, dragging rows to a spot between or into others, a menu on the rows or on the room
//! below them, and scrolling a row into view. What a row shows is drawn by the caller.

use std::collections::HashMap;

use egui::style::ScrollAnimation;
use egui::{
    Align, Event, EventFilter, Id, Key, Layout, Modifiers, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2,
};

use crate::glyph::Mark;
use crate::{ink, keys, theme};

/// A row as the tree lists it: what it stands for, how many folders deep it lies, and whether it
/// is a folder and open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeLine {
    pub id: String,
    pub depth: usize,
    pub folder: bool,
    pub open: bool,
}

/// The lines a tree lists, in order, each found by its id at once.
#[derive(Clone, Debug, Default)]
pub struct TreeList {
    lines: Vec<TreeLine>,
    at: HashMap<String, usize>,
}

impl TreeList {
    pub fn new(lines: Vec<TreeLine>) -> TreeList {
        let at = lines
            .iter()
            .enumerate()
            .map(|(index, line)| (line.id.clone(), index))
            .collect();
        TreeList { lines, at }
    }

    pub fn lines(&self) -> &[TreeLine] {
        &self.lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Where the line of `id` lies among the lines.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.at.get(id).copied()
    }

    pub fn get(&self, id: &str) -> Option<&TreeLine> {
        self.index_of(id).map(|index| &self.lines[index])
    }

    /// The folder line `id` lies in, the nearest line above it one folder shallower.
    pub fn parent_of(&self, id: &str) -> Option<&TreeLine> {
        let index = self.index_of(id)?;
        let depth = self.lines[index].depth.checked_sub(1)?;
        self.lines[..index]
            .iter()
            .rev()
            .find(|line| line.depth == depth)
    }
}

/// Where dragged rows would land: before or after a row, into a folder, or below every row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropSpot {
    Before(String),
    Into(String),
    After(String),
    End,
}

/// A tree's own state between frames: what is selected, the row a range is picked from, the row
/// the keys move from, a row to scroll into view, what is being dragged, and where it scrolled.
#[derive(Clone, Debug, Default)]
pub struct TreeState {
    selected: Vec<String>,
    anchor: Option<String>,
    cursor: Option<String>,
    /// A row to scroll into view, and where in the view it goes: none to scroll no further than
    /// it takes.
    scroll_to: Option<(String, Option<Align>)>,
    drag: Option<Drag>,
    /// How tall a stretch of the rows showed, last frame.
    viewport: f32,
}

#[derive(Clone, Debug)]
struct Drag {
    rows: Vec<String>,
    cancelled: bool,
    /// The closed folder the drag has rested on, and since when.
    resting: Option<(String, f64)>,
}

impl TreeState {
    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    /// Selects `ids`, the last of them where the keys move from.
    pub fn select(&mut self, ids: Vec<String>) {
        self.cursor = ids.last().cloned();
        self.anchor = self.cursor.clone();
        self.selected = ids;
    }

    /// Scrolls the row of `id` to the middle of the tree at the next frame it is listed in.
    pub fn scroll_to(&mut self, id: &str) {
        self.scroll_to = Some((id.to_owned(), Some(Align::Center)));
    }

    /// Whether rows are being dragged.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

/// How a row is drawn this frame: selected, under the pointer, or where the keys move from while
/// the tree has the keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLook {
    pub selected: bool,
    pub hovered: bool,
    pub cursor: bool,
}

/// What the user did to the tree this frame.
#[derive(Debug)]
pub struct TreeOutput {
    /// The selection, when the user changed it.
    pub selected: Option<Vec<String>>,
    /// Rows opened with Enter or a double click.
    pub activated: Vec<String>,
    /// Folders the user opened or closed, each with whether it is now open.
    pub opened: Vec<(String, bool)>,
    /// The rows being dragged and where they would land, while the pointer is over the tree.
    pub dragging: Option<(Vec<String>, DropSpot)>,
    /// The rows being dragged, while the pointer is past the tree.
    pub dragged_out: Option<Vec<String>>,
    /// Rows dropped on the tree this frame, and where.
    pub dropped: Option<(Vec<String>, DropSpot)>,
    pub hovered: Option<String>,
    /// Whether the keys go to the tree.
    pub focused: bool,
    /// Every row drawn this frame, with where.
    pub rows: Vec<(String, Rect)>,
    /// The room the tree took on screen.
    pub rect: Rect,
}

/// A tree over `list`, drawn by row: each `height` tall, a folder's rows deeper by `indent`, and
/// below the rows `room` more rows' worth of empty room, whose menu is the tree's own.
#[derive(Clone, Copy)]
pub struct VirtualTree<'a> {
    id: Id,
    list: &'a TreeList,
    height: f32,
    indent: f32,
    room: usize,
}

impl<'a> VirtualTree<'a> {
    pub fn new(id: Id, list: &'a TreeList) -> VirtualTree<'a> {
        VirtualTree {
            id,
            list,
            height: 22.0,
            indent: 14.0,
            room: 3,
        }
    }

    pub fn row_height(self, height: f32) -> Self {
        VirtualTree { height, ..self }
    }

    pub fn indent(self, indent: f32) -> Self {
        VirtualTree { indent, ..self }
    }

    pub fn empty_room(self, rows: usize) -> Self {
        VirtualTree { room: rows, ..self }
    }

    /// Where the room a row of `depth` gives what it shows starts, from the row's left edge.
    pub fn content_left(&self, depth: usize) -> f32 {
        depth as f32 * self.indent + CARET
    }

    /// Draws the rows in view, each by `row` in the room right of its caret, and the menu `menu`
    /// on what a right click picks: the rows selected, or none on the room below them.
    pub fn show(
        self,
        ui: &mut Ui,
        state: &mut TreeState,
        mut row: impl FnMut(&mut Ui, &TreeLine, RowLook),
        mut menu: impl FnMut(&mut Ui, &[String]),
    ) -> TreeOutput {
        let VirtualTree {
            id,
            list,
            height,
            indent,
            room,
        } = self;
        ui.interact(
            ui.available_rect_before_wrap(),
            id,
            Sense::focusable_noninteractive(),
        );
        let mut out = TreeOutput {
            selected: None,
            activated: Vec::new(),
            opened: Vec::new(),
            dragging: None,
            dragged_out: None,
            dropped: None,
            hovered: None,
            focused: ui.memory(|memory| memory.has_focus(id)),
            rows: Vec::new(),
            rect: Rect::NOTHING,
        };
        if out.focused {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
            keys_ui(ui, list, state, height, &mut out);
        }
        let scroll = egui::ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false]);
        let asked = state
            .scroll_to
            .take()
            .and_then(|(target, align)| list.index_of(&target).map(|index| (index, align)));
        let hover = ui.input(|input| input.pointer.hover_pos());
        let shown = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                scroll.show_rows(ui, height, list.len() + room, |ui, range| {
                    if let Some((index, align)) = asked {
                        let first = ui.cursor().top() - range.start as f32 * height;
                        let row = Rect::from_min_size(
                            Pos2::new(ui.cursor().left(), first + index as f32 * height),
                            Vec2::new(1.0, height),
                        );
                        ui.scroll_to_rect_animation(row, align, ScrollAnimation::none());
                        ui.ctx().request_repaint();
                    }
                    let seen = ui.clip_rect();
                    for index in range {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), height),
                            Sense::hover(),
                        );
                        let Some(line) = list.lines().get(index) else {
                            let response =
                                ui.interact(rect, id.with(("room", index)), Sense::click());
                            response.context_menu(|ui| menu(ui, &[]));
                            continue;
                        };
                        let response =
                            ui.interact(rect, id.with(("row", &line.id)), Sense::click_and_drag());
                        let hovered =
                            hover.is_some_and(|at| rect.contains(at) && seen.contains(at));
                        let look = RowLook {
                            selected: state.selected.contains(&line.id),
                            hovered: hovered && state.drag.is_none(),
                            cursor: out.focused
                                && state.cursor.as_deref() == Some(line.id.as_str()),
                        };
                        paint_row(ui, rect, look);
                        let caret = Rect::from_min_size(
                            rect.min + Vec2::new(line.depth as f32 * indent, 0.0),
                            Vec2::new(CARET, height),
                        );
                        let mut inside = ui.new_child(
                            UiBuilder::new()
                                .max_rect(Rect::from_min_max(
                                    Pos2::new(caret.right(), rect.top()),
                                    rect.max,
                                ))
                                .layout(Layout::left_to_right(Align::Center)),
                        );
                        row(&mut inside, line, look);
                        let toggled = line.folder && caret_ui(ui, id, line, caret);
                        if toggled {
                            out.opened.push((line.id.clone(), !line.open));
                        } else {
                            pressed(ui, id, list, state, line, &response, &mut out);
                        }
                        response.context_menu(|ui| menu(ui, &state.selected));
                        if hovered {
                            out.hovered = Some(line.id.clone());
                        }
                        out.rows.push((line.id.clone(), rect));
                    }
                })
            })
            .inner;
        out.rect = shown.inner_rect;
        state.viewport = shown.inner_rect.height();
        dragged(ui, list, state, height, &mut out);
        out
    }
}

/// The room a folder's caret takes, left of what the row shows.
const CARET: f32 = 16.0;
/// How long a drag rests on a closed folder before it opens.
const RESTING_OPENS: f64 = 0.5;

fn paint_row(ui: &Ui, rect: Rect, look: RowLook) {
    if look.selected {
        ui.painter().rect_filled(rect, 3.0, theme::SELECTION);
    } else if look.hovered {
        ui.painter()
            .rect_filled(rect, 3.0, ink::wash(theme::SELECTION, 0.4));
    }
    if look.cursor {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            3.0,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );
    }
}

/// Draws a folder's caret: whether it was clicked.
fn caret_ui(ui: &mut Ui, id: Id, line: &TreeLine, caret: Rect) -> bool {
    let response = ui.interact(caret, id.with(("caret", &line.id)), Sense::click());
    let mark = if line.open {
        Mark::CaretDown
    } else {
        Mark::CaretRight
    };
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::FAINT
    };
    mark.paint(
        ui.painter(),
        Rect::from_center_size(caret.center(), Vec2::splat(10.0)),
        color,
    );
    response.clicked()
}

/// Takes a press on `line`'s row: a click selects it, alone, toggled with the command key or as
/// the range from the anchor with shift; a plain click opens or closes a folder; a double click
/// activates; a right click on a row not selected selects it alone; a drag starts dragging the
/// selection, or the row alone when it isn't selected.
fn pressed(
    ui: &Ui,
    id: Id,
    list: &TreeList,
    state: &mut TreeState,
    line: &TreeLine,
    response: &Response,
    out: &mut TreeOutput,
) {
    let modifiers = ui.input(|input| input.modifiers);
    if response.clicked() || response.secondary_clicked() || response.drag_started() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    if response.double_clicked() {
        out.activated = state.selected.clone();
        return;
    }
    if response.clicked() {
        let picked = picked(list, state, &line.id, modifiers);
        if modifiers.is_none() && line.folder {
            out.opened.push((line.id.clone(), !line.open));
        }
        changed(state, picked, &line.id, !modifiers.shift, out);
    }
    if response.secondary_clicked() && !state.selected.contains(&line.id) {
        changed(state, vec![line.id.clone()], &line.id, true, out);
    }
    if response.drag_started() {
        if !state.selected.contains(&line.id) {
            changed(state, vec![line.id.clone()], &line.id, true, out);
        }
        state.drag = Some(Drag {
            rows: state.selected.clone(),
            cancelled: false,
            resting: None,
        });
    }
}

/// What a click on `id` with `modifiers` selects.
fn picked(list: &TreeList, state: &TreeState, id: &str, modifiers: Modifiers) -> Vec<String> {
    if modifiers.shift
        && let Some(from) = state.anchor.as_deref().and_then(|from| list.index_of(from))
        && let Some(to) = list.index_of(id)
    {
        let (low, high) = (from.min(to), from.max(to));
        return list.lines()[low..=high]
            .iter()
            .map(|line| line.id.clone())
            .collect();
    }
    if modifiers.command {
        let mut selected = state.selected.clone();
        match selected.iter().position(|held| held == id) {
            Some(at) => {
                selected.remove(at);
            }
            None => selected.push(id.to_owned()),
        }
        return selected;
    }
    vec![id.to_owned()]
}

/// Makes `selected` the selection, the keys moving from `cursor` and a range picked from it next
/// when `anchored`.
fn changed(
    state: &mut TreeState,
    selected: Vec<String>,
    cursor: &str,
    anchored: bool,
    out: &mut TreeOutput,
) {
    state.cursor = Some(cursor.to_owned());
    if anchored {
        state.anchor = Some(cursor.to_owned());
    }
    if selected != state.selected {
        state.selected = selected.clone();
        out.selected = Some(selected);
    }
}

/// Takes the keys while the tree has them: up and down move the selection, with shift a range;
/// home and end, page up and down go further; left closes a folder or goes up to the one holding
/// the row, right opens a folder or goes into it; confirming activates the selection.
fn keys_ui(ui: &Ui, list: &TreeList, state: &mut TreeState, height: f32, out: &mut TreeOutput) {
    let pressed: Vec<(Key, Modifiers)> = ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    let page = (state.viewport / height).max(1.0) as usize;
    for (key, modifiers) in pressed {
        let at = state
            .cursor
            .as_deref()
            .and_then(|cursor| list.index_of(cursor));
        let last = list.len().saturating_sub(1);
        let to = match key {
            Key::ArrowDown => Some(at.map_or(0, |at| (at + 1).min(last))),
            Key::ArrowUp => Some(at.map_or(0, |at| at.saturating_sub(1))),
            Key::Home => Some(0),
            Key::End => Some(last),
            Key::PageDown => Some(at.map_or(0, |at| (at + page).min(last))),
            Key::PageUp => Some(at.map_or(0, |at| at.saturating_sub(page))),
            Key::ArrowLeft => at.and_then(|at| {
                let line = &list.lines()[at];
                if line.folder && line.open {
                    out.opened.push((line.id.clone(), false));
                    None
                } else {
                    list.parent_of(&line.id)
                        .and_then(|parent| list.index_of(&parent.id))
                }
            }),
            Key::ArrowRight => at.and_then(|at| {
                let line = &list.lines()[at];
                match (line.folder, line.open) {
                    (true, false) => {
                        out.opened.push((line.id.clone(), true));
                        None
                    }
                    (true, true) => list
                        .lines()
                        .get(at + 1)
                        .filter(|next| next.depth > line.depth)
                        .map(|_| at + 1),
                    _ => None,
                }
            }),
            key if key == keys::CONFIRM.logical_key
                && modifiers.matches_exact(keys::CONFIRM.modifiers) =>
            {
                out.activated = state.selected.clone();
                None
            }
            _ => None,
        };
        let Some(line) = to.and_then(|to| list.lines().get(to)) else {
            continue;
        };
        let selected = if modifiers.shift {
            picked(list, state, &line.id, Modifiers::SHIFT)
        } else {
            vec![line.id.clone()]
        };
        changed(state, selected, &line.id, !modifiers.shift, out);
        state.scroll_to = Some((line.id.clone(), None));
    }
}

/// Follows a drag: where the rows would land under the pointer, a closed folder opening once the
/// drag rests on it, and the drop once the pointer lets go; cancelling calls the drag off.
fn dragged(ui: &Ui, list: &TreeList, state: &mut TreeState, height: f32, out: &mut TreeOutput) {
    let Some(drag) = state.drag.as_mut() else {
        return;
    };
    let (pointer, released, cancel, now) = ui.input(|input| {
        (
            input.pointer.latest_pos(),
            input.pointer.any_released(),
            keys::pressed(input, keys::CANCEL),
            input.time,
        )
    });
    drag.cancelled |= cancel;
    let spot = pointer
        .filter(|pointer| out.rect.contains(*pointer))
        .map(|pointer| spot_at(list, &out.rows, pointer, height));
    if !drag.cancelled {
        match &spot {
            Some(spot) => out.dragging = Some((drag.rows.clone(), spot.clone())),
            None => out.dragged_out = Some(drag.rows.clone()),
        }
        ui.ctx().request_repaint();
    }
    let resting_on = match &spot {
        Some(DropSpot::Into(folder)) if list.get(folder).is_some_and(|line| !line.open) => {
            Some(folder.clone())
        }
        _ => None,
    };
    match (&drag.resting, resting_on) {
        (Some((held, since)), Some(on)) if *held == on => {
            if now - since >= RESTING_OPENS {
                out.opened.push((on, true));
                drag.resting = None;
            }
        }
        (_, on) => drag.resting = on.map(|on| (on, now)),
    }
    if released {
        let drag = state.drag.take().expect("a drag being followed");
        if !drag.cancelled
            && let Some(spot) = spot
        {
            out.dropped = Some((drag.rows, spot));
        }
    }
}

/// Where rows dropped at `pointer` land: before a row in its top quarter (half, for a row that
/// isn't a folder), after it in its bottom one, into a folder between, and below every row in the
/// room past them.
fn spot_at(list: &TreeList, rows: &[(String, Rect)], pointer: Pos2, height: f32) -> DropSpot {
    let Some((id, rect)) = rows
        .iter()
        .find(|(_, rect)| rect.y_range().contains(pointer.y))
    else {
        return DropSpot::End;
    };
    let share = (pointer.y - rect.top()) / height.max(1.0);
    let folder = list.get(id).is_some_and(|line| line.folder);
    match (folder, share) {
        (true, share) if share < 0.25 => DropSpot::Before(id.clone()),
        (true, share) if share > 0.75 => DropSpot::After(id.clone()),
        (true, _) => DropSpot::Into(id.clone()),
        (false, share) if share < 0.5 => DropSpot::Before(id.clone()),
        (false, _) => DropSpot::After(id.clone()),
    }
}
