use std::cell::{OnceCell, RefCell};
use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;
use ui::chip;
use ui::drag;
use ui::egui;
use ui::ink;
use ui::paint;
use ui::theme;

use ui::controls;
use ui::egui::{
    Align, Align2, Color32, FontId, Id, Layout, Pos2, Rangef, Rect, Stroke, StrokeKind, Ui, Vec2,
};
use ui::egui_ltreeview::{
    Action, CloserState, DirPosition, IndentHintStyle, NodeBuilder, RowLayout, TreeView,
    TreeViewBuilder, TreeViewState,
};
use ui::glyph::Mark;

use super::widgets::{self, KindMark};
use crate::icons::{self, IconTone, KindIcon, Preview};
use crate::keymap::EditorKey;
use crate::picker::Dragged;
use crate::widgets::Badge;

/// What a tree lists and what may be done to its rows. The game folder and a scene each implement
/// it; the tree brings the behaviour, the keys and the looks once.
pub trait TreeSource {
    type Action;
    /// The rows inside the folder `id` as they are shown, in order; the top of the tree is "".
    fn children(&self, id: &str) -> Vec<String>;
    /// The folder holding `id`, "" at the top.
    fn parent(&self, id: &str) -> String;
    fn is_folder(&self, id: &str) -> bool;
    fn row(&self, id: &str) -> TreeRow;
    /// The menu on `ids`, or on empty space when there are none; the action picked.
    fn menu(&self, ui: &mut Ui, ids: &[String]) -> Option<Self::Action>;
    /// What dropping `dragged` at `drop` does, or why it is refused.
    fn drop_tip(&self, dragged: &[String], drop: &TreeDrop) -> Result<String, String>;
    /// Whether rows keep an order of their own, so a drop may land between two of them.
    fn ordered(&self) -> bool;
    /// Whether the rows shown are narrowed to what matches, every folder opening on what it holds.
    fn narrowed(&self) -> bool;
    /// What the pointer carries out of the tree when `ids` are dragged out of it.
    fn carried(&self, ids: &[String]) -> Option<Dragged>;
    /// What renaming `id` touches beyond itself, said under its edit box.
    fn rename_tip(&self, id: &str) -> Option<String>;
}

/// How one row reads.
#[derive(Clone, Debug, Default)]
pub struct TreeRow {
    pub folder: bool,
    /// The row's icon; a folder's is the folder's own.
    pub icon: Option<KindIcon>,
    /// A picture of what the row holds, drawn in place of its icon.
    pub preview: Option<Preview>,
    pub name: String,
    /// Read after the name in its own colour, such as a content file's type.
    pub suffix: Option<(String, Color32)>,
    /// Read at the right end, such as a folder's count.
    pub detail: String,
    /// Greyed: a folder only shown for what it holds, or a row that isn't there yet.
    pub dim: bool,
    /// What is wrong with the row, or with something inside a folder.
    pub problem: Option<String>,
    pub unsaved: bool,
    /// Why the row can't move, when it can't.
    pub locked: Option<String>,
    /// What assistive tech reads for the row, when more than its name.
    pub spoken: Option<String>,
    /// A letter in a coloured square in place of the icon, as the editor marks module kinds.
    pub badge: Option<Badge>,
    /// The row's eye and padlock, when it has them.
    pub toggles: Option<RowToggles>,
    /// A closed folder with something selected inside.
    pub holds_selected: bool,
    /// Lit for what the pointer is over elsewhere, such as on the map.
    pub lit: bool,
    /// The part of the name a search matched.
    pub matched: Option<Range<usize>>,
    /// Said when the pointer rests on the row, after its problem.
    pub tip: Option<String>,
}

/// A row's hide and lock, each off, set on the row, or set by a folder above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowToggles {
    pub hidden: ToggleState,
    pub locked: ToggleState,
    /// Shown but not pressable; the row's tip says why.
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToggleState {
    #[default]
    Off,
    Own,
    Inherited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Hide,
    Lock,
}

/// A press on a row's eye or padlock, and whether Show only was held.
#[derive(Clone, Debug)]
pub struct Toggled {
    pub id: String,
    pub toggle: Toggle,
    pub only: bool,
}

/// Where dragged rows land: in `folder`, at `at` among what it holds. `onto` is the row that
/// isn't a folder the pointer rests on the middle of, which a source may refuse to drop onto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDrop {
    pub folder: String,
    pub at: DropAt,
    pub onto: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropAt {
    First,
    End,
    Before(String),
    After(String),
}

/// A tree's own state between frames: what is selected and open, and what it was asked to do.
#[derive(Default)]
pub struct TreeState {
    view: TreeViewState<String>,
    /// Openness while the rows are narrowed, kept apart so a search never closes a folder.
    narrowed_view: TreeViewState<String>,
    reveal: Option<String>,
    renaming: Option<Renaming>,
    flash: Option<(String, f64)>,
    drag_hover: Option<(String, f64)>,
    drag_cancelled: bool,
}

struct Renaming {
    id: String,
    text: String,
    focused: bool,
}

impl TreeState {
    pub fn selected(&self) -> &[String] {
        self.view.selected()
    }

    pub fn select(&mut self, ids: Vec<String>) {
        self.narrowed_view.set_selected(ids.clone());
        self.view.set_selected(ids);
    }

    /// Opens the folders above `id`, selects it, scrolls it into view and flashes it.
    pub fn reveal(&mut self, id: &str) {
        self.reveal = Some(id.to_owned());
    }

    /// Puts `name` in an edit box on `id`'s row, ready to type over.
    pub fn begin_rename(&mut self, id: &str, name: &str) {
        self.reveal(id);
        self.renaming = Some(Renaming {
            id: id.to_owned(),
            text: name.to_owned(),
            focused: false,
        });
    }

    pub fn renaming(&self) -> Option<&str> {
        self.renaming.as_ref().map(|renaming| renaming.id.as_str())
    }

    /// Forgets which folders the tree holds open and what it has selected, for rows whose ids
    /// now name other things.
    pub fn forget(&mut self) {
        self.view = TreeViewState::default();
        self.narrowed_view = TreeViewState::default();
    }

    /// Opens or closes every folder among `ids`.
    pub fn set_open(&mut self, ids: impl IntoIterator<Item = String>, open: bool) {
        for id in ids {
            self.view.set_openness(id.clone(), open);
            self.narrowed_view.set_openness(id, open);
        }
    }
}

/// What the user did to the tree this frame.
pub struct TreeResponse<A> {
    /// The selection, when the user changed it.
    pub selected: Option<Vec<String>>,
    /// Rows opened with Enter or a double click.
    pub activated: Vec<String>,
    pub action: Option<A>,
    pub dropped: Option<(Vec<String>, TreeDrop)>,
    /// A row renamed in place, with the name typed.
    pub renamed: Option<(String, String)>,
    pub toggled: Option<Toggled>,
    /// Whether the keys go to the tree.
    pub focused: bool,
}

/// Draws the tree of `source`. `open` holds the folders open while nothing narrows the rows, kept
/// by the caller; it answers whether that changed.
pub fn show<S: TreeSource>(
    ui: &mut Ui,
    id: Id,
    state: &mut TreeState,
    source: &S,
    open: &mut BTreeSet<String>,
) -> (TreeResponse<S::Action>, bool) {
    let narrowed = source.narrowed();
    if let Some(target) = &state.reveal {
        let mut parent = source.parent(target);
        while !parent.is_empty() {
            state.view.set_openness(parent.clone(), true);
            state.narrowed_view.set_openness(parent.clone(), true);
            parent = source.parent(&parent);
        }
        state.select(vec![target.clone()]);
    }
    let TreeState {
        view,
        narrowed_view,
        reveal,
        renaming,
        flash,
        drag_hover,
        drag_cancelled,
    } = state;
    let view = if narrowed { narrowed_view } else { view };
    let lines = lines_of(source, "", &|id| {
        view.is_open(&id.to_owned())
            .unwrap_or(narrowed || open.contains(id))
    });
    let mut folders: Vec<(String, bool)> = Vec::new();
    let mut revealed_at = None;
    let mut index = 0;
    walk_lines(&lines, &mut |line| {
        if reveal.as_deref() == Some(line.id.as_str()) {
            revealed_at = Some(index);
        }
        if line.folder {
            folders.push((line.id.clone(), line.open));
        }
        index += 1;
    });

    let painter = ui.painter().clone();
    let top = ui.cursor().min.y;
    let drawn: RefCell<Vec<DrawnRow>> = RefCell::new(Vec::new());
    let renamed: RefCell<Option<RenameEnd>> = RefCell::new(None);
    let toggled: RefCell<Option<Toggled>> = RefCell::new(None);
    let picked: RefCell<Option<S::Action>> = RefCell::new(None);
    let rename_tip = renaming
        .as_ref()
        .and_then(|renaming| source.rename_tip(&renaming.id));
    let renaming_cell = RefCell::new(renaming.take());
    // The tree fills the room it is given, however few its rows, so it is given the panel less the
    // room kept below it, which then fits too until the rows outgrow the panel.
    let rows_room = egui::vec2(
        ui.available_width(),
        (ui.available_height() - EMPTY_ROOM - ui.spacing().item_spacing.y).max(0.0),
    );
    let (response, actions) = ui
        .allocate_ui(rows_room, |ui| {
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = theme::SELECTION;
            // The tree fades the selection to 0.3 of this while it lacks focus; the selection reads
            // the same either way.
            visuals.widgets.inactive.weak_bg_fill = theme::SELECTION.linear_multiply(1.0 / 0.3);
            TreeView::new(id)
                .row_layout(RowLayout::AlignedIconsAndLabels)
                .indent_hint_style(IndentHintStyle::Line)
                .override_indent(Some(INDENT))
                .default_node_height(Some(ROW))
                .fallback_context_menu(|ui, ids| {
                    if let Some(action) = source.menu(ui, ids) {
                        *picked.borrow_mut() = Some(action);
                    }
                })
                .show_state(ui, view, |builder| {
                    builder.node(NodeBuilder::dir(String::new()).flatten(true));
                    let rows = RowsCx {
                        row: &|id| source.row(id),
                        drawn: &drawn,
                        renaming: &renaming_cell,
                        renamed: &renamed,
                        toggled: &toggled,
                        rename_tip: rename_tip.as_deref(),
                    };
                    build(builder, lines, &rows);
                    builder.close_dir();
                })
        })
        .inner;
    let room = ui.allocate_response(
        egui::vec2(ui.available_width(), EMPTY_ROOM),
        egui::Sense::click(),
    );
    room.context_menu(|ui| {
        if let Some(action) = source.menu(ui, &[]) {
            *picked.borrow_mut() = Some(action);
        }
    });
    *renaming = renaming_cell.into_inner();
    let drawn = drawn.into_inner();
    let now = ui.input(|i| i.time);
    let mut out = TreeResponse {
        selected: None,
        activated: Vec::new(),
        action: picked.into_inner(),
        dropped: None,
        renamed: None,
        toggled: toggled.into_inner(),
        focused: ui.memory(|m| m.has_focus(id)),
    };
    match renamed.into_inner() {
        Some(RenameEnd::Typed(id, text)) => {
            out.renamed = Some((id, text));
            *renaming = None;
        }
        Some(RenameEnd::Left) => *renaming = None,
        None => {}
    }
    if let Some(revealed) = reveal.take() {
        if let Some(index) = revealed_at {
            let row_height = ROW + ui.spacing().item_spacing.y;
            let y = top + index as f32 * row_height;
            let rect = Rect::from_x_y_ranges(response.rect.x_range(), Rangef::new(y, y + ROW));
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
        *flash = Some((revealed, now));
    }
    if let Some((flashed, since)) = flash.clone() {
        let fade = 1.0 - ((now - since) / FLASH) as f32;
        match drawn.iter().find(|row| row.id == flashed) {
            Some(row) if fade > 0.0 => {
                painter.rect_filled(
                    row.rect(&response.rect),
                    3.0,
                    ink::wash(theme::DROP, fade * 0.35),
                );
                ui.ctx().request_repaint();
            }
            _ if fade <= 0.0 => *flash = None,
            _ => {}
        }
    }

    let escape = ui.input(|i| EditorKey::Cancel.pressed(i));
    let mut dragging = false;
    for action in actions {
        match action {
            Action::SetSelected(ids) => out.selected = Some(ids),
            Action::Activate(activate) => out.activated = activate.selected,
            Action::Drag(drag) => {
                dragging = true;
                drag.remove_drop_marker(ui);
                if escape {
                    *drag_cancelled = true;
                }
                if *drag_cancelled {
                    continue;
                }
                let pointer = ui.input(|i| i.pointer.latest_pos()).unwrap_or_default();
                let drop = drop_of(source, &drawn, pointer, &drag.target, &drag.position);
                match source.drop_tip(&drag.source, &drop) {
                    Ok(tip) => {
                        mark_drop(&painter, source, &drawn, &response.rect, &drop);
                        drag_tip(ui, pointer, &tip, theme::TEXT);
                    }
                    Err(refusal) => {
                        if let Some(row) = drawn.iter().find(|row| row.contains(pointer)) {
                            painter.rect_stroke(
                                row.rect(&response.rect),
                                3.0,
                                Stroke::new(1.5, theme::BLOCKED),
                                StrokeKind::Inside,
                            );
                        }
                        drag_tip(ui, pointer, &refusal, theme::BLOCKED);
                    }
                }
                open_on_hover(view, drag_hover, &drawn, pointer, now);
                ui.ctx().request_repaint();
            }
            Action::Move(drag) => {
                if !std::mem::take(drag_cancelled) {
                    let pointer = ui.input(|i| i.pointer.latest_pos()).unwrap_or_default();
                    let drop = drop_of(source, &drawn, pointer, &drag.target, &drag.position);
                    if source.drop_tip(&drag.source, &drop).is_ok() {
                        out.dropped = Some((drag.source, drop));
                    }
                }
            }
            Action::DragExternal(drag) => {
                dragging = true;
                if let Some(carried) = source.carried(&drag.source) {
                    drag_tip(ui, drag.position, &carried.label(), theme::TEXT);
                    drag::carry(ui.ctx(), carried);
                }
            }
            Action::MoveExternal(_) => *drag_cancelled = false,
        }
    }
    if !dragging {
        *drag_hover = None;
    }

    let clicked = response.clicked() && ui.input(|i| i.modifiers.is_none());
    if clicked
        && out.toggled.is_none()
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        && let Some(row) = drawn.iter().find(|row| row.contains(pointer))
        && row.folder
        && renaming.as_ref().is_none_or(|renaming| renaming.id != row.id)
        // The tree toggles a folder itself when its closer is clicked.
        && view.is_open(&row.id).unwrap_or(row.open) == row.open
    {
        view.set_openness(row.id.clone(), !row.open);
    }
    let mut changed = false;
    if !narrowed {
        for (folder, was_open) in folders {
            let now_open = view.is_open(&folder).unwrap_or(was_open);
            changed |= if now_open {
                open.insert(folder)
            } else {
                open.remove(&folder)
            };
        }
    }
    (out, changed)
}

const ROW: f32 = 22.0;
/// The room a tree keeps below its last row, so its menu of empty space, where top-level files
/// are made, stays in reach however long the tree grows.
const EMPTY_ROOM: f32 = 3.0 * ROW;
const INDENT: f32 = 14.0;
const FLASH: f64 = 0.9;
const HOVER_OPENS: f64 = 0.5;
const MARK: f32 = 14.0;
const ICON: f32 = 15.0;
const ICON_GAP: f32 = 2.0;
const TOGGLE: f32 = 17.0;
const TOGGLE_GLYPH: f32 = 14.0;
const SHOW_MENU_WIDTH: f32 = 220.0;

/// A row's place in the tree. What it reads is asked of the source only once it is drawn, as a
/// tree may hold far more rows than fit on screen.
struct Line {
    id: String,
    folder: bool,
    open: bool,
    children: Vec<Line>,
}

fn lines_of<S: TreeSource>(source: &S, folder: &str, is_open: &dyn Fn(&str) -> bool) -> Vec<Line> {
    source
        .children(folder)
        .into_iter()
        .map(|id| {
            let folder = source.is_folder(&id);
            let open = folder && is_open(&id);
            let children = if open {
                lines_of(source, &id, is_open)
            } else {
                Vec::new()
            };
            Line {
                id,
                folder,
                open,
                children,
            }
        })
        .collect()
}

fn walk_lines(lines: &[Line], visit: &mut dyn FnMut(&Line)) {
    for line in lines {
        visit(line);
        walk_lines(&line.children, visit);
    }
}

/// A row as it was drawn this frame.
struct DrawnRow {
    id: String,
    folder: bool,
    open: bool,
    y: Rangef,
    /// Where the row's name starts.
    left: f32,
}

impl DrawnRow {
    fn rect(&self, tree: &Rect) -> Rect {
        Rect::from_x_y_ranges(tree.x_range(), self.y)
    }

    fn contains(&self, pointer: Pos2) -> bool {
        self.y.contains(pointer.y)
    }
}

enum RenameEnd {
    Typed(String, String),
    Left,
}

struct RowsCx<'a> {
    row: &'a dyn Fn(&str) -> TreeRow,
    drawn: &'a RefCell<Vec<DrawnRow>>,
    renaming: &'a RefCell<Option<Renaming>>,
    renamed: &'a RefCell<Option<RenameEnd>>,
    toggled: &'a RefCell<Option<Toggled>>,
    rename_tip: Option<&'a str>,
}

fn build(builder: &mut TreeViewBuilder<'_, String>, lines: Vec<Line>, cx: &RowsCx) {
    for line in lines {
        let Line {
            id,
            folder,
            open,
            children,
        } = line;
        let mut node = if folder {
            NodeBuilder::dir(id.clone())
                .default_open(open)
                .activatable(true)
        } else {
            NodeBuilder::leaf(id.clone())
        };
        let label = Rc::new(LazyRow {
            id,
            row: OnceCell::new(),
        });
        let icon = label.clone();
        node = node
            .closer(|ui: &mut Ui, closer: CloserState| {
                let mark = if closer.is_open {
                    Mark::CaretDown
                } else {
                    Mark::CaretRight
                };
                let color = if closer.is_hovered {
                    theme::TEXT
                } else {
                    theme::FAINT
                };
                mark.paint(
                    ui.painter(),
                    Rect::from_center_size(ui.max_rect().center(), Vec2::splat(10.0)),
                    color,
                );
            })
            .label_ui(move |ui: &mut Ui| {
                row_label(ui, cx, &label.id, label.get(cx), open);
            })
            .icon(move |ui: &mut Ui| {
                row_icon(ui, icon.get(cx), open);
            });
        builder.node(node);
        if folder {
            build(builder, children, cx);
            builder.close_dir();
        }
    }
}

/// A row's look, asked of the source the first time the row is drawn.
struct LazyRow {
    id: String,
    row: OnceCell<TreeRow>,
}

impl LazyRow {
    fn get(&self, cx: &RowsCx) -> &TreeRow {
        self.row.get_or_init(|| (cx.row)(&self.id))
    }
}

fn row_icon(ui: &mut Ui, row: &TreeRow, open: bool) {
    let center = ui.max_rect().center();
    let badge = row.badge.as_ref().filter(|_| !row.folder);
    if let Some(&Badge { letter, color }) = badge {
        let rect = Rect::from_center_size(center, Vec2::splat(14.0));
        let color = if row.dim {
            color.gamma_multiply(0.5)
        } else {
            color
        };
        ui.painter().rect_filled(rect, 3.0, color);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            letter,
            FontId::monospace(10.0),
            ui::ink::CREAM,
        );
        return;
    }
    let icon = match (row.folder, open) {
        (true, true) => Some(KindIcon::from(Mark::FolderOpen)),
        (true, false) => Some(KindIcon::from(Mark::Folder)),
        (false, _) => row.icon,
    };
    if let Some(icon) = icon {
        let tone = if row.dim {
            IconTone::Dim
        } else {
            IconTone::Normal
        };
        let rect = Rect::from_center_size(center, Vec2::splat(ICON));
        icons::paint_file(ui.painter(), rect, row.preview, icon, tone);
    }
}

fn row_label(ui: &mut Ui, cx: &RowsCx, id: &str, row: &TreeRow, open: bool) {
    let rect = ui.max_rect();
    ui.add_space(ICON_GAP);
    if row.lit {
        paint::row_wash(ui.painter(), rect.expand2(Vec2::new(2.0, 1.0)));
    }
    let mut right = rect.right() - 6.0;
    if let Some(toggles) = row.toggles {
        right = row_toggles(ui, cx, id, row, toggles, right);
    }
    let marks = row_marks(row, !row.folder || !open);
    let mut reserved = rect.right() - right + MARK * (marks.len() as f32 + 1.0) + 6.0;
    let detail = (!row.detail.is_empty()).then(|| {
        ui.painter()
            .layout_no_wrap(row.detail.clone(), FontId::proportional(12.0), theme::FAINT)
    });
    if let Some(detail) = &detail {
        reserved += detail.size().x + 4.0;
    }
    let left = ui.cursor().min.x;
    let width = (right - reserved - left).max(24.0);
    let mut x = right;
    for (index, mark) in marks.iter().enumerate() {
        let at = egui::pos2(x - MARK / 2.0, rect.center().y);
        let painted = Rect::from_center_size(at, Vec2::splat(10.0));
        mark.mark.paint(ui.painter(), painted, mark.color);
        ui.interact(
            Rect::from_center_size(at, Vec2::splat(MARK)),
            ui.id().with(("tree mark", id, index)),
            egui::Sense::hover(),
        )
        .on_hover_text(&mark.tip);
        x -= MARK;
    }
    if let Some(detail) = detail {
        let at = egui::pos2(
            right - MARK * marks.len().max(1) as f32 - 4.0,
            rect.center().y,
        );
        ui.painter().galley(
            Align2::RIGHT_CENTER.anchor_size(at, detail.size()).min,
            detail,
            theme::FAINT,
        );
    }
    let mut renaming = cx.renaming.borrow_mut();
    match renaming.as_mut().filter(|renaming| renaming.id == id) {
        Some(renaming) => rename_box(ui, cx, renaming, width),
        None => name_label(ui, row, width),
    }
    cx.drawn.borrow_mut().push(DrawnRow {
        id: id.to_owned(),
        folder: row.folder,
        open,
        y: rect.y_range(),
        left,
    });
}

fn name_label(ui: &mut Ui, row: &TreeRow, width: f32) {
    let ink = if row.dim { theme::FAINT } else { theme::TEXT };
    let mut job = egui::text::LayoutJob::default();
    let plain = egui::TextFormat::simple(FontId::monospace(13.0), ink);
    match row
        .matched
        .clone()
        .filter(|range| row.name.get(range.clone()).is_some())
    {
        Some(range) => {
            job.append(&row.name[..range.start], 0.0, plain.clone());
            job.append(
                &row.name[range.clone()],
                0.0,
                egui::TextFormat {
                    background: ink::wash(theme::WARN, 0.27),
                    ..plain.clone()
                },
            );
            job.append(&row.name[range.end..], 0.0, plain);
        }
        None => job.append(&row.name, 0.0, plain),
    }
    if let Some((suffix, color)) = &row.suffix {
        let color = if row.dim { theme::FAINT } else { *color };
        job.append(
            suffix,
            0.0,
            egui::TextFormat::simple(FontId::monospace(13.0), color),
        );
    }
    let spoken = row.spoken.clone().unwrap_or_else(|| row.name.clone());
    let response = ui
        .allocate_ui(Vec2::new(width, ui.available_height()), |ui| {
            ui.add(egui::Label::new(job).selectable(false).truncate())
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    let tip: Vec<&str> = [row.problem.as_deref(), row.tip.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if !tip.is_empty() {
        response.on_hover_text(tip.join("\n\n"));
    }
}

fn rename_box(ui: &mut Ui, cx: &RowsCx, renaming: &mut Renaming, width: f32) {
    let edit_id = Id::new(("tree rename", &renaming.id));
    let response = ui.add(
        egui::TextEdit::singleline(&mut renaming.text)
            .id(edit_id)
            .font(FontId::monospace(13.0))
            .desired_width(width)
            .margin(egui::Margin::symmetric(2, 0)),
    );
    if let Some(tip) = cx.rename_tip {
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Tooltip, edit_id));
        chip::floating(
            &painter,
            response.rect.left_bottom() + Vec2::new(0.0, 4.0),
            Align2::LEFT_TOP,
            tip,
            theme::RAISED,
            theme::MUTED,
            12.0,
        );
    }
    if !renaming.focused {
        renaming.focused = true;
        response.request_focus();
        response.scroll_to_me(None);
        let mut edit = egui::TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(renaming.text.chars().count()),
        );
        edit.cursor.set_char_range(Some(all));
        edit.store(ui.ctx(), edit_id);
    } else if response.lost_focus() {
        let end = if ui.input(|i| EditorKey::Confirm.pressed(i)) {
            RenameEnd::Typed(renaming.id.clone(), renaming.text.trim().to_owned())
        } else {
            RenameEnd::Left
        };
        *cx.renamed.borrow_mut() = Some(end);
    }
}

/// The marks at a row's right end, rightmost first. A folder shows a problem or an unsaved file
/// inside it only while it is closed, the rows inside showing their own while it is open.
fn row_marks(row: &TreeRow, inside: bool) -> Vec<RowMark> {
    let mut marks = Vec::new();
    if inside && let Some(problem) = &row.problem {
        marks.push(RowMark {
            mark: Mark::Warning,
            color: theme::DANGER,
            tip: problem.clone(),
        });
    }
    if inside && row.unsaved {
        let tip = if row.folder {
            "holds unsaved changes"
        } else {
            "unsaved changes"
        };
        marks.push(RowMark {
            mark: Mark::Dot,
            color: theme::WARN,
            tip: tip.to_owned(),
        });
    }
    if inside && row.holds_selected {
        marks.push(RowMark {
            mark: Mark::Dot,
            color: theme::DANGER,
            tip: "holds what is selected".to_owned(),
        });
    }
    if let Some(reason) = &row.locked {
        marks.push(RowMark {
            mark: Mark::Lock,
            color: theme::FAINT,
            tip: format!("Stays where it is: {reason}, so it can't be renamed or moved"),
        });
    }
    marks
}

/// A mark at a row's right end, and what it says while the pointer rests on it.
struct RowMark {
    mark: Mark,
    color: Color32,
    tip: String,
}

/// Draws a row's eye and padlock at its right end and takes presses on them; answers where the
/// rest of the row ends.
fn row_toggles(
    ui: &mut Ui,
    cx: &RowsCx,
    id: &str,
    row: &TreeRow,
    toggles: RowToggles,
    right: f32,
) -> f32 {
    let center_y = ui.max_rect().center().y;
    let mut x = right;
    for (toggle, state) in [
        (Toggle::Lock, toggles.locked),
        (Toggle::Hide, toggles.hidden),
    ] {
        let rect =
            Rect::from_center_size(egui::pos2(x - TOGGLE / 2.0, center_y), Vec2::splat(TOGGLE));
        let sense = if toggles.disabled {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        };
        let response = ui.interact(rect, ui.id().with(("tree toggle", id, toggle as u8)), sense);
        let (on, off, verb, undo) = match toggle {
            Toggle::Hide => (Mark::EyeSlash, Mark::Eye, "Hide", "Show"),
            Toggle::Lock => (Mark::Lock, Mark::Unlock, "Lock", "Unlock"),
        };
        let (mark, color) = match state {
            ToggleState::Off if toggles.disabled => (off, ink::wash(theme::FAINT, 0.47)),
            ToggleState::Own | ToggleState::Inherited if toggles.disabled => (on, theme::FAINT),
            ToggleState::Off if response.hovered() => (off, theme::MUTED),
            ToggleState::Off => (off, ink::wash(theme::FAINT, 0.47)),
            ToggleState::Own => (on, theme::TEXT),
            ToggleState::Inherited => (on, theme::FAINT),
        };
        mark.paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(TOGGLE_GLYPH)),
            color,
        );
        if state == ToggleState::Inherited {
            let y = rect.center().y + TOGGLE_GLYPH / 2.0 + 1.5;
            for dot in 0..3 {
                ui.painter().circle_filled(
                    egui::pos2(rect.center().x - 4.0 + dot as f32 * 4.0, y),
                    0.8,
                    theme::FAINT,
                );
            }
        }
        let said = match state {
            ToggleState::Own => format!("{undo} {}", row.name),
            ToggleState::Inherited => format!("{verb} {}, now set by a folder above", row.name),
            ToggleState::Off => format!("{verb} {}", row.name),
        };
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, !toggles.disabled, &said)
        });
        let said = match (&row.tip, toggles.disabled) {
            (Some(why), true) => why.clone(),
            _ => said,
        };
        if response.on_hover_text(&said).clicked() {
            *cx.toggled.borrow_mut() = Some(Toggled {
                id: id.to_owned(),
                toggle,
                only: ui.input(|i| EditorKey::ShowOnly.held(i)),
            });
        }
        x -= TOGGLE;
    }
    x - 4.0
}

/// Where `needle`, lowercased, first shows in `name`, to highlight it.
pub fn matched(name: &str, needle: &str) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let start = name.to_lowercase().find(needle)?;
    Some(start..start + needle.len()).filter(|range| name.is_char_boundary(range.end))
}

/// A tree's search field, filling the row, with Show ▾ at its right. Answers what `search`
/// answers and whether the menu changed what the tree keeps in the user's settings.
pub(crate) fn filter_row<R>(
    ui: &mut Ui,
    search: impl FnOnce(&mut Ui) -> R,
    menu: impl FnOnce(&mut Ui) -> bool,
) -> (R, bool) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let changed = menu(ui);
            let searched = ui
                .with_layout(Layout::left_to_right(Align::Center), search)
                .inner;
            (searched, changed)
        })
        .inner
    })
    .inner
}

/// A narrowing of what a tree lists that lives as long as the editor runs, shown in Show ▾ as a
/// checkbox above the kinds.
pub(crate) struct Narrowing<'a> {
    pub label: &'a str,
    pub on: &'a mut bool,
}

/// Show ▾: which kinds of row a tree lists, whether folders with nothing shown stay, and the
/// tree's narrowings, each kind after its mark. Answers whether what the user's settings keep
/// changed.
pub(crate) fn show_menu(
    ui: &mut Ui,
    view: &mut crate::user::TreeView,
    kinds: &[String],
    mark_of: &dyn Fn(&str) -> KindMark,
    narrowings: &mut [Narrowing],
) -> bool {
    let shown = kinds
        .iter()
        .filter(|kind| !view.hidden_kinds.contains(*kind))
        .count();
    let mut changed = false;
    let button = controls::dropdown(ui, "Show");
    let menu = egui::Popup::from_toggle_button_response(&button)
        .width(SHOW_MENU_WIDTH)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    menu.show(|ui| {
        for narrowing in narrowings.iter_mut() {
            ui.checkbox(narrowing.on, narrowing.label);
        }
        if !narrowings.is_empty() {
            ui.separator();
        }
        let mut all = shown == kinds.len();
        if ui.checkbox(&mut all, "All").changed() {
            view.hidden_kinds = if all {
                BTreeSet::new()
            } else {
                kinds.iter().cloned().collect()
            };
            changed = true;
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for kind in kinds {
                    let mut on = !view.hidden_kinds.contains(kind);
                    if widgets::kind_checkbox(ui, mark_of(kind), kind, &mut on).changed() {
                        if on {
                            view.hidden_kinds.remove(kind);
                        } else {
                            view.hidden_kinds.insert(kind.clone());
                        }
                        changed = true;
                    }
                }
            });
        ui.separator();
        changed |= ui
            .checkbox(&mut view.hide_empty_folders, "Hide empty folders")
            .changed();
    });
    changed
}

/// Where a drag would land. A tree without an order of its own always drops into a folder: the
/// one under the pointer, or the one holding the row under it.
fn drop_of<S: TreeSource>(
    source: &S,
    drawn: &[DrawnRow],
    pointer: Pos2,
    target: &str,
    position: &DirPosition<String>,
) -> TreeDrop {
    if source.ordered() {
        let at = match position {
            DirPosition::First => DropAt::First,
            DirPosition::Last => DropAt::End,
            DirPosition::Before(id) => DropAt::Before(id.clone()),
            DirPosition::After(id) => DropAt::After(id.clone()),
        };
        let onto = drawn
            .iter()
            .find(|row| !row.folder && row.contains(pointer))
            .filter(|row| {
                let band = row.y.span() / 4.0;
                pointer.y > row.y.min + band && pointer.y < row.y.max - band
            })
            .map(|row| row.id.clone());
        return TreeDrop {
            folder: target.to_owned(),
            at,
            onto,
        };
    }
    let folder = match drawn.iter().find(|row| row.contains(pointer)) {
        Some(row) if row.folder => row.id.clone(),
        Some(row) => source.parent(&row.id),
        None => String::new(),
    };
    TreeDrop {
        folder,
        at: DropAt::End,
        onto: None,
    }
}

/// A drop into a folder boxes the folder and what shows inside it; one between rows is a line
/// with a dot at its depth.
fn mark_drop<S: TreeSource>(
    painter: &egui::Painter,
    source: &S,
    drawn: &[DrawnRow],
    tree: &Rect,
    drop: &TreeDrop,
) {
    let line = |id: &str, below: bool| {
        let row = drawn.iter().find(|row| row.id == id)?;
        let y = if below { row.y.max } else { row.y.min };
        Some((row.left, y))
    };
    let between = match &drop.at {
        DropAt::Before(id) => line(id, false),
        DropAt::After(id) => line(id, true),
        DropAt::First if !drop.folder.is_empty() => drawn
            .iter()
            .find(|row| source.parent(&row.id) == drop.folder)
            .map(|row| (row.left, row.y.min)),
        _ => None,
    };
    if let Some((left, y)) = between {
        let stroke = Stroke::new(2.0, theme::DROP);
        painter.line_segment(
            [egui::pos2(left, y), egui::pos2(tree.right() - 4.0, y)],
            stroke,
        );
        painter.circle_stroke(egui::pos2(left - 3.0, y), 3.0, stroke);
        return;
    }
    let inside = |id: &str| {
        drop.folder.is_empty() || id == drop.folder || id.starts_with(&format!("{}/", drop.folder))
    };
    let boxed = drawn
        .iter()
        .filter(|row| inside(&row.id))
        .map(|row| row.y)
        .reduce(|a, b| Rangef::new(a.min.min(b.min), a.max.max(b.max)));
    let y = boxed.unwrap_or(tree.y_range());
    painter.rect_stroke(
        Rect::from_x_y_ranges(tree.x_range(), y).shrink(1.0),
        3.0,
        Stroke::new(1.5, theme::DROP),
        StrokeKind::Inside,
    );
}

fn drag_tip(ui: &Ui, pointer: Pos2, text: &str, ink: Color32) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("tree drag tip"),
    ));
    chip::floating(
        &painter,
        pointer + Vec2::new(14.0, 16.0),
        Align2::LEFT_TOP,
        text,
        theme::RAISED,
        ink,
        12.5,
    );
}

/// Opens a closed folder the pointer has rested on for a moment while dragging.
fn open_on_hover(
    view: &mut TreeViewState<String>,
    hover: &mut Option<(String, f64)>,
    drawn: &[DrawnRow],
    pointer: Pos2,
    now: f64,
) {
    let Some(row) = drawn
        .iter()
        .find(|row| row.contains(pointer) && row.folder && !row.open)
    else {
        *hover = None;
        return;
    };
    match hover {
        Some((id, since)) if *id == row.id => {
            if now - *since >= HOVER_OPENS {
                view.set_openness(row.id.clone(), true);
                *hover = None;
            }
        }
        _ => *hover = Some((row.id.clone(), now)),
    }
}
